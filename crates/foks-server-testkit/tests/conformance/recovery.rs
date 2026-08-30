use foks_client::{
    NewSoftwareDeviceSecrets, NoPassphraseConfigured, SoftwareDeviceProvisionRequest,
    UserPukRotation,
};
use foks_crypto::{derive_device_public, BackupKey};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient};

use crate::support::Fixture;

#[test]
pub(crate) fn recovery_success() {
    let fixture = Fixture::start("recovery-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("recoveryuser", 0x21))
        .unwrap();
    let seed = [0x01; foks_crypto::BACKUP_SEED_BYTES];
    let backup = BackupKey::from_seed(seed).unwrap();
    let enrolled = fixture
        .client
        .foks()
        .enroll_backup_key(fixture.host(), &created.credential, Role::OWNER, &backup)
        .unwrap();
    assert_eq!(enrolled.authenticated.verified.chain_seqno(), 2);

    // Re-authenticating the just-enrolled backup on the existing client can
    // produce a different, equally valid user-evidence path for the same
    // Merkle head. Durable state must compare the authenticated root, not the
    // particular proof serialization chosen for that authentication.
    let same_client_backup = fixture
        .client
        .foks()
        .load_backup_key(fixture.host(), BackupKey::from_seed(seed).unwrap())
        .unwrap();
    let same_client_authenticated = fixture
        .client
        .foks()
        .authenticate_backup_and_pin(fixture.host(), &same_client_backup)
        .unwrap();
    assert_eq!(same_client_authenticated.verified.chain_seqno(), 2);

    // Recovery normally begins on a fresh installation. Give it an
    // independent durable trust store so the test also proves reconstruction
    // from the host pin and backup phrase alone.
    let recovery_client = TestClient::new(&fixture.environment, "recovery-fresh-client").unwrap();
    let recovery_probe = recovery_client.probe_and_pin().unwrap();
    let located = recovery_client
        .foks()
        .load_backup_key(&recovery_probe.pinned, BackupKey::from_seed(seed).unwrap())
        .unwrap();
    let recovered = recovery_client
        .foks()
        .recover_software_device(
            &recovery_probe.pinned,
            located,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: "recovered device".to_owned(),
                serial: 2,
            },
            NewSoftwareDeviceSecrets::new(SecretSeed::new([0x31; 32]), None, [0x32; 17]),
        )
        .unwrap();
    assert_eq!(recovered.authenticated.verified.chain_seqno(), 3);

    // The recovery credential can recover user material but route policy does
    // not permit it to be used as an ordinary personal-KV principal. The
    // durable replacement is immediately usable as an ordinary device.
    let refreshed = recovery_client
        .foks()
        .authenticate_and_pin(&recovery_probe.pinned, &recovered.credential)
        .unwrap();
    assert_eq!(refreshed.verified.chain_seqno(), 3);

    // Recovery credentials remain active recipients after provisioning. A
    // subsequent revoke must rotate the PUK to both the recovered software
    // device and the enrolled backup key, exactly as Go's box gameplan does.
    let original = derive_device_public(&SecretSeed::new([0x21; 32])).unwrap();
    let mut protected = fixture.client.open_protected_store().unwrap();
    let revoked = fixture
        .client
        .foks()
        .revoke_user_credential_with_software_device(
            fixture.host(),
            &recovered.credential,
            &original.id,
            &[UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::new([0x22; 32]),
                new_seed: SecretSeed::new([0x51; 32]),
            }],
            Some(NoPassphraseConfigured),
            &mut protected,
        )
        .unwrap();
    assert_eq!(revoked.verified.chain_seqno(), 4);
    assert_eq!(
        revoked.verified.shared_key(Role::OWNER).unwrap().generation,
        2
    );
    assert!(revoked
        .verified
        .devices()
        .iter()
        .any(|device| device.id.entity_type() == foks_proto::ENTITY_BACKUP_KEY));
}
