use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, KvMutationPrecondition,
    KvRoleSummary, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

#[test]
fn scoped_reads_bind_identity_preserve_absent_roots_and_verify_team_and_file_data() {
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
            label: None,
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
            let (host, user) = session.data_identity("owner", &mut vault)?;
            session.check_data_identity("owner", &host, &user, &mut vault)?;
            assert!(session
                .check_data_identity("owner", &"00".repeat(33), &user, &mut vault)
                .is_err());
            assert!(session
                .check_data_identity("owner", &host, &"00".repeat(33), &mut vault)
                .is_err());
            let db = rusqlite::Connection::open(environment.database_path()).unwrap();
            assert_eq!(db.execute("DELETE FROM kv_roots", []).unwrap(), 1);
            assert!(
                session.data_catalog("owner", None, &mut vault).is_err(),
                "disappearance of a previously known root must remain an error"
            );
            // Model a newly provisioned Go client that has never cached a KV root.
            std::fs::remove_file(&session.paths().soft_database).unwrap();
            let empty = session.data_catalog("owner", None, &mut vault)?;
            assert!(empty.root_id.is_none() && empty.entries.is_empty());
            let _usage = session.data_usage("owner", None, &mut vault)?;
            let roots: i64 = db
                .query_row("SELECT count(*) FROM kv_roots", [], |row| row.get(0))
                .unwrap();
            assert_eq!(roots, 0, "read adapters must not initialize the namespace");
            let data = vec![0x61; 300 * 1024];
            let data_size = data.len() as u64;
            session.put_kv_file_checked_with_size(
                "owner",
                "/large",
                &mut data.as_slice(),
                Some(data_size),
                KvMutationPrecondition::Create,
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                false,
                &mut vault,
                &master,
            )?;
            let catalog = session.data_catalog("owner", None, &mut vault)?;
            let row = catalog
                .entries
                .iter()
                .find(|entry| entry.metadata.path == "/large")
                .unwrap();
            assert!(row.modified_microseconds > 0);
            let node =
                session.data_entry("owner", None, "/large", row.metadata.version, &mut vault)?;
            assert_eq!(node.size, Some(data_size));
            assert!(
                node.content.is_none(),
                "large-file metadata must not fetch content"
            );
            assert!(session
                .put_kv_file_checked_with_size(
                    "owner",
                    "/wrong-size",
                    &mut data.as_slice(),
                    Some(data_size - 1),
                    KvMutationPrecondition::Create,
                    KvRoleSummary::Owner,
                    KvRoleSummary::Owner,
                    false,
                    &mut vault,
                    &master,
                )
                .is_err());
            assert!(session
                .data_catalog("owner", None, &mut vault)?
                .entries
                .iter()
                .all(|entry| entry.metadata.path != "/wrong-size"));
            let chunk = session.data_chunk(
                "owner",
                None,
                "/large",
                row.metadata.version,
                128 * 1024,
                128 * 1024,
                &mut vault,
            )?;
            assert_eq!(chunk.content, data[128 * 1024..256 * 1024]);
            assert!(!chunk.eof);
            assert!(session
                .data_chunk(
                    "owner",
                    None,
                    "/large",
                    row.metadata.version + 1,
                    0,
                    8,
                    &mut vault
                )
                .is_err());
            let (files, bytes) = session.data_usage("owner", None, &mut vault)?;
            assert!(files >= 1 && bytes >= data.len() as u64);
            let created =
                session.create_named_team("owner", "group", "datateam", &mut vault, &master)?;
            let team_id = session.resolve_data_team("owner", "datateam", &mut vault)?;
            assert_eq!(team_id, created.team_id_hex);
            assert_eq!(
                session.resolve_data_team("owner", "group", &mut vault)?,
                team_id
            );
            let members = session.data_members("owner", &team_id, &mut vault)?;
            assert_eq!(members.len(), 1);
            assert_eq!(members[0].name, "dataowner");
            assert_eq!(members[0].destination_role, "o");
            assert!(members[0].added_millis > 0);
            let memberships = session.data_memberships("owner", &mut vault)?;
            assert!(memberships.complete);
            assert!(memberships
                .teams
                .iter()
                .any(|row| row.team_id == team_id && row.via.is_none()));
            session.data_catalog("owner", Some(&team_id), &mut vault)?;
            assert!(session
                .data_catalog("owner", Some(&"03".repeat(33)), &mut vault)
                .is_err());
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
}
