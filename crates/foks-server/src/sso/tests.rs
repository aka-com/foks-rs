use super::*;
use crate::keys::{DirectoryKeyProvider, KeyPurpose};
use foks_proto::OAuth2Secret;
use http_body_util::{BodyExt, Full};
use hyper::{
    body::{Bytes, Incoming},
    Request, Response,
};
use serde_json::json;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
const NOW: u64 = 1_700_000_100_000;
struct Clock(AtomicU64);
impl foks_server_db::Clock for Clock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        Ok(self.0.load(Ordering::SeqCst) * 1000)
    }
}
struct Idp {
    url: String,
    calls: Arc<AtomicUsize>,
    stale_keys: Arc<AtomicUsize>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Idp {
    fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let root = url.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let stale_keys = Arc::new(AtomicUsize::new(0));
        let stale = stale_keys.clone();
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener=tokio::net::TcpListener::from_std(listener).unwrap();
                loop {tokio::select! {_=&mut stopped=>break,stream=listener.accept()=>{
                    let (stream,_)=stream.unwrap();let url=root.clone();let count=count.clone();let stale=stale.clone();
                    tokio::spawn(async move {let handler=hyper::service::service_fn(move|request|idp_response(request,url.clone(),count.clone(),stale.clone()));let _=hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream),handler).await;});
                }}}
            });
        });
        Self {
            url,
            calls,
            stale_keys,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}
impl Drop for Idp {
    fn drop(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.thread.take().unwrap().join().unwrap();
    }
}
async fn idp_response(
    request: Request<Incoming>,
    url: String,
    count: Arc<AtomicUsize>,
    stale: Arc<AtomicUsize>,
) -> std::result::Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let mut status = 200;
    let body=match request.uri().path() {
        "/discovery"=>json!({"issuer":"https://idp.example","authorization_endpoint":format!("{url}/authorize"),"token_endpoint":format!("{url}/token"),"jwks_uri":format!("{url}/jwks"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"scopes_supported":["openid","profile","email","offline_access"]}).to_string(),
        "/jwks"=>{
            let mut keys:serde_json::Value=serde_json::from_str(include_str!("../../../foks-oidc/tests/fixtures/jwks.json")).unwrap();
            if stale.fetch_update(Ordering::SeqCst,Ordering::SeqCst,|n|n.checked_sub(1)).is_ok() { keys["keys"][0]["kid"]=json!("retired-key"); }
            keys.to_string()
        },
        "/token"=>{
            count.fetch_add(1,Ordering::SeqCst);
            let body=request.into_body().collect().await.unwrap().to_bytes();
            let fields=url::form_urlencoded::parse(&body).collect::<std::collections::BTreeMap<_,_>>();
            assert_eq!(fields.get("code_verifier").unwrap(),&"a".repeat(43));
            match fields.get("code").map(|s|s.as_ref()).unwrap() {
                "malformed"=>"[]".to_string(),
                "oversized"=>"x".repeat(foks_oidc::MAX_PROVIDER_BYTES+1),
                "outage"=>{status=503;"temporarily unavailable".into()},
                "invalid"=>{status=400;json!({"error":"invalid_grant"}).to_string()},
                _=>json!({"access_token":"opaque-access-SECRET","id_token":include_str!("../../../foks-oidc/tests/fixtures/valid.jwt"),"refresh_token":"refresh-SECRET","token_type":"Bearer","expires_in":300}).to_string(),
            }
        }
        _=>{status=404;"unknown".into()}
    };
    Ok(Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap())
}
struct Fixture {
    _dir: tempfile::TempDir,
    idp: Idp,
    writer: crate::Writer,
    keys: Arc<DirectoryKeyProvider>,
    clock: Arc<Clock>,
    config: OidcOperatorConfig,
}
impl Fixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let idp = Idp::new();
        let secret = dir.path().join("client-secret");
        std::fs::write(&secret, b"test-client-secret").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let keys = Arc::new(DirectoryKeyProvider::open(dir.path().join("keys"), [7; 32]).unwrap());
        keys.load_or_create(KeyPurpose::Recovery).unwrap();
        let writer =
            crate::Writer::start(dir.path().join("server.sqlite"), Default::default(), 32).unwrap();
        let config = OidcOperatorConfig {
            config_id: [50; 17],
            issuer: "https://idp.example".into(),
            discovery_uri: format!("{}/discovery", idp.url),
            client_id: "fennec".into(),
            client_secret_file: secret,
            redirect_uri: "http://127.0.0.1:1234/oauth2/callback".into(),
            listen: "127.0.0.1:0".parse().unwrap(),
            client_secret_post: true,
        };
        Self {
            _dir: dir,
            idp,
            writer,
            keys,
            clock: Arc::new(Clock(AtomicU64::new(NOW))),
            config,
        }
    }
    fn open(&self) -> Arc<SsoService> {
        SsoService::open(
            self.config.clone(),
            NetworkPolicy::loopback_test(),
            [1; 33],
            self.writer.handle(),
            self.keys.clone(),
            self.clock.clone(),
            Arc::new(crate::OsEntropy),
        )
        .unwrap()
    }
}
fn init(id: u8) -> InitOAuth2SessionArgument {
    let mut session = [id; 17];
    session[0] = 51;
    InitOAuth2SessionArgument {
        id: OAuth2SessionId(session),
        pkce_verifier: OAuth2Secret::new("a".repeat(43)),
        nonce: OAuth2Secret::new("fixture-nonce".into()),
        uid: None,
    }
}
fn state(arg: &InitOAuth2SessionArgument) -> String {
    URL_SAFE_NO_PAD.encode(arg.id.0)
}
#[test]
fn callbacks_are_durable_nonreplayable_and_database_contains_only_envelopes() {
    let fixture = Fixture::new();
    let service = fixture.open();
    let arg = init(1);
    let start = service.init(&arg, b"actual-peer").unwrap();
    assert!(start.starts_with("http://127.0.0.1:1234/oauth2/start?state="));
    let url = url::Url::parse(&service.browser_start(&state(&arg)).unwrap()).unwrap();
    let query = url
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(query.get("nonce").unwrap(), "fixture-nonce");
    assert_eq!(query.get("code_challenge_method").unwrap(), "S256");
    assert_eq!(
        query.get("redirect_uri").unwrap(),
        &fixture.config.redirect_uri
    );
    service
        .callback(&state(&arg), Some("valid"), false)
        .unwrap();
    assert!(service
        .callback(&state(&arg), Some("valid"), false)
        .is_err());
    assert_eq!(fixture.idp.calls.load(Ordering::SeqCst), 1);
    let row = service.row(&arg.id).unwrap();
    assert_eq!(row.state, SsoSessionState::Ready);
    let material = service.material(&row).unwrap();
    assert_eq!(material.subject, "stable-subject");
    assert_eq!(material.access_token, "opaque-access-SECRET");
    assert!(!row.ciphertext.windows(6).any(|v| v == b"SECRET"));
    assert_eq!(
        service.session_state(&arg.id).unwrap(),
        SsoSessionState::Ready
    );
    drop(service);
    let reopened = fixture.open();
    assert_eq!(
        reopened
            .material(&reopened.row(&arg.id).unwrap())
            .unwrap()
            .refresh_token
            .as_deref(),
        Some("refresh-SECRET")
    );
    let mut altered = row.clone();
    altered.uid = Some([3; 33]);
    assert!(envelope::open(fixture.keys.as_ref(), &altered).is_err());
    altered = row.clone();
    altered.expires_at_ms += 1;
    assert!(envelope::open(fixture.keys.as_ref(), &altered).is_err());
    altered = row.clone();
    altered.config_hash = [9; 32];
    assert!(envelope::open(fixture.keys.as_ref(), &altered).is_err());
}
#[test]
fn denied_ambiguous_and_expired_flows_cannot_exchange_again() {
    let fixture = Fixture::new();
    let service = fixture.open();
    for (i, code, expected) in [
        (1, "malformed", SsoSessionState::ExchangeUnknown),
        (2, "oversized", SsoSessionState::ExchangeUnknown),
        (3, "outage", SsoSessionState::ExchangeUnknown),
        (4, "invalid", SsoSessionState::Rejected),
    ] {
        let arg = init(i);
        service.init(&arg, &[i]).unwrap();
        assert!(service.callback(&state(&arg), Some(code), false).is_err());
        assert_eq!(service.session_state(&arg.id).unwrap(), expected);
        assert!(service.callback(&state(&arg), Some(code), false).is_err());
    }
    let arg = init(5);
    service.init(&arg, b"peer-denied").unwrap();
    service.callback(&state(&arg), None, true).unwrap();
    assert_eq!(
        service.session_state(&arg.id).unwrap(),
        SsoSessionState::Denied
    );
    assert!(service
        .callback(&state(&arg), Some("valid"), false)
        .is_err());
    let arg = init(6);
    service.init(&arg, b"peer-expired").unwrap();
    fixture.clock.0.store(NOW + 600_000, Ordering::SeqCst);
    assert!(service
        .callback(&state(&arg), Some("valid"), false)
        .is_err());
    assert_eq!(fixture.idp.calls.load(Ordering::SeqCst), 4);
}
#[test]
fn interrupted_exchange_is_fenced_on_restart_and_policy_rotation_fences_old_owner() {
    let mut fixture = Fixture::new();
    let service = fixture.open();
    let arg = init(1);
    service.init(&arg, b"peer").unwrap();
    let row = service.row(&arg.id).unwrap();
    service
        .transition(
            &row,
            SsoSessionState::Exchanging,
            &service.material(&row).unwrap(),
        )
        .unwrap();
    drop(service);
    let service = fixture.open();
    assert_eq!(
        service.session_state(&arg.id).unwrap(),
        SsoSessionState::ExchangeUnknown
    );
    assert!(service
        .callback(&state(&arg), Some("valid"), false)
        .is_err());
    let arg = init(2);
    service.init(&arg, b"peer").unwrap();
    fixture.config.config_id[1] = 42;
    let replacement = fixture.open();
    assert!(service.browser_start(&state(&arg)).is_err());
    assert!(replacement.browser_start(&state(&arg)).is_err());
    assert_eq!(fixture.idp.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn http_adapter_does_not_reflect_tokens_or_accept_ambiguous_callback_parameters() {
    use std::io::{Read as _, Write as _};
    let fixture = Fixture::new();
    let service = fixture.open();
    let arg = init(1);
    service.init(&arg, b"peer").unwrap();
    let http = OidcHttpServer::start(service.clone()).unwrap();
    let request = |path: &str| {
        let mut stream = std::net::TcpStream::connect(http.address()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).unwrap();
        out
    };
    let bad = request(&format!(
        "/oauth2/callback?state={}&state={}&code=SECRET",
        state(&arg),
        state(&arg)
    ));
    assert!(bad.starts_with("HTTP/1.1 400"));
    assert!(!bad.contains("SECRET"));
    let bad = request(&format!(
        "/oauth2/callback?state={}&code=SECRET&iss=https%3A%2F%2Fevil.example",
        state(&arg)
    ));
    assert!(bad.starts_with("HTTP/1.1 400"));
    let good = request(&format!("/oauth2/start?state={}", state(&arg)));
    assert!(good.starts_with("HTTP/1.1 303"));
    assert!(good.contains("cache-control: no-store"));
    assert!(!good.contains("test-client-secret"));
    let good = request(&format!(
        "/oauth2/callback?state={}&code=valid",
        state(&arg)
    ));
    assert!(good.starts_with("HTTP/1.1 200"));
    assert!(!good.contains("opaque-access"));
    assert!(request("/admin").starts_with("HTTP/1.1 400"));
    http.shutdown().unwrap();
}
#[test]
fn envelope_survives_operator_root_rotation_and_rejects_missing_or_wrong_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys");
    let keys = DirectoryKeyProvider::open(&path, [7; 32]).unwrap();
    keys.load_or_create(KeyPurpose::Recovery).unwrap();
    let mut row = SsoSession {
        host: [1; 33],
        session_hash: [2; 32],
        config_hash: [3; 32],
        admission_hash: [4; 32],
        uid: None,
        state: SsoSessionState::Ready,
        revision: 2,
        expires_at_ms: 100,
        ciphertext: Vec::new(),
    };
    row.ciphertext = envelope::seal(&keys, &crate::OsEntropy, &row, b"token-SECRET").unwrap();
    drop(keys);
    assert!(DirectoryKeyProvider::open(&path, [8; 32]).is_err());
    DirectoryKeyProvider::rotate_operator_root(&path, [7; 32], [8; 32]).unwrap();
    let keys = DirectoryKeyProvider::open(&path, [8; 32]).unwrap();
    assert_eq!(
        envelope::open(&keys, &row).unwrap().as_slice(),
        b"token-SECRET"
    );
    let empty = crate::keys::MemoryKeyProvider::default();
    assert!(envelope::open(&empty, &row).is_err());
    assert!(envelope::seal(&empty, &crate::OsEntropy, &row, b"token").is_err());
}

#[test]
fn simultaneous_callbacks_have_one_exchange_owner() {
    let fixture = Fixture::new();
    let service = fixture.open();
    let arg = init(1);
    service.init(&arg, b"peer").unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let threads = (0..4)
        .map(|_| {
            let barrier = barrier.clone();
            let service = service.clone();
            let state = state(&arg);
            std::thread::spawn(move || {
                barrier.wait();
                service.callback(&state, Some("valid"), false).is_ok()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        threads
            .into_iter()
            .filter_map(|t| t.join().ok())
            .filter(|v| *v)
            .count(),
        1
    );
    assert_eq!(fixture.idp.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn operator_settings_require_public_https_and_do_not_publish_secrets() {
    let fixture = Fixture::new();
    assert!(fixture.config.validate(NetworkPolicy::default()).is_err());
    let mut config = fixture.config.clone();
    config.discovery_uri = "https://idp.example/discovery".into();
    config.redirect_uri = "https://foks.example/oauth2/callback".into();
    config.validate(NetworkPolicy::default()).unwrap();
    assert!(config
        .public()
        .oauth2
        .unwrap()
        .client_secret
        .expose()
        .is_empty());
    config.listen = "0.0.0.0:9000".parse().unwrap();
    assert!(config.validate(NetworkPolicy::default()).is_err());
    let service = fixture.open();
    let arg = init(1);
    service.init(&arg, b"peer").unwrap();
    std::fs::write(&fixture.config.client_secret_file, b"rotated-client-secret").unwrap();
    let replacement = fixture.open();
    assert_ne!(service.config_hash, replacement.config_hash);
    assert!(service.browser_start(&state(&arg)).is_err());
}

#[test]
fn signing_key_rotation_refreshes_jwks_without_reexchanging_code() {
    let fixture = Fixture::new();
    fixture.idp.stale_keys.store(2, Ordering::SeqCst);
    let service = fixture.open();
    let arg = init(1);
    service.init(&arg, b"peer").unwrap();
    service
        .callback(&state(&arg), Some("valid"), false)
        .unwrap();
    assert_eq!(
        service.session_state(&arg.id).unwrap(),
        SsoSessionState::Ready
    );
    assert_eq!(fixture.idp.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.idp.stale_keys.load(Ordering::SeqCst), 0);
}
