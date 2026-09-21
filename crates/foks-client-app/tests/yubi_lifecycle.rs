use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, Passphrase, Profile,
    ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot, YubiProvisionInput,
    YubiSignupInput,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;
use foks_yubi::{MockYubiProvider, Pin, PinRetryConfiguration, SlotId, YubiProvider as _};

#[test]
fn product_vault_covers_yubikey_provisioning_recovery_administration_and_revocation() {
    let environment = TestEnvironment::new().unwrap();
    let _server = environment.start_server().unwrap();
    let state = environment.client_path("yubi-product", "state").unwrap();
    let root = environment
        .client_path("yubi-product", "probe-root.der")
        .unwrap();
    environment.write_probe_root(&root).unwrap();
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let addresses = environment.addresses().unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".to_owned(),
            label: None,
            probe: format!("localhost:{}", addresses.probe.port()),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let session = ProfileSession::open(&registry, "local").unwrap();
    let credentials = ClientCredentials::open(&state).unwrap();
    credentials
        .with_checked_session(&session, |session| {
            session.probe_and_pin()?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    let pin = Pin::new("123456").unwrap();
    let provider = MockYubiProvider::with_card("product-yubikey", 72001, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let signup_provider =
        MockYubiProvider::with_card("signup-product-yubikey", 72002, &pin).unwrap();
    let signup_card = signup_provider.cards().unwrap().remove(0);
    credentials
        .with_checked_session(&session, |session| {
            let master = credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "software",
                "productuser",
                "software owner",
                "product@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            let signup = session.create_yubi_account(
                YubiSignupInput {
                    alias: "hardware-signup".to_owned(),
                    username: "productyubi".to_owned(),
                    device_name: "Yubi-only owner".to_owned(),
                    email: "product-yubi@example.test".to_owned(),
                    invite: String::new(),
                    passphrase: Some(Passphrase::new("hardware-only recovery phrase")?),
                    card: signup_card,
                    signing_slot: SlotId::new(0x82)?,
                    pq_slot: SlotId::new(0x83)?,
                    retry_configuration: None,
                },
                Pin::new("123456")?,
                &signup_provider,
                &mut vault,
                &master,
            )?;
            assert!(signup.management_enrolled);
            let scheduler_now = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("test clock is after the Unix epoch")
                    .as_micros(),
            )
            .expect("test clock fits in u64")
            .saturating_add(20 * 60 * 1_000_000);
            let scheduled = session.run_due_jobs(scheduler_now, &mut vault, &master)?;
            assert_eq!(scheduled.runs.len(), 4);
            assert!(
                scheduled.runs.iter().all(|run| run.completed),
                "hardware-only refresh jobs must defer without failure: {:?}",
                scheduled.runs
            );
            session.sync_yubi_account(
                "hardware-signup",
                Pin::new("123456")?,
                &signup_provider,
                &mut vault,
                &master,
            )?;
            let changed = Passphrase::new("hardware-only rotated recovery phrase")?;
            let report = session.change_yubi_passphrase(
                "hardware-signup",
                Pin::new("123456")?,
                changed,
                &signup_provider,
                &mut vault,
                &master,
            )?;
            assert_eq!(report.generation, 2);
            assert!(report.verified);
            let verified = session.verify_yubi_passphrase(
                "hardware-signup",
                Pin::new("123456")?,
                Passphrase::new("hardware-only rotated recovery phrase")?,
                &signup_provider,
                &mut vault,
            )?;
            assert_eq!(verified.generation, 2);
            let resumed_signup = session.resume_yubi_account(
                "hardware-signup",
                Pin::new("123456")?,
                &signup_provider,
                &mut vault,
                &master,
            )?;
            assert_eq!(resumed_signup.user_chain_sequence, 1);
            let provisioned = session.provision_yubi_device(
                YubiProvisionInput {
                    source_alias: "software".to_owned(),
                    target_alias: "hardware".to_owned(),
                    device_name: "primary YubiKey".to_owned(),
                    serial: 2,
                    card,
                    signing_slot: SlotId::new(0x82)?,
                    pq_slot: SlotId::new(0x83)?,
                    retry_configuration: Some(PinRetryConfiguration::new(
                        Pin::new("12345678")?,
                        5,
                        4,
                    )?),
                },
                Pin::new("123456")?,
                &provider,
                &mut vault,
                &master,
            )?;
            assert!(provisioned.management_enrolled);
            let resumed_provision = session.resume_yubi_account(
                "hardware",
                Pin::new("123456")?,
                &provider,
                &mut vault,
                &master,
            )?;
            assert_eq!(resumed_provision.user_chain_sequence, 2);
            assert_eq!(
                vault.yubi_aliases()?,
                vec!["hardware".to_owned(), "hardware-signup".to_owned()]
            );
            assert_eq!(
                session
                    .yubi_pin_status("hardware", &provider, &mut vault)?
                    .remaining,
                5
            );
            session.sync_yubi_account(
                "hardware",
                Pin::new("123456")?,
                &provider,
                &mut vault,
                &master,
            )?;
            let recovered = session.recover_yubi_subkey(
                "hardware",
                Pin::new("123456")?,
                &provider,
                &mut vault,
                &master,
            )?;
            assert!(!recovered.subkey_id_hex.is_empty());
            assert!(recovered.certificate_count >= 1);
            assert!(
                session
                    .rotate_yubi_management_key(
                        "hardware",
                        Pin::new("123456")?,
                        &provider,
                        &mut vault,
                        &master,
                    )?
                    .management_enrolled
            );
            assert_eq!(
                session
                    .change_yubi_pin(
                        "hardware",
                        Pin::new("123456")?,
                        Pin::new("234567")?,
                        &provider,
                        &mut vault,
                    )?
                    .remaining,
                5
            );
            session.change_yubi_puk(
                "hardware",
                Pin::new("12345678")?,
                Pin::new("87654321")?,
                &provider,
                &mut vault,
            )?;
            session.sync_yubi_account(
                "hardware",
                Pin::new("234567")?,
                &provider,
                &mut vault,
                &master,
            )?;
            let locator = vault.yubi_account("hardware")?.locator;
            for _ in 0..5 {
                assert!(provider.open(&locator, Some(&Pin::new("999999")?)).is_err());
            }
            assert!(
                session
                    .yubi_pin_status("hardware", &provider, &mut vault)?
                    .blocked
            );
            assert_eq!(
                session
                    .unblock_yubi_pin(
                        "hardware",
                        Pin::new("87654321")?,
                        Pin::new("345678")?,
                        &provider,
                        &mut vault,
                    )?
                    .remaining,
                5
            );
            session.sync_yubi_account(
                "hardware",
                Pin::new("345678")?,
                &provider,
                &mut vault,
                &master,
            )?;
            assert!(
                session
                    .recover_yubi_management_key("hardware", "software", &mut vault)?
                    .management_enrolled
            );
            let revoked =
                session.revoke_yubi_device("software", "hardware", &mut vault, &master)?;
            assert_eq!(revoked.user_chain_sequence, 3);
            assert!(revoked.removed_local_credential);
            assert!(!vault.contains("hardware")?);
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
}
