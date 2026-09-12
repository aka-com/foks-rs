#![cfg(unix)]
use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, Profile, ProfileRegistry,
    ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::{oidc::TestOidcProvider, TestEnvironment};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Agent(Child);
impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn cli(state: &Path, action: &str, account: &str, handle: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_foks-rs"))
        .arg("--state-dir")
        .arg(state)
        .args([
            "sso",
            action,
            "--profile",
            "local",
            "--account",
            account,
            "--operation",
            handle,
        ])
        .output()
        .unwrap()
}
#[test]
fn cli_and_resident_agent_recover_and_cancel_the_original_protected_flow() {
    let idp = TestOidcProvider::start();
    let env = TestEnvironment::new().unwrap();
    let _server = env
        .start_oidc_server(idp.config(env.client_path("oidc", "secret").unwrap()))
        .unwrap();
    let state = env.client_path("ipc", "state").unwrap();
    let root = env.client_path("ipc", "root.der").unwrap();
    env.write_probe_root(&root).unwrap();
    let credentials =
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".into(),
            probe: format!("localhost:{}", env.addresses().unwrap().probe.port()),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let session = ProfileSession::open(&registry, "local").unwrap();
    let start = credentials
        .with_checked_session(&session, |s| {
            s.probe_and_pin()?;
            let key = credentials.master_key()?;
            let mut secrets = EncryptedFileSecretStore::open(
                &s.paths().credential_store,
                derive_vault_key(&key),
            )?;
            s.begin_account_sso(
                "work",
                false,
                &mut AccountVault::new(&mut secrets),
                &key,
                &foks_oidc::ProviderHttp::new(foks_oidc::NetworkPolicy::loopback_test()).unwrap(),
            )
        })
        .unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_foks-rs")).with_file_name("foks-agent");
    assert!(binary.exists(), "build foks-agent first");
    let _agent = Agent(
        Command::new(binary)
            .arg("--state-dir")
            .arg(&state)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let client = foks_agent_client::AgentClient::new(state.join("foks-rs.sock"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while client.call(foks_agent_proto::Operation::Ping).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let read = cli(&state, "status", "work", &start.operation_id);
    assert!(
        read.status.success(),
        "{}",
        String::from_utf8_lossy(&read.stderr)
    );
    let progress: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
    assert_eq!(progress["state"], "waiting");
    assert_eq!(progress["operation_id"], start.operation_id);
    assert!(!cli(&state, "status", "other", &start.operation_id)
        .status
        .success());
    for _ in 0..2 {
        let result = cli(&state, "cancel", "work", &start.operation_id);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let p: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(p["state"], "cancelled");
        assert!(p["browser_url"].is_null());
    }
    assert!(!cli(&state, "status", "work", "wrong").status.success());
}
