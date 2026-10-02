use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, KvMutationPrecondition,
    KvRoleSummary, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

#[test]
fn nonrecursive_folder_deletion_requires_empty_and_exact_source() {
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

            let folder = session.mkdir_kv("owner", "/empty", false, &mut vault, &master)?;
            assert!(session
                .remove_kv_checked(
                    "owner",
                    "/empty",
                    false,
                    folder.version + 1,
                    &mut vault,
                    &master
                )
                .is_err());
            session.remove_kv_checked(
                "owner",
                "/empty",
                false,
                folder.version,
                &mut vault,
                &master,
            )?;
            assert!(!session
                .list_kv_metadata("owner", &mut vault)?
                .entries
                .iter()
                .any(|entry| entry.path == "/empty"));
            let folder = session.mkdir_kv("owner", "/occupied", false, &mut vault, &master)?;
            session.put_kv_file(
                "owner",
                "/occupied/child",
                &mut b"keep".as_slice(),
                false,
                false,
                &mut vault,
                &master,
            )?;
            assert!(session
                .remove_kv_checked(
                    "owner",
                    "/occupied",
                    false,
                    folder.version,
                    &mut vault,
                    &master
                )
                .is_err());
            assert_eq!(
                session.read_kv_file("owner", "/occupied/child", &mut vault)?,
                b"keep"
            );
            assert!(session
                .remove_kv_checked("owner", "/", false, 1, &mut vault, &master)
                .is_err());
            // An explicitly requested legacy recursive removal remains supported.
            session.remove_kv_checked(
                "owner",
                "/occupied",
                true,
                folder.version,
                &mut vault,
                &master,
            )?;
            let team =
                session.create_named_team("owner", "group", "folderteam", &mut vault, &master)?;
            let folder = session.mkdir_team_kv_checked(
                "owner",
                "group",
                &team.team_id_hex,
                "/empty",
                KvMutationPrecondition::Create,
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                false,
                &mut vault,
                &master,
            )?;
            assert!(session
                .remove_team_kv_checked(
                    "owner",
                    "group",
                    &team.team_id_hex,
                    "/empty",
                    false,
                    folder.version + 1,
                    &mut vault,
                    &master,
                )
                .is_err());
            session.remove_team_kv_checked(
                "owner",
                "group",
                &team.team_id_hex,
                "/empty",
                false,
                folder.version,
                &mut vault,
                &master,
            )?;
            assert!(!session
                .data_catalog("owner", Some(&team.team_id_hex), &mut vault)?
                .entries
                .iter()
                .any(|entry| entry.metadata.path == "/empty"));
            Ok::<(), foks_client_app::Error>(())
        })
        .unwrap();
}
