use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, Profile, ProfileRegistry,
    ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

#[test]
fn checked_moves_preserve_selected_versions_and_never_overwrite_destinations() {
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

            let put = session.put_kv_file(
                "owner",
                "/source",
                &mut b"original".as_slice(),
                false,
                false,
                &mut vault,
                &master,
            )?;
            session.mkdir_kv("owner", "/folder", false, &mut vault, &master)?;
            session.put_kv_file(
                "owner",
                "/occupied",
                &mut b"keep".as_slice(),
                false,
                false,
                &mut vault,
                &master,
            )?;
            assert!(session
                .move_kv_checked(
                    "owner",
                    "/source",
                    "/occupied",
                    put.version,
                    &mut vault,
                    &master
                )
                .is_err());
            assert!(session
                .move_kv_checked(
                    "owner",
                    "/source",
                    "/folder/renamed",
                    put.version + 1,
                    &mut vault,
                    &master
                )
                .is_err());
            let moved = session.move_kv_checked(
                "owner",
                "/source",
                "/folder/renamed",
                put.version,
                &mut vault,
                &master,
            )?;
            assert_eq!(moved.path, "/folder/renamed");
            assert_eq!(
                session.read_kv_file("owner", "/folder/renamed", &mut vault)?,
                b"original"
            );
            assert_eq!(
                session.read_kv_file("owner", "/occupied", &mut vault)?,
                b"keep"
            );
            assert!(session
                .read_kv_file("owner", "/source", &mut vault)
                .is_err());
            let directory = session
                .list_kv_metadata("owner", &mut vault)?
                .entries
                .into_iter()
                .find(|entry| entry.path == "/folder")
                .unwrap();
            assert!(session
                .move_kv_checked(
                    "owner",
                    "/folder",
                    "/folder/inside",
                    directory.version,
                    &mut vault,
                    &master
                )
                .is_err());
            session.move_kv_checked(
                "owner",
                "/folder",
                "/renamed-folder",
                directory.version,
                &mut vault,
                &master,
            )?;
            assert_eq!(
                session.read_kv_file("owner", "/renamed-folder/renamed", &mut vault)?,
                b"original"
            );
            Ok::<(), foks_client_app::Error>(())
        })
        .unwrap();
}
