//! Local deterministic OIDC provider for Rust and pinned-Go interoperability tests only.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use http_body_util::{BodyExt, Full};
use hyper::{
    body::{Bytes, Incoming},
    Request, Response,
};
use rsa::{
    pkcs1::DecodeRsaPrivateKey,
    signature::{SignatureEncoding, Signer},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[derive(Clone)]
struct Code {
    nonce: String,
    challenge: String,
    subject: String,
    username: String,
}
struct State {
    next: u64,
    codes: BTreeMap<String, Code>,
    refresh: BTreeMap<String, Code>,
    username: String,
    subject: String,
    refreshes: usize,
    outage: bool,
    invalid_grant: bool,
    access_lifetime: u64,
}
pub struct TestOidcProvider {
    url: String,
    callback: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl TestOidcProvider {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let reserve = TcpListener::bind("127.0.0.1:0").unwrap();
        let callback = reserve.local_addr().unwrap();
        drop(reserve);
        let state = Arc::new(Mutex::new(State {
            next: 0,
            codes: BTreeMap::new(),
            refresh: BTreeMap::new(),
            username: "ssoalice".into(),
            subject: "alice-subject".into(),
            refreshes: 0,
            outage: false,
            invalid_grant: false,
            access_lifetime: 300,
        }));
        let shared = state.clone();
        let root = url.clone();
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let key = rsa::RsaPrivateKey::from_pkcs1_pem(include_str!(
            "../../foks-oidc/tests/fixtures/TEST_ONLY_RSA_KEY.pem"
        ))
        .unwrap();
        let key = Arc::new(rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(key));
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move{
            let listener=tokio::net::TcpListener::from_std(listener).unwrap();loop{tokio::select!{_=&mut stopped=>break,stream=listener.accept()=>{let (stream,_)=stream.unwrap();let state=shared.clone();let root=root.clone();let key=key.clone();tokio::spawn(async move{let handler=hyper::service::service_fn(move|request|reply(request,root.clone(),callback,state.clone(),key.clone()));let _=hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream),handler).await;});}}}
        });
        });
        Self {
            url,
            callback,
            state,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
    pub fn config(&self, secret_file: std::path::PathBuf) -> foks_server::sso::OidcOperatorConfig {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::write(&secret_file, b"test-secret").unwrap();
        std::fs::set_permissions(&secret_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        foks_server::sso::OidcOperatorConfig {
            config_id: [50; 17],
            issuer: self.url.clone(),
            discovery_uri: format!("{}/discovery", self.url),
            client_id: "fennec".into(),
            client_secret_file: secret_file,
            redirect_uri: format!("http://{}/oauth2/callback", self.callback),
            listen: self.callback,
            client_secret_post: true,
        }
    }
    pub fn complete(&self, start: &str) {
        let response = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap()
            .get(start)
            .send()
            .unwrap();
        assert!(
            response.status().is_success(),
            "browser callback failed: {}",
            response.status()
        );
    }
    pub fn identity(&self, username: &str, subject: &str) {
        let mut s = self.state.lock().unwrap();
        s.username = username.into();
        s.subject = subject.into();
    }
    pub fn set_outage(&self, value: bool) {
        self.state.lock().unwrap().outage = value;
    }
    pub fn set_invalid_grant(&self, value: bool) {
        self.state.lock().unwrap().invalid_grant = value;
    }
    pub fn refreshes(&self) -> usize {
        self.state.lock().unwrap().refreshes
    }
    pub fn access_lifetime(&self, seconds: u64) {
        self.state.lock().unwrap().access_lifetime = seconds;
    }
}
impl Drop for TestOidcProvider {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn response(code: u16, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(code)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}
async fn reply(
    request: Request<Incoming>,
    root: String,
    callback: SocketAddr,
    shared: Arc<Mutex<State>>,
    key: Arc<rsa::pkcs1v15::SigningKey<sha2::Sha256>>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    let body = if path == "/token" {
        request.into_body().collect().await.unwrap().to_bytes()
    } else {
        Bytes::new()
    };
    let mut state = shared.lock().unwrap();
    let redirect = format!("http://{callback}/oauth2/callback");
    if state.outage {
        return Ok(response(503, "provider unavailable".into()));
    }
    let result=match path.as_str(){
        "/discovery"=>response(200,json!({"issuer":root,"authorization_endpoint":format!("{root}/authorize"),"token_endpoint":format!("{root}/token"),"jwks_uri":format!("{root}/jwks"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"scopes_supported":["openid","email","profile","offline_access"],"token_endpoint_auth_methods_supported":["client_secret_post"]}).to_string()),
        "/jwks"=>response(200,include_str!("../../foks-oidc/tests/fixtures/jwks.json").into()),
        "/authorize"=>{
            let q=url::form_urlencoded::parse(query.as_bytes()).collect::<BTreeMap<_,_>>();
            if q.get("redirect_uri").map(|s|s.as_ref())!=Some(&redirect)||q.get("client_id").map(|s|s.as_ref())!=Some("fennec")||q.get("code_challenge_method").map(|s|s.as_ref())!=Some("S256"){return Ok(response(400,"invalid authorization request".into()));}
            state.next+=1;let code=format!("test-code-{}",state.next);
            let entry=Code{nonce:q.get("nonce").unwrap().to_string(),challenge:q.get("code_challenge").unwrap().to_string(),username:state.username.clone(),subject:state.subject.clone()};state.codes.insert(code.clone(),entry);
            let mut uri=url::Url::parse(&redirect).unwrap();uri.query_pairs_mut().append_pair("state",q.get("state").unwrap()).append_pair("code",&code);
            Response::builder().status(303).header("location",uri.as_str()).body(Full::new(Bytes::new())).unwrap()
        }
        "/token"=>{
            use sha2::Digest as _;
            let q=url::form_urlencoded::parse(&body).collect::<BTreeMap<_,_>>();
            if q.get("client_id").map(|s|s.as_ref())!=Some("fennec")||q.get("client_secret").map(|s|s.as_ref())!=Some("test-secret"){return Ok(response(400,json!({"error":"invalid_client"}).to_string()));}
            let refreshing=q.get("grant_type").map(|s|s.as_ref())==Some("refresh_token");
            if state.invalid_grant{return Ok(response(400,json!({"error":"invalid_grant"}).to_string()));}
            let entry=if refreshing {state.refreshes+=1;state.refresh.remove(q.get("refresh_token").unwrap().as_ref())}else{state.codes.remove(q.get("code").unwrap().as_ref())};
            let Some(entry)=entry else{return Ok(response(400,json!({"error":"invalid_grant"}).to_string()));};
            if !refreshing {
                let verifier=q.get("code_verifier").unwrap();
                // Deliberately accept pinned Go's 27-char verifier here to measure FOKS wire
                // interoperability. Strict-provider rejection is covered by the Go IdP harness.
                if !(27..=128).contains(&verifier.len())||URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))!=entry.challenge||q.get("redirect_uri").map(|s|s.as_ref())!=Some(&redirect){return Ok(response(400,json!({"error":"invalid_grant"}).to_string()));}
            }
            let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            let mut claims=json!({"iss":root,"sub":entry.subject,"aud":"fennec","iat":now.saturating_sub(30),"exp":now+3600,"preferred_username":entry.username,"email":format!("{}@example.test",entry.username)});
            if !refreshing {claims["nonce"]=json!(entry.nonce);}
            let jwks:serde_json::Value=serde_json::from_str(include_str!("../../foks-oidc/tests/fixtures/jwks.json")).unwrap();
            let header=json!({"alg":"RS256","typ":"JWT","kid":jwks["keys"][0]["kid"]});let input=format!("{}.{}",URL_SAFE_NO_PAD.encode(header.to_string()),URL_SAFE_NO_PAD.encode(claims.to_string()));let signed=format!("{}.{}",input,URL_SAFE_NO_PAD.encode(key.sign(input.as_bytes()).to_bytes()));
            state.next+=1;let refresh=format!("refresh-{}",state.next);state.refresh.insert(refresh.clone(),entry);
            response(200,json!({"access_token":"opaque-access","id_token":signed,"refresh_token":refresh,"expires_in":state.access_lifetime,"token_type":"Bearer"}).to_string())
        }
        _=>response(404,"unknown endpoint".into()),
    };
    Ok(result)
}
