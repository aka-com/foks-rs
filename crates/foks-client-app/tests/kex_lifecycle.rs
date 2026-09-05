use std::path::{Path, PathBuf};

use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, KexAcceptanceInput,
    KvMutationPrecondition, KvRoleSummary, Passphrase, Profile, ProfileRegistry, ProfileSession,
    ProtocolPolicy, TrustRoot,
};
use foks_keystore::{EncryptedFileSecretStore, SecretStore as _};
use foks_server_testkit::TestEnvironment;

#[test]
fn protected_product_workflow_pairs_two_machine_state_roots() {
    paired_workflow(None);
}

#[test]
fn paired_account_without_a_kv_root_lists_empty_and_initializes_on_first_write() {
    // Check the desktop's versioned write, CLI upload, and mkdir entry points.
    for first_write in 0..3 {
        paired_workflow(Some(first_write));
    }
}

fn paired_workflow(first_write: Option<u8>) {
    let environment = TestEnvironment::new().unwrap();
    let _server = environment.start_server().unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let provisioner_state = temporary.path().join("provisioner");
    let provisionee_state = temporary.path().join("provisionee");
    let port = environment.addresses().unwrap().probe.port();
    let (provisioner_registry, provisioner, provisioner_credentials) =
        machine(&environment, &provisioner_state, port);
    let (_provisionee_registry, provisionee, provisionee_credentials) =
        machine(&environment, &provisionee_state, port);

    let offer = provisioner_credentials
        .with_checked_session(&provisioner, |session| {
            let master = provisioner_credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "owner",
                "kexproductowner",
                "original device",
                "owner@example.test",
                "",
                Some(Passphrase::new("paired-account passphrase")?),
                &mut vault,
                &master,
            )?;
            if first_write.is_some() {
                // Simulate the Go signup state before its first KV write. This
                // database belongs exclusively to this disposable test server.
                let db = rusqlite::Connection::open(environment.database_path()).unwrap();
                assert_eq!(db.execute("DELETE FROM kv_roots", []).unwrap(), 1);
            }
            session.start_owner_device_pairing("owner", &mut vault)
        })
        .unwrap();
    assert_eq!(offer.phrase.split_whitespace().count(), 13);
    assert!(!format!("{offer:?}").contains(&offer.phrase));

    let finish_state = provisioner_state.clone();
    let finish = std::thread::spawn(move || {
        let registry = ProfileRegistry::open(&finish_state).unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        let credentials = ClientCredentials::open(&finish_state).unwrap();
        credentials
            .with_checked_session(&session, |session| {
                let master = credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.finish_owner_device_pairing("owner", &mut vault, &master)
            })
            .unwrap()
    });

    let accepted = provisionee_credentials
        .with_checked_session(&provisionee, |session| {
            let master = provisionee_credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.accept_owner_device_pairing(
                KexAcceptanceInput {
                    target_alias: "owner".to_owned(),
                    device_name: "paired laptop".to_owned(),
                    serial: 2,
                    phrase: offer.phrase.clone(),
                },
                &mut vault,
            )
        })
        .unwrap();
    let finished = finish.join().unwrap();
    assert_eq!(accepted.user_chain_sequence, 2);
    assert_eq!(finished.user_chain_sequence, 2);
    assert_eq!(accepted.device_id_hex, finished.device_id_hex);

    provisionee_credentials
        .with_checked_session(&provisionee, |session| {
            let master = provisionee_credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let account: serde_json::Value = serde_json::from_slice(&store.get("account.owner")?)?;
            let pending = serde_json::json!({
                "version": 1,
                "target_alias": "owner",
                "device_name": "paired laptop",
                "serial": 2,
                "device_seed": account["device_seed"],
                "phrase": offer.phrase,
            });
            store.put("pending-kex.owner", &serde_json::to_vec(&pending)?)?;
            let mut vault = AccountVault::new(&mut store);
            let resumed = session.resume_owner_device_pairing_acceptance("owner", &mut vault)?;
            assert_eq!(resumed.device_id_hex, accepted.device_id_hex);
            assert_eq!(vault.aliases()?, vec!["owner".to_owned()]);
            assert_eq!(
                session
                    .sync_account("owner", &mut vault)?
                    .user_chain_sequence,
                2
            );
            if let Some(first_write) = first_write {
                let db = rusqlite::Connection::open(environment.database_path()).unwrap();
                let root_count = || {
                    db.query_row("SELECT count(*) FROM kv_roots", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap()
                };
                assert!(session.list_kv("owner", &mut vault)?.entries.is_empty());
                let catalog = session.list_kv_metadata("owner", &mut vault)?;
                assert!(catalog.entries.is_empty());
                assert_eq!(catalog.snapshot_version, 0);
                assert_eq!(root_count(), 0, "reads must not create a root");
                assert!(session
                    .put_kv_file_checked(
                        "owner",
                        "/missing",
                        &mut &b"no overwrite"[..],
                        KvMutationPrecondition::ExactVersion(1),
                        KvRoleSummary::Owner,
                        KvRoleSummary::Owner,
                        false,
                        &mut vault,
                        &master,
                    )
                    .is_err());
                assert_eq!(
                    root_count(),
                    0,
                    "failed versioned update must not create a root"
                );
                match first_write {
                    0 => {
                        session.put_kv_file_checked(
                            "owner",
                            "/first",
                            &mut &b"first value"[..],
                            KvMutationPrecondition::Create,
                            KvRoleSummary::Owner,
                            KvRoleSummary::Owner,
                            false,
                            &mut vault,
                            &master,
                        )?;
                    }
                    1 => {
                        session.put_kv_file(
                            "owner",
                            "/first",
                            &mut &b"first value"[..],
                            false,
                            false,
                            &mut vault,
                            &master,
                        )?;
                    }
                    _ => {
                        session.mkdir_kv("owner", "/first", false, &mut vault, &master)?;
                    }
                }
                assert_eq!(root_count(), 1);
                assert_eq!(session.list_kv("owner", &mut vault)?.entries.len(), 1);
                assert_eq!(
                    session.list_kv_metadata("owner", &mut vault)?.entries.len(),
                    1
                );
                if first_write < 2 {
                    assert_eq!(
                        session.read_kv_file("owner", "/first", &mut vault)?,
                        b"first value"
                    );
                }
                // A namespace already observed by this client cannot silently
                // turn into a new, empty namespace after server-side data loss.
                assert_eq!(db.execute("DELETE FROM kv_roots", []).unwrap(), 1);
                assert!(session.sync_account("owner", &mut vault).is_err());
                assert!(session
                    .put_kv_file(
                        "owner",
                        "/replacement",
                        &mut &b"no"[..],
                        false,
                        false,
                        &mut vault,
                        &master
                    )
                    .is_err());
                assert_eq!(root_count(), 0);
            }
            assert_eq!(
                session
                    .verify_passphrase(
                        "owner",
                        Passphrase::new("paired-account passphrase")?,
                        &mut vault,
                    )?
                    .generation,
                1
            );
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    drop(provisioner_registry);
}

fn machine(
    environment: &TestEnvironment,
    state: &Path,
    port: u16,
) -> (ProfileRegistry, ProfileSession, ClientCredentials) {
    let root = PathBuf::from(state).join("probe-root.der");
    std::fs::create_dir_all(state).unwrap();
    environment.write_probe_root(&root).unwrap();
    ClientCredentials::initialize(state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(state).unwrap();
    registry
        .add(Profile {
            name: "local".to_owned(),
            probe: format!("localhost:{port}"),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let session = ProfileSession::open(&registry, "local").unwrap();
    let credentials = ClientCredentials::open(state).unwrap();
    credentials
        .with_checked_session(&session, |session| {
            session.probe_and_pin()?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    (registry, session, credentials)
}
