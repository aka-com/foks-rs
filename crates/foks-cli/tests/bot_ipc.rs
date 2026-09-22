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

fn bot(state: &Path, action: &str, alias: &str, args: &[&str]) -> std::process::Output {
    let mut all = vec![
        "account",
        "bot",
        action,
        "--profile",
        "local",
        "--account-alias",
        alias,
    ];
    all.extend(args);
    cli(state, &all)
}
fn success(output: std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn bot_secrets_cross_real_agent_only_through_private_files_and_live_sessions() {
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
    while client.call(foks_agent_proto::Operation::Ping).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }

    let start = success(bot(&state, "prepare", "work", &["--role", "owner"]));
    let handle = start["operation_id"].as_str().unwrap();
    assert_eq!(start["state"], "prepared");
    let done = success(bot(&state, "attempt", "work", &["--operation", handle]));
    assert_eq!(done["state"], "complete");
    let occupied = state.join("occupied");
    std::fs::write(&occupied, "preserve").unwrap();
    assert!(!bot(
        &state,
        "export",
        "work",
        &[
            "--operation",
            handle,
            "--output",
            occupied.to_str().unwrap()
        ]
    )
    .status
    .success());
    let path = state.join("bot-token");
    let exported = success(bot(
        &state,
        "export",
        "work",
        &["--operation", handle, "--output", path.to_str().unwrap()],
    ));
    assert!(exported.get("token").is_none());
    assert!(!bot(
        &state,
        "export",
        "work",
        &[
            "--operation",
            handle,
            "--output",
            state.join("again").to_str().unwrap()
        ]
    )
    .status
    .success());
    success(bot(
        &state,
        "load",
        "bot",
        &["--input", path.to_str().unwrap()],
    ));
    success(cli(
        &state,
        &["account", "sync", "local", "--account-alias", "bot"],
    ));
    let binding = client
        .call(foks_agent_proto::Operation::BindDataAccount {
            profile: "local".into(),
            account_alias: "bot".into(),
        })
        .unwrap();
    let foks_agent_proto::ResponseResult::Success { value } = binding.result else {
        panic!("MCP bot binding failed")
    };
    let scope: foks_agent_proto::data::DataScope = serde_json::from_value(value).unwrap();
    for query in [
        foks_agent_proto::data::DataRead::Usage,
        foks_agent_proto::data::DataRead::Memberships,
    ] {
        let result = client
            .call(foks_agent_proto::Operation::ReadData {
                scope: scope.clone(),
                query,
            })
            .unwrap();
        assert!(
            matches!(
                result.result,
                foks_agent_proto::ResponseResult::Success { .. }
            ),
            "bot data access failed"
        );
    }

    success(bot(&state, "unload", "bot", &[]));
    assert!(!cli(
        &state,
        &["account", "sync", "local", "--account-alias", "bot"]
    )
    .status
    .success());
    success(bot(
        &state,
        "load",
        "bot",
        &["--input", path.to_str().unwrap()],
    ));
    drop(agent);
    // A new resident process sees the public selection, but has no signing seed.
    let _agent = Agent(
        Command::new(Path::new(env!("CARGO_BIN_EXE_foks-rs")).with_file_name("foks-agent"))
            .arg("--state-dir")
            .arg(&state)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while client.call(foks_agent_proto::Operation::Ping).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let list = success(cli(&state, &["account", "list", "local"]));
    assert!(list.to_string().contains("bot"));
    assert!(!cli(
        &state,
        &["account", "sync", "local", "--account-alias", "bot"]
    )
    .status
    .success());
    success(bot(
        &state,
        "load",
        "bot",
        &["--input", path.to_str().unwrap()],
    ));
    let target = start["device_id"].as_str().unwrap();
    let revoked = success(bot(&state, "revoke", "work", &["--device-id", target]));
    assert_eq!(revoked["currently_active"], false);
    assert!(!cli(
        &state,
        &["account", "sync", "local", "--account-alias", "bot"]
    )
    .status
    .success());
    assert!(
        !bot(&state, "load", "bot", &["--input", path.to_str().unwrap()])
            .status
            .success()
    );
    success(cli(
        &state,
        &["account", "sync", "local", "--account-alias", "work"],
    ));
}
