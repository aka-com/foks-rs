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

#[test]
fn resident_agent_upload_intent_survives_early_disconnect_and_finishes_without_source() {
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
                "ipcupload",
                "laptop",
                "",
                "",
                None,
                &mut vault,
                &key,
            )?;
            Ok::<(), foks_client_app::Error>(())
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

    use foks_agent_proto::{
        AccountStoreRef, KvPrecondition, KvRole, KvStoreRef, KvUploadHeader, Operation, Response,
        ResponseResult,
    };
    fn success(response: Response) -> serde_json::Value {
        match response.result {
            ResponseResult::Success { value } => value,
            ResponseResult::Error { code, message, .. } => {
                panic!("agent failed: {code:?}: {message}")
            }
        }
    }
    let store = KvStoreRef::Account(AccountStoreRef {
        profile: "local".into(),
        account_alias: "work".into(),
    });
    let header = |id| KvUploadHeader {
        store: store.clone(),
        path: "/file".into(),
        total_length: 3000,
        precondition: KvPrecondition::Create,
        read_role: KvRole::Owner,
        write_role: KvRole::Owner,
        mkdir_p: false,
        adapter: None,
        upload_id: Some(id),
    };
    struct Disconnected;
    impl std::io::Read for Disconnected {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("source lost before first byte"))
        }
    }
    let context: foks_agent_proto::KvUploadIssuance = serde_json::from_value(success(
        client
            .call(Operation::KvUploadIssuance {
                store: store.clone(),
            })
            .unwrap(),
    ))
    .unwrap();
    let lost =
        foks_agent_proto::UploadHandle::new(context.database_id, context.issued_at, [41; 16]);
    assert!(client
        .put_kv_stream(header(lost), &mut Disconnected)
        .is_err());
    let inventory = success(
        client
            .call(Operation::ListKvUploads {
                store: store.clone(),
                after: None,
            })
            .unwrap(),
    );
    let row = &inventory["uploads"][0];
    assert_eq!(row["upload_id"], lost.to_string());
    assert_eq!(row["state"], "needs-source");
    let cancelled = success(
        client
            .call(Operation::CancelKvUpload {
                store: store.clone(),
                upload_id: lost,
            })
            .unwrap(),
    );
    assert_eq!(cancelled["state"], "cancelled");
    let completed =
        foks_agent_proto::UploadHandle::new(context.database_id, context.issued_at, [42; 16]);
    success(
        client
            .put_kv_stream(header(completed), &mut std::io::Cursor::new(vec![7; 3000]))
            .unwrap(),
    );
    let receipt = success(
        client
            .call(Operation::ResumeKvUpload {
                store: store.clone(),
                upload_id: completed,
            })
            .unwrap(),
    );
    assert_eq!(receipt["state"], "completed");
    let repeated = success(
        client
            .call(Operation::ResumeKvUpload {
                store,
                upload_id: completed,
            })
            .unwrap(),
    );
    // The resident agent's retention maintenance may erase the completed
    // upload's spool between the two calls, which releases its reservation.
    // That resume itself never erases the spool is covered without a
    // resident agent in foks-client-app's upload tests.
    let without_reservation = |mut report: serde_json::Value| {
        report.as_object_mut().unwrap().remove("reserved_bytes");
        report
    };
    assert!(
        repeated["reserved_bytes"].as_u64().unwrap() <= receipt["reserved_bytes"].as_u64().unwrap()
    );
    assert_eq!(without_reservation(receipt), without_reservation(repeated));
}
