use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, KvMutationPrecondition,
    KvRoleSummary, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

#[test]
fn recreated_account_and_team_files_reject_stale_overwrites() {
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

            let team =
                session.create_named_team("owner", "group", "identityteam", &mut vault, &master)?;
            for team_id in [None, Some(team.team_id_hex.as_str())] {
                let put = |body: &[u8], precondition, vault: &mut AccountVault<'_>| {
                    let mut reader = body;
                    match team_id {
                        None => session.put_kv_file_checked_with_size(
                            "owner",
                            "/source",
                            &mut reader,
                            Some(body.len() as u64),
                            precondition,
                            KvRoleSummary::Owner,
                            KvRoleSummary::Owner,
                            false,
                            vault,
                            &master,
                        ),
                        Some(id) => session.put_team_kv_file_checked_with_size(
                            "owner",
                            "group",
                            id,
                            "/source",
                            &mut reader,
                            Some(body.len() as u64),
                            precondition,
                            KvRoleSummary::Owner,
                            KvRoleSummary::Owner,
                            false,
                            vault,
                            &master,
                        ),
                    }
                };
                put(b"original", KvMutationPrecondition::Create, &mut vault)?;
                let original = session
                    .data_catalog("owner", team_id, &mut vault)?
                    .entries
                    .into_iter()
                    .find(|entry| entry.metadata.path == "/source")
                    .unwrap()
                    .metadata;
                match team_id {
                    None => session.remove_kv_bound(
                        "owner",
                        "/source",
                        false,
                        original.dirent_id,
                        original.version,
                        &mut vault,
                        &master,
                    )?,
                    Some(id) => session.remove_team_kv_bound(
                        "owner",
                        "group",
                        id,
                        "/source",
                        false,
                        original.dirent_id,
                        original.version,
                        &mut vault,
                        &master,
                    )?,
                };
                put(b"replacement", KvMutationPrecondition::Create, &mut vault)?;
                let replacement = session
                    .data_catalog("owner", team_id, &mut vault)?
                    .entries
                    .into_iter()
                    .find(|entry| entry.metadata.path == "/source")
                    .unwrap()
                    .metadata;
                assert_eq!(replacement.version, original.version);
                assert_ne!(replacement.dirent_id, original.dirent_id);
                let journal = rusqlite::Connection::open(&session.paths().hard_database).unwrap();
                let count = || {
                    journal
                        .query_row("SELECT count(*) FROM mutation_operations", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap()
                };
                let before = count();
                for body in [b"stale text".to_vec(), vec![42; 8192]] {
                    assert!(matches!(
                        put(
                            &body,
                            KvMutationPrecondition::ExactEntry {
                                dirent_id: original.dirent_id,
                                version: original.version,
                            },
                            &mut vault
                        ),
                        Err(foks_client_app::Error::KvConflict)
                    ));
                }
                assert_eq!(count(), before, "stale selection cannot prepare a mutation");
                let after = session
                    .data_catalog("owner", team_id, &mut vault)?
                    .entries
                    .into_iter()
                    .find(|entry| entry.metadata.path == "/source")
                    .unwrap()
                    .metadata;
                assert_eq!(after, replacement);
                for body in [b"fresh text".to_vec(), vec![24; 8192]] {
                    let selected = session
                        .data_catalog("owner", team_id, &mut vault)?
                        .entries
                        .into_iter()
                        .find(|entry| entry.metadata.path == "/source")
                        .unwrap()
                        .metadata;
                    put(
                        &body,
                        KvMutationPrecondition::ExactEntry {
                            dirent_id: selected.dirent_id,
                            version: selected.version,
                        },
                        &mut vault,
                    )?;
                    let after = session
                        .data_catalog("owner", team_id, &mut vault)?
                        .entries
                        .into_iter()
                        .find(|entry| entry.metadata.path == "/source")
                        .unwrap()
                        .metadata;
                    assert_eq!(after.dirent_id, replacement.dirent_id);
                    assert_eq!(after.version, selected.version + 1);
                }
            }
            Ok::<(), foks_client_app::Error>(())
        })
        .unwrap();
}

#[test]
fn overwrite_refresh_cannot_rebind_after_a_peer_recreates_the_source_during_upload() {
    use foks_client::KvWriteOptions;
    use foks_proto::Role;
    use foks_server_testkit::{TestAccountSpec, TestClient};
    use std::io::{Cursor, Read};

    struct RacingReader<F: FnMut()> {
        before_read: Option<F>,
        bytes: Cursor<Vec<u8>>,
    }
    impl<F: FnMut()> Read for RacingReader<F> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if let Some(mut action) = self.before_read.take() {
                action();
            }
            self.bytes.read(output)
        }
    }
    let environment = TestEnvironment::new().unwrap();
    let _server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "overwrite-main").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let created = client
        .create_account(&probe.pinned, &TestAccountSpec::new("overwriteuser", 0x47))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let peer = TestClient::new(&environment, "overwrite-peer").unwrap();
    let peer_probe = peer.probe_and_pin().unwrap();
    let authenticated = peer
        .foks()
        .authenticate_and_pin(&peer_probe.pinned, &created.credential)
        .unwrap();
    let mut protected = client.open_protected_store().unwrap();
    let mut peer_protected = peer.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .user_kv_write_session(
            &probe.pinned,
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let mut peer_session = peer
        .foks()
        .user_kv_write_session(
            &peer_probe.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            peer.soft_state_path(),
            &mut peer_protected,
        )
        .unwrap();
    let create = KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    };
    for (index, size) in [10, 8192].into_iter().enumerate() {
        let name = format!("source-{index}");
        let initial = session
            .put_file(root, &name, &mut Cursor::new(b"original"), create)
            .unwrap();
        let original = initial
            .path
            .iter()
            .find(|directory| directory.directory_id == root)
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .unwrap()
            .clone();
        let mut reader = RacingReader {
            before_read: Some(|| {
                peer_session
                    .unlink_bound(
                        root,
                        &name,
                        original.dirent_id,
                        original.version,
                        Role::OWNER,
                        false,
                    )
                    .unwrap();
                peer_session
                    .put_file(root, &name, &mut Cursor::new(b"peer replacement"), create)
                    .unwrap();
            }),
            bytes: Cursor::new(vec![42; size]),
        };
        let error = session
            .put_file_with_size_bound(
                root,
                &name,
                &mut reader,
                Some(size as u64),
                KvWriteOptions {
                    overwrite: true,
                    expected_version: Some(original.version),
                    ..create
                },
                Some(original.dirent_id),
            )
            .unwrap_err();
        assert!(
            matches!(
                error,
                foks_client::Error::KvResponse("KV dirent identity precondition failed")
            ),
            "{error}"
        );
        let refreshed = session.sync().unwrap();
        let replacement = refreshed
            .iter()
            .find(|directory| directory.directory_id == root)
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .unwrap();
        assert_ne!(replacement.dirent_id, original.dirent_id);
        assert_eq!(replacement.version, original.version);
        assert_eq!(
            client
                .foks()
                .read_user_kv_node(
                    &probe.pinned,
                    &created.credential,
                    &created.authenticated.verified,
                    &created.authenticated.puks,
                    foks_proto::KvNodeId(replacement.node_id)
                )
                .unwrap(),
            foks_client::KvFetchedNode::SmallFile(b"peer replacement".to_vec())
        );
    }
}
