use foks_client::{
    NewSoftwareDeviceSecrets, NoPassphraseConfigured, SoftwareDeviceProvisionRequest,
    UserPukRotation,
};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::TestAccountSpec;

use crate::support::Fixture;

#[test]
pub(crate) fn provisioning_success() {
    let fixture = Fixture::start("provisioning-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("provisionuser", 0x31))
        .unwrap();
    let mut protected = fixture.client.open_protected_store().unwrap();
    let provisioned = fixture
        .client
        .foks()
        .provision_software_device(
            fixture.host(),
            &created.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: "second device".to_owned(),
                serial: 2,
            },
            NewSoftwareDeviceSecrets::new(SecretSeed::new([0x41; 32]), None, [0x42; 17]),
            &mut protected,
        )
        .unwrap();
    assert_eq!(provisioned.authenticated.verified.chain_seqno(), 2);
    assert_eq!(provisioned.authenticated.verified.devices().len(), 2);

    let original = foks_crypto::derive_device_public(&SecretSeed::new([0x31; 32])).unwrap();
    let revoked = fixture
        .client
        .foks()
        .revoke_user_credential_with_software_device(
            fixture.host(),
            &provisioned.credential,
            &original.id,
            &[UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::new([0x32; 32]),
                new_seed: SecretSeed::new([0x51; 32]),
            }],
            Some(NoPassphraseConfigured),
            &mut protected,
        )
        .unwrap();
    assert_eq!(revoked.verified.chain_seqno(), 3);
    assert_eq!(revoked.verified.devices().len(), 1);
    assert_eq!(
        revoked.verified.shared_key(Role::OWNER).unwrap().generation,
        2
    );
}
