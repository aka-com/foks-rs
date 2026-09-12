//! Real native handoff followed by HTTPS reverse-proxy browser requests.
use foks_client::{AdminDestination, DeviceCredential, FederationCredential, PinnedHost};
use foks_server_testkit::{InProcessServer, TestAccountSpec, TestClient, TestEnvironment};
use reqwest::{blocking::Client, StatusCode};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
struct Proxy {
    certificate: Vec<u8>,
    address: SocketAddr,
    origin: String,
    client: Client,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Proxy {
    fn new(backend: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("https://admin.test:{}", address.port());
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["admin.test".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let tls = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![cert.der().clone()],
                    rustls_pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
                )
                .unwrap(),
        );
        let client = Client::builder()
            .https_only(true)
            .no_proxy()
            .add_root_certificate(reqwest::Certificate::from_der(cert.der()).unwrap())
            .resolve("admin.test", address)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut children = Vec::new();
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        let tls = tls.clone();
                        children.push(std::thread::spawn(move || {
                            let _ = forward(socket, backend, tls);
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
            for child in children {
                child.join().unwrap();
            }
        });
        Self {
            certificate: cert.der().to_vec(),
            address,
            origin,
            client,
            stop,
            thread: Some(thread),
        }
    }
    fn get(&self, path: &str, cookie: Option<&str>) -> reqwest::blocking::Response {
        let mut r = self.client.get(format!("{}{path}", self.origin));
        if let Some(c) = cookie {
            r = r.header("cookie", c);
        }
        r.send().unwrap()
    }
    fn post(&self, path: &str, cookie: &str, body: &str) -> reqwest::blocking::Response {
        self.client
            .post(format!("{}{path}", self.origin))
            .header("origin", &self.origin)
            .header("cookie", cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.to_owned())
            .send()
            .unwrap()
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn forward(
    socket: TcpStream,
    backend: SocketAddr,
    config: Arc<rustls::ServerConfig>,
) -> std::io::Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    socket.set_write_timeout(Some(Duration::from_secs(10)))?;
    let connection = rustls::ServerConnection::new(config).map_err(std::io::Error::other)?;
    let mut tls = rustls::StreamOwned::new(connection, socket);
    let mut request = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let n = tls.read(&mut buffer)?;
        if n == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..n]);
        if request.len() > 32 * 1024 {
            return Ok(());
        }
        if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..end]);
            let length = headers
                .lines()
                .filter_map(|v| v.split_once(':'))
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.trim().parse::<usize>().unwrap_or(usize::MAX))
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                break;
            }
        }
    }
    let mut upstream = TcpStream::connect_timeout(&backend, Duration::from_secs(3))?;
    upstream.set_read_timeout(Some(Duration::from_secs(12)))?;
    upstream.write_all(&request)?;
    let mut response = Vec::new();
    upstream.take(256 * 1024).read_to_end(&mut response)?;
    tls.write_all(&response)?;
    tls.flush()?;
    Ok(())
}
struct Fixture {
    environment: TestEnvironment,
    server: Option<InProcessServer>,
    client: TestClient,
    host: PinnedHost,
    credential: DeviceCredential,
    proxy: Proxy,
    config: foks_server::web_admin::WebAdminConfig,
}
impl Fixture {
    fn new(operator: bool) -> Self {
        let environment = TestEnvironment::new().unwrap();
        let server = environment.start_server().unwrap();
        let client = TestClient::new(&environment, "admin-https").unwrap();
        let host = client.probe_and_pin().unwrap().pinned;
        let credential = client
            .create_account(&host, &TestAccountSpec::new("adminowner", 0x41))
            .unwrap()
            .credential;
        server.shutdown().unwrap();
        if operator {
            let guard =
                foks_server::DatabaseWriterGuard::acquire(environment.database_path()).unwrap();
            guard
                .open_database(Default::default())
                .unwrap()
                .admin_set_grant(
                    host.host_id().as_bytes().try_into().unwrap(),
                    credential.uid.as_bytes().try_into().unwrap(),
                    true,
                    "test operator",
                    environment.advance_clock(0),
                )
                .unwrap();
        }
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let backend = reservation.local_addr().unwrap();
        let proxy = Proxy::new(backend);
        drop(reservation);
        let config = foks_server::web_admin::WebAdminConfig {
            origin: proxy.origin.clone(),
            listen: backend,
        };
        let server = environment
            .start_web_admin_server(config.clone(), None)
            .unwrap();
        Self {
            environment,
            server: Some(server),
            client,
            host,
            credential,
            proxy,
            config,
        }
    }
    fn ticket(&self) -> String {
        self.client
            .foks()
            .new_web_admin_handoff(
                &self.host,
                FederationCredential::Software(&self.credential),
                &AdminDestination::new(&self.proxy.origin).unwrap(),
            )
            .unwrap()
            .expose()
            .to_owned()
    }
    fn login(&self) -> (String, String) {
        let ticket = self.ticket();
        let url = url::Url::parse(&ticket).unwrap();
        let staged = self.proxy.get(&format!("/?{}", url.query().unwrap()), None);
        assert_eq!(staged.status(), StatusCode::SEE_OTHER);
        assert_eq!(staged.headers()["location"], "/login/confirm");
        let pending = cookie(&staged, "__Host-foks_admin_pending");
        let confirmation = self.proxy.get("/login/confirm", Some(&pending));
        assert_eq!(confirmation.status(), StatusCode::OK);
        let body = confirmation.text().unwrap();
        assert!(body.contains("adminowner"));
        assert!(!body.contains("?session="));
        let csrf = hidden(&body, "csrf");
        let redeemed = self
            .proxy
            .post("/login/confirm", &pending, &format!("csrf={csrf}"));
        assert_eq!(redeemed.status(), StatusCode::SEE_OTHER);
        let session = cookie(&redeemed, "__Host-foks_admin");
        let page = self.proxy.get("/admin", Some(&session));
        assert_eq!(page.status(), StatusCode::OK);
        let csrf = hidden(&page.text().unwrap(), "csrf");
        assert!(matches!(
            self.client.foks().check_web_admin_session(
                &self.host,
                FederationCredential::Software(&self.credential),
                &ticket
            ),
            Err(foks_client::WebAdminError::Expired)
        ));
        (session, csrf)
    }
}
fn cookie(r: &reqwest::blocking::Response, name: &str) -> String {
    r.headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap())
        .find(|v| v.starts_with(&format!("{name}=")))
        .unwrap()
        .to_owned()
}
fn hidden(html: &str, name: &str) -> String {
    html.split_once(&format!("name=\"{name}\" value=\""))
        .unwrap()
        .1
        .split('"')
        .next()
        .unwrap()
        .to_owned()
}
#[test]
pub(crate) fn https_native_handoff_invites_cas_and_server_logout() {
    let f = Fixture::new(true);
    let (session, csrf) = f.login();
    let page = f.proxy.get("/admin/invites", Some(&session));
    assert_eq!(page.status(), StatusCode::OK);
    let body = page.text().unwrap();
    let nonce = hidden(&body, "nonce");
    let form = format!("csrf={csrf}&nonce={nonce}");
    let issued = f.proxy.post("/admin/invites", &session, &form);
    assert_eq!(issued.status(), StatusCode::OK);
    let issued = issued.text().unwrap();
    assert!(issued.contains("Copy this code now"));
    let replay = f.proxy.post("/admin/invites", &session, &form);
    assert_eq!(replay.status(), StatusCode::OK);
    assert!(replay
        .text()
        .unwrap()
        .contains("secret code is no longer available"));
    let policy = format!("csrf={csrf}&revision=1&regime=required&confirm=yes");
    assert_eq!(
        f.proxy
            .post("/admin/signup-policy", &session, &policy)
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        f.proxy
            .post("/admin/signup-policy", &session, &policy)
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.environment
            .read_database()
            .unwrap()
            .invite_policy()
            .unwrap()
            .regime,
        foks_server_db::InviteRegime::Required
    );
    let invites = f.environment.read_database().unwrap().invites().unwrap();
    let id = invites[0]
        .invite_id
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    assert_eq!(
        f.proxy
            .post(
                &format!("/admin/invites/{id}/disable"),
                &session,
                &format!("csrf={csrf}&revision=1")
            )
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        f.proxy.get("/admin/audit", Some(&session)).status(),
        StatusCode::OK
    );
    assert_eq!(
        f.proxy
            .post("/logout", &session, &format!("csrf={csrf}"))
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        f.proxy.get("/admin", Some(&session)).status(),
        StatusCode::UNAUTHORIZED
    );
}
#[test]
fn hostile_browser_inputs_and_team_independent_self_permissions() {
    let f = Fixture::new(false);
    let (session, csrf) = f.login();
    assert_eq!(
        f.proxy.get("/admin/invites", Some(&session)).status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.proxy.get("/admin/sessions", Some(&session)).status(),
        StatusCode::OK
    );
    let wrong_origin = f
        .proxy
        .client
        .post(format!("{}/logout", f.proxy.origin))
        .header("origin", "https://evil.test")
        .header("cookie", &session)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("csrf={csrf}"))
        .send()
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);
    let spoof = f
        .proxy
        .client
        .get(format!("{}/admin", f.proxy.origin))
        .header("host", "evil.test")
        .header("cookie", &session)
        .send()
        .unwrap();
    assert_eq!(spoof.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        f.proxy
            .get("/admin", Some(&format!("{session}; {session}")))
            .status(),
        StatusCode::BAD_REQUEST
    );
    for path in [
        "/admin/sessions/revoke",
        "/admin/invites/disable",
        "/admin/sessions//revoke",
    ] {
        assert_eq!(
            f.proxy
                .post(path, &session, &format!("csrf={csrf}&uid=00"))
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let extra = f
        .proxy
        .post("/logout", &session, &format!("csrf={csrf}&csrf={csrf}"));
    assert_eq!(extra.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        f.proxy.get("/admin", Some(&session)).status(),
        StatusCode::OK
    );
}
#[test]
fn concurrent_confirmation_redeems_once_and_restart_invalidates_cookie() {
    let mut f = Fixture::new(true);
    let ticket = f.ticket();
    let url = url::Url::parse(&ticket).unwrap();
    let staged = f.proxy.get(&format!("/?{}", url.query().unwrap()), None);
    let pending = cookie(&staged, "__Host-foks_admin_pending");
    let csrf = hidden(
        &f.proxy
            .get("/login/confirm", Some(&pending))
            .text()
            .unwrap(),
        "csrf",
    );
    let results = std::thread::scope(|scope| {
        let tasks = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    f.proxy
                        .post("/login/confirm", &pending, &format!("csrf={csrf}"))
                })
            })
            .collect::<Vec<_>>();
        tasks
            .into_iter()
            .map(|v| v.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results
            .iter()
            .filter(|r| r.status() == StatusCode::SEE_OTHER)
            .count(),
        1
    );
    let session = cookie(
        results
            .iter()
            .find(|r| r.status() == StatusCode::SEE_OTHER)
            .unwrap(),
        "__Host-foks_admin",
    );
    f.server.take().unwrap().shutdown().unwrap();
    f.server = Some(
        f.environment
            .start_web_admin_server(f.config.clone(), None)
            .unwrap(),
    );
    assert_eq!(
        f.proxy.get("/admin", Some(&session)).status(),
        StatusCode::UNAUTHORIZED
    );
}
#[test]
fn saturated_browser_connections_do_not_consume_rpc_capacity() {
    let f = Fixture::new(false);
    let backend = f.server.as_ref().unwrap().web_admin_address().unwrap();
    let sockets = (0..32)
        .map(|_| TcpStream::connect(backend).unwrap())
        .collect::<Vec<_>>();
    f.client
        .foks()
        .authenticate_and_pin(&f.host, &f.credential)
        .unwrap();
    drop(sockets);
}

#[test]
fn pinned_go_native_handoff_and_browser_ordering() {
    let Some(oracle) = std::env::var_os("FOKS_GO_ORACLE_DIR") else {
        return;
    };
    let f = Fixture::new(false);
    let root = f.environment.client_path("go-admin", "probe.der").unwrap();
    f.environment.write_probe_root(&root).unwrap();
    let ca = f.environment.client_path("go-admin", "admin.der").unwrap();
    std::fs::write(&ca, &f.proxy.certificate).unwrap();
    let output = std::process::Command::new("go")
        .args(["test", "-C"])
        .arg(oracle)
        .args([
            "-mod=readonly",
            "-run",
            "^TestGoAdminAgainstRustServer$",
            "-count=1",
            "-timeout=2m",
            "-v",
        ])
        .env(
            "FOKS_GO_RUST_PROBE",
            f.server.as_ref().unwrap().addresses().probe.to_string(),
        )
        .env("FOKS_GO_RUST_CA_DER", root)
        .env("FOKS_GO_ADMIN_PROXY", f.proxy.address.to_string())
        .env("FOKS_GO_ADMIN_CA_DER", ca)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Go admin gate failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn queued_browser_mutation_rechecks_expiry_at_execution() {
    let f = Fixture::new(true);
    let (session, csrf) = f.login();
    let writer = f.server.as_ref().unwrap().writer_handle();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let blocked = writer.clone();
    let blocker = std::thread::spawn(move || {
        blocked.call(move |_| {
            entered.send(()).unwrap();
            wait.recv().unwrap();
            Ok(())
        })
    });
    started.recv().unwrap();
    std::thread::scope(|scope| {
        let submit = scope.spawn(|| {
            f.proxy.post(
                "/admin/signup-policy",
                &session,
                &format!("csrf={csrf}&revision=1&regime=required&confirm=yes"),
            )
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while writer.metrics().pending < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "browser write did not queue"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        f.environment.advance_clock(301_000_000);
        release.send(()).unwrap();
        assert_eq!(submit.join().unwrap().status(), StatusCode::UNAUTHORIZED);
    });
    blocker.join().unwrap().unwrap();
    assert_eq!(
        f.environment
            .read_database()
            .unwrap()
            .invite_policy()
            .unwrap()
            .regime,
        foks_server_db::InviteRegime::Optional
    );
}

#[test]
fn yubi_owner_uses_the_same_native_handoff_without_a_browser_private_key() {
    use foks_yubi::{MockYubiProvider, Pin, PivPolicy, SlotId, YubiProvider as _};
    let f = Fixture::new(false);
    let client = TestClient::new(&f.environment, "admin-yubi").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let mut store = client.open_protected_store().unwrap();
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("admin-yubi", 73003, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let hardware = provider
        .prepare(
            &card,
            SlotId::new(0x82).unwrap(),
            SlotId::new(0x83).unwrap(),
            &pin,
            None,
            PivPolicy::Once,
            PivPolicy::Never,
        )
        .unwrap();
    let account = client
        .foks()
        .create_yubi_account(
            &host,
            hardware.device.as_ref(),
            foks_client::YubiAccountRequest {
                username_utf8: "adminyubi".into(),
                device_name: "owner key".into(),
                invite_code: foks_proto::InviteCode::Empty,
                email: "adminyubi@example.test".into(),
                passphrase: None,
                pq_hint: foks_proto::YubiSlotAndPqKeyId {
                    slot: 0x83,
                    id: hardware.locator.pq_key_id,
                },
            },
            foks_client::YubiAccountSecrets::new(
                foks_proto::SecretSeed::new([71; 32]),
                foks_proto::SecretSeed::new([72; 32]),
                [73; 17],
            ),
            client.soft_state_path(),
            &mut store,
        )
        .unwrap();
    let handoff = client
        .foks()
        .new_web_admin_handoff(
            &host,
            FederationCredential::Yubi(&account.credential),
            &AdminDestination::new(&f.proxy.origin).unwrap(),
        )
        .unwrap();
    let url = url::Url::parse(handoff.expose()).unwrap();
    let staged = f.proxy.get(&format!("/?{}", url.query().unwrap()), None);
    assert_eq!(staged.status(), StatusCode::SEE_OTHER);
    let pending = cookie(&staged, "__Host-foks_admin_pending");
    let body = f
        .proxy
        .get("/login/confirm", Some(&pending))
        .text()
        .unwrap();
    assert!(body.contains("adminyubi"));
    let csrf = hidden(&body, "csrf");
    assert_eq!(
        f.proxy
            .post("/login/confirm", &pending, &format!("csrf={csrf}"))
            .status(),
        StatusCode::SEE_OTHER
    );
}

#[test]
fn restored_grants_persist_but_restored_browser_credentials_do_not() {
    let mut f = Fixture::new(true);
    let (session, _) = f.login();
    let server = f.server.take().unwrap();
    let addresses = server.addresses();
    let backup = server.backup_named("admin-state").unwrap();
    server.shutdown().unwrap();
    let restored = TestEnvironment::restore_backup(&backup, addresses).unwrap();
    assert_eq!(
        restored
            .read_database()
            .unwrap()
            .admin_operator_count()
            .unwrap(),
        1
    );
    let running = restored
        .start_web_admin_server(f.config.clone(), None)
        .unwrap();
    assert_eq!(
        f.proxy.get("/admin", Some(&session)).status(),
        StatusCode::UNAUTHORIZED
    );
    running.shutdown().unwrap();
}
