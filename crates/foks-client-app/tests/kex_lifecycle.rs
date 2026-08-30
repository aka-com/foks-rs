use std::path::{Path, PathBuf};

use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, KexAcceptanceInput,
    Passphrase, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::{EncryptedFileSecretStore, SecretStore as _};
use foks_server_testkit::TestEnvironment;

#[test]
fn protected_product_workflow_pairs_two_machine_state_roots() {
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
