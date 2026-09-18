#![cfg(unix)]
use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, Profile, ProfileRegistry,
    ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;
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
mod support;
use support::cli;

fn success(output: std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn admin_configuration_and_checks_use_the_real_agent_without_printing_sessions() {
    let env = TestEnvironment::new().unwrap();
    let _server = env.start_server().unwrap();
    let state = env.client_path("ipc", "state").unwrap();
    let root = env.client_path("ipc", "root.der").unwrap();
    env.write_probe_root(&root).unwrap();
    let credentials =
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".into(),
            label: None,
            probe: format!("localhost:{}", env.addresses().unwrap().probe.port()),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let session = ProfileSession::open(&registry, "local").unwrap();
    credentials
        .with_checked_session(&session, |s| {
            s.probe_and_pin()?;
            let key = credentials.master_key()?;
            let mut secrets = EncryptedFileSecretStore::open(
                &s.paths().credential_store,
                derive_vault_key(&key),
            )?;
            let mut vault = AccountVault::new(&mut secrets);
            s.create_account(
                "work",
                "ipcrename",
                "laptop",
                "",
                "",
                None,
                &mut vault,
                &key,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_foks-rs")).with_file_name("foks-agent");
    assert!(binary.exists(), "build foks-agent first");
    let agent = Agent(
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
    while let Err(error) = client.call(foks_agent_proto::Operation::Ping) {
        assert!(Instant::now() < deadline, "agent readiness failed: {error}");
        std::thread::sleep(Duration::from_millis(20));
    }

    success(cli(
        &state,
        &[
            "account",
            "admin",
            "configure",
            "--profile",
            "local",
            "--account",
            "work",
            "--destination",
            "https://admin.example/",
        ],
    ));
    let checked = cli(
        &state,
        &[
            "account",
            "admin",
            "check",
            "--profile",
            "local",
            "--account",
            "work",
        ],
    );
    assert!(!checked.status.success());
    assert!(String::from_utf8_lossy(&checked.stderr).contains("unsupported"));
    assert!(checked.stdout.is_empty());
    let rejected = cli(
        &state,
        &[
            "account",
            "admin",
            "configure",
            "--profile",
            "local",
            "--account",
            "work",
            "--destination",
            "https://admin.example/?s=private-bearer",
        ],
    );
    assert!(!rejected.status.success());
    assert!(!String::from_utf8_lossy(&rejected.stderr).contains("private-bearer"));
    drop(agent);
}
