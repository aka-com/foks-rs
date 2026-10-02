use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, KvMutationPrecondition,
    KvRoleSummary, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

#[test]
fn recreated_account_and_team_folders_reject_the_previous_selected_identity() {
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
                let create = |vault: &mut AccountVault<'_>| match team_id {
                    None => session.mkdir_kv("owner", "/source", false, vault, &master),
                    Some(id) => session.mkdir_team_kv_checked(
                        "owner",
                        "group",
                        id,
                        "/source",
                        KvMutationPrecondition::Create,
                        KvRoleSummary::Owner,
                        KvRoleSummary::Owner,
                        false,
                        vault,
                        &master,
                    ),
                };
                let remove =
                    |path: &str, dirent: [u8; 16], version: u64, vault: &mut AccountVault<'_>| {
                        match team_id {
                            None => session.remove_kv_bound(
                                "owner", path, true, dirent, version, vault, &master,
                            ),
                            Some(id) => session.remove_team_kv_bound(
                                "owner", "group", id, path, true, dirent, version, vault, &master,
                            ),
                        }
                    };
                let move_source =
                    |dirent: [u8; 16], version: u64, vault: &mut AccountVault<'_>| match team_id {
                        None => session.move_kv_bound(
                            "owner",
                            "/source",
                            "/destination",
                            dirent,
                            version,
                            vault,
                            &master,
                        ),
                        Some(id) => session.move_team_kv_bound(
                            "owner",
                            "group",
                            id,
                            "/source",
                            "/destination",
                            dirent,
                            version,
                            vault,
                            &master,
                        ),
                    };
                create(&mut vault)?;
                let original = session
                    .data_catalog("owner", team_id, &mut vault)?
                    .entries
                    .into_iter()
                    .find(|entry| entry.metadata.path == "/source")
                    .unwrap()
                    .metadata;
                remove("/source", original.dirent_id, original.version, &mut vault)?;
                create(&mut vault)?;
                let replacement = session
                    .data_catalog("owner", team_id, &mut vault)?
                    .entries
                    .into_iter()
                    .find(|entry| entry.metadata.path == "/source")
                    .unwrap()
                    .metadata;
                assert_eq!(original.version, 1);
                assert_eq!(
                    replacement.version, original.version,
                    "recreation must collide on version to exercise the identity guard"
                );
                assert_ne!(replacement.dirent_id, original.dirent_id);

                let journal = rusqlite::Connection::open(&session.paths().hard_database).unwrap();
                let count_journals = || {
                    journal
                        .query_row("SELECT count(*) FROM mutation_operations", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap()
                };
                let before = count_journals();
                assert!(
                    matches!(
                        move_source(original.dirent_id, original.version, &mut vault),
                        Err(foks_client_app::Error::KvConflict)
                    ),
                    "a stale selection must not move the recreated folder"
                );
                assert!(
                    matches!(
                        remove("/source", original.dirent_id, original.version, &mut vault),
                        Err(foks_client_app::Error::KvConflict)
                    ),
                    "a stale selection must not delete the recreated folder"
                );
                assert_eq!(
                    count_journals(),
                    before,
                    "identity conflicts must fail before preparing any namespace mutation"
                );
                let catalog = session.data_catalog("owner", team_id, &mut vault)?;
                assert!(!catalog
                    .entries
                    .iter()
                    .any(|entry| entry.metadata.path == "/destination"));
                assert_eq!(
                    catalog
                        .entries
                        .iter()
                        .find(|entry| entry.metadata.path == "/source")
                        .unwrap()
                        .metadata
                        .dirent_id,
                    replacement.dirent_id
                );

                // A refreshed selection remains usable: rejection was identity-specific,
                // not a missing capability, permissions failure or blanket write refusal.
                move_source(replacement.dirent_id, replacement.version, &mut vault)?;
                let moved = session
                    .data_catalog("owner", team_id, &mut vault)?
                    .entries
                    .into_iter()
                    .find(|entry| entry.metadata.path == "/destination")
                    .unwrap()
                    .metadata;
                remove("/destination", moved.dirent_id, moved.version, &mut vault)?;
                assert!(session
                    .data_catalog("owner", team_id, &mut vault)?
                    .entries
                    .is_empty());
            }
            Ok::<(), foks_client_app::Error>(())
        })
        .unwrap();
}
