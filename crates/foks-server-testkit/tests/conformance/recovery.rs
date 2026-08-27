use foks_client::{NewSoftwareDeviceSecrets, SoftwareDeviceProvisionRequest};
use foks_crypto::BackupKey;
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
}
