use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, Profile, ProfileRegistry,
    ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

#[test]
fn adapter_writes_are_bound_durable_and_never_replayed() {
    let issued_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let handle = |byte| foks_client_app::SubmissionHandle::new(issued_at, [byte; 16]);

    let environment = TestEnvironment::new().unwrap();
    let _server = environment.start_server().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let root = state.join("root.der");
    environment.write_probe_root(&root).unwrap();
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".into(),
            probe: format!(
                "localhost:{}",
                environment.addresses().unwrap().probe.port()
            ),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let session = ProfileSession::open(&registry, "local").unwrap();
    let credentials = ClientCredentials::open(&state).unwrap();
    credentials
        .with_checked_session(&session, |session| {
            session.probe_and_pin()?;
            let master = credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "owner",
                "dataowner",
                "device",
                "owner@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            use foks_client_app::{
                DataWriteKind as Kind, DataWriteSpec, DataWriteStatus as Status,
            };
            let spec = |kind, path: &str, body: &[u8]| DataWriteSpec {
                kind,
                path: path.into(),
                destination: None,
                team_selector: None,
                overwrite: false,
                mkdir_p: false,
                recursive: false,
                body_length: body.len() as u64,
                body_hash: foks_crypto::kv_adapter_body_hash(body),
            };
            let bytes = vec![42; 300 * 1024];
            let mut put = spec(Kind::Put, "/parents/file", &bytes);
            put.mkdir_p = true;
            assert_eq!(
                session
                    .prepare_data_write("owner", handle(1), put.clone(), &mut vault, &master)?
                    .status,
                Status::Prepared
            );
            assert_eq!(session.pending_data_writes("owner", &mut vault)?.len(), 1);
            // Wrong/interrupted body cannot cross the durable submission boundary.
            assert!(session
                .execute_data_write(
                    "owner",
                    handle(1),
                    &mut b"wrong".as_slice(),
                    &mut vault,
                    &master
                )
                .is_err());
            assert_eq!(
                session
                    .data_write_status("owner", handle(1), &mut vault, &master)?
                    .status,
                Status::Prepared
            );
            let result = session.execute_data_write(
                "owner",
                handle(1),
                &mut bytes.as_slice(),
                &mut vault,
                &master,
            )?;
            assert_eq!(result.status, Status::Committed);
            let before = session.data_catalog("owner", None, &mut vault)?;
            // Lost final reply followed by prepare and execute with the same ID.
            assert_eq!(
                session
                    .prepare_data_write("owner", handle(1), put.clone(), &mut vault, &master)?
                    .status,
                Status::Committed
            );
            assert_eq!(
                session
                    .execute_data_write(
                        "owner",
                        handle(1),
                        &mut std::io::empty(),
                        &mut vault,
                        &master
                    )?
                    .status,
                Status::Committed
            );
            assert_eq!(
                session
                    .data_catalog("owner", None, &mut vault)?
                    .snapshot_version,
                before.snapshot_version
            );
            put.path = "/different".into();
            assert!(session
                .prepare_data_write("owner", handle(1), put, &mut vault, &master)
                .is_err());
            let hard = foks_client_db::HardStateStore::open(&session.paths().hard_database)?;
            let retained = hard.adapter_submission(handle(1))?.unwrap();
            assert_eq!(
                retained.state,
                foks_client_db::AdapterLedgerState::Committed
            );
            assert!(retained.internal_id.is_none());
            assert_eq!(
                retained
                    .node_id
                    .map(|id| id.iter().map(|b| format!("{b:02x}")).collect::<String>()),
                result.node_id
            );
            let mut mv = spec(Kind::Move, "/parents/file", &[]);
            mv.destination = Some("/parents/moved".into());
            session.prepare_data_write("owner", handle(2), mv, &mut vault, &master)?;
            assert_eq!(
                session
                    .execute_data_write(
                        "owner",
                        handle(2),
                        &mut std::io::empty(),
                        &mut vault,
                        &master
                    )?
                    .status,
                Status::Committed
            );
            // Namespace response loss can be reconciled by the exact created dirent.
            session.prepare_data_write(
                "owner",
                handle(3),
                spec(Kind::Put, "/lost", b"lost"),
                &mut vault,
                &master,
            )?;
            let hits = environment
                .arm_fault(foks_server_testkit::TestFault::KvPutAfterCommitBeforeResponse);
            let first = session.execute_data_write(
                "owner",
                handle(3),
                &mut b"lost".as_slice(),
                &mut vault,
                &master,
            )?;
            assert!(matches!(
                first.status,
                Status::Committed | Status::SubmissionUnknown
            ));
            assert_eq!(
                session
                    .data_write_status("owner", handle(3), &mut vault, &master)?
                    .status,
                Status::Committed
            );
            assert_eq!(environment.fault_hits(), hits + 1);
            session.prepare_data_write(
                "owner",
                handle(4),
                spec(Kind::Remove, "/lost", &[]),
                &mut vault,
                &master,
            )?;
            environment.arm_fault(foks_server_testkit::TestFault::KvPutAfterCommitBeforeResponse);
            assert_eq!(
                session
                    .execute_data_write(
                        "owner",
                        handle(4),
                        &mut std::io::empty(),
                        &mut vault,
                        &master
                    )?
                    .status,
                Status::SubmissionUnknown
            );
            // Current absence cannot identify which unlink committed.
            assert_eq!(
                session
                    .data_write_status("owner", handle(4), &mut vault, &master)?
                    .status,
                Status::SubmissionUnknown
            );
            assert_eq!(
                hard.mutation(
                    &hard
                        .adapter_submission(handle(4))?
                        .unwrap()
                        .internal_id
                        .unwrap()
                )?
                .unwrap()
                .attempt_count,
                1
            );
            // An existing-directory mkdir -p is an explicit successful no-op.
            let mut mkdir = spec(Kind::Mkdir, "/parents", &[]);
            mkdir.mkdir_p = true;
            session.prepare_data_write("owner", handle(5), mkdir, &mut vault, &master)?;
            assert_eq!(
                session
                    .execute_data_write(
                        "owner",
                        handle(5),
                        &mut std::io::empty(),
                        &mut vault,
                        &master
                    )?
                    .status,
                Status::Committed
            );
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
}
