use foks_client::{NewSoftwareDeviceSecrets, SoftwareDeviceProvisionRequest};
use foks_proto::{
    ClientVersionExt, DeviceNagInfo, PermissionToken, Role, SecretSeed, SemVer, ViewershipMode,
};
use foks_server_db::InviteRegime;
use foks_server_testkit::TestAccountSpec;

use crate::support::Fixture;

#[test]
pub(crate) fn go_client_activation_success() {
    let fixture = Fixture::start("go-client-activation");
    let config = fixture
        .client
        .foks()
        .registration_server_config(fixture.host())
        .unwrap();
    assert_eq!(config.sso, None);
    assert_eq!(config.host_type, 4);
    assert_eq!(config.user_viewership, ViewershipMode::Open);
    assert_eq!(config.team_viewership, ViewershipMode::Open);
    assert_eq!(config.invite_code_regime, 2);

    fixture
        .environment
        .set_invite_regime(InviteRegime::Required)
        .unwrap();
    let required = fixture
        .client
        .foks()
        .registration_server_config(fixture.host())
        .unwrap();
    assert_eq!(required.invite_code_regime, 1);
    fixture
        .environment
        .set_invite_regime(InviteRegime::Optional)
        .unwrap();

    let version = fixture
        .client
        .foks()
        .client_version_info(
            fixture.host(),
            &ClientVersionExt {
                version: SemVer {
                    major: 0,
                    minor: 1,
                    patch: 9,
                },
                linker_version: b"go1.25".to_vec(),
                linker_packaging: b"test".to_vec(),
            },
        )
        .unwrap();
    assert_eq!(version.minimum, None);
    assert_eq!(version.newest, None);
    assert!(version.message.is_empty());

    assert!(matches!(
        fixture
            .client
            .foks()
            .check_name_exists(fixture.host(), "available-name"),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1027,
            ..
        }))
    ));

    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("goactivation", 0x31))
        .unwrap();
    fixture
        .client
        .foks()
        .check_name_exists(fixture.host(), "GoActivation")
        .unwrap();
    assert_eq!(
        fixture
            .client
            .foks()
            .resolve_username(fixture.host(), &created.credential, "GoActivation", false,)
            .unwrap(),
        created.credential.uid
    );
    assert_eq!(
        fixture
            .client
            .foks()
            .resolve_username(fixture.host(), &created.credential, "GoActivation", true,)
            .unwrap(),
        created.credential.uid
    );
    let first_device = foks_crypto::derive_device_public(&created.credential.seed).unwrap();
    fixture
        .client
        .foks()
        .probe_key_exists(
            fixture.host(),
            &created.credential.uid,
            &first_device.id,
            &PermissionToken::new([0x33; 17]),
        )
        .unwrap();
    assert!(matches!(
        fixture.client.foks().probe_key_exists(
            fixture.host(),
            &created.credential.uid,
            &first_device.id,
            &PermissionToken::new([0x34; 17]),
        ),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1025,
            ..
        }))
    ));
    assert_eq!(
        fixture
            .client
            .foks()
            .ping(fixture.host(), &created.credential)
            .unwrap(),
        created.credential.uid
    );
    assert_eq!(
        fixture
            .client
            .foks()
            .device_nag(fixture.host(), &created.credential)
            .unwrap(),
        DeviceNagInfo {
            num_devices: 1,
            cleared: false,
        }
    );
    fixture
        .client
        .foks()
        .clear_device_nag(fixture.host(), &created.credential, true)
        .unwrap();
    assert_eq!(
        fixture
            .client
            .foks()
            .device_nag(fixture.host(), &created.credential)
            .unwrap(),
        DeviceNagInfo {
            num_devices: 1,
            cleared: true,
        }
    );
    let mut protected = fixture.client.open_protected_store().unwrap();
    let provisioned = fixture
        .client
        .foks()
        .provision_software_device(
            fixture.host(),
            &created.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: "second housekeeping device".to_owned(),
                serial: 2,
            },
            NewSoftwareDeviceSecrets::new(SecretSeed::new([0x41; 32]), None, [0x42; 17]),
            &mut protected,
        )
        .unwrap();
    let second_device = foks_crypto::derive_device_public(&provisioned.credential.seed).unwrap();
    fixture
        .client
        .foks()
        .probe_key_exists(
            fixture.host(),
            &provisioned.credential.uid,
            &second_device.id,
            &PermissionToken::new([0x42; 17]),
        )
        .unwrap();
    assert_eq!(
        fixture
            .client
            .foks()
            .device_nag(fixture.host(), &provisioned.credential)
            .unwrap(),
        DeviceNagInfo {
            num_devices: 2,
            cleared: true,
        }
    );
    fixture
        .client
        .foks()
        .clear_device_nag(fixture.host(), &provisioned.credential, false)
        .unwrap();
    assert_eq!(
        fixture
            .client
            .foks()
            .device_nag(fixture.host(), &provisioned.credential)
            .unwrap(),
        DeviceNagInfo {
            num_devices: 2,
            cleared: false,
        }
    );
}
