use foks_client::{DeviceCredential, NoPassphraseConfigured, Passphrase, UserPukRotation};
use foks_proto::{GenericLinkPayload, Role, SecretSeed, CHAIN_TYPE_USER_SETTINGS};
use foks_server_testkit::{TestAccountSpec, TestClient};

use super::support::Fixture;

#[test]
pub(crate) fn signup_set_change_and_public_login_cover_the_passphrase_lifecycle() {
    let fixture = Fixture::start("passphrase-client");

    let signup = fixture
        .client
        .create_account_with_passphrase(
            fixture.host(),
            &TestAccountSpec::new("signupphrase", 0x91),
            "signup passphrase one",
        )
        .unwrap();
    assert_eq!(
        fixture
            .client
            .foks()
            .verify_passphrase(
                fixture.host(),
                &signup.credential,
                &Passphrase::new("signup passphrase one").unwrap(),
            )
            .unwrap()
            .generation,
        1
    );
    let later = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("laterphrase", 0xa1))
        .unwrap();
    let first = Passphrase::new("configured after signup").unwrap();
    assert_eq!(
        fixture
            .client
            .foks()
            .set_passphrase(fixture.host(), &later.credential, &first)
            .unwrap()
            .generation,
        1
    );
    let settings = fixture
        .client
        .foks()
        .load_generic_chain(
            fixture.host(),
            &later.credential,
            CHAIN_TYPE_USER_SETTINGS,
            1,
        )
        .unwrap();
    assert_eq!(settings.links.len(), 1);
    assert!(matches!(
        settings.links[0].decode_generic().unwrap().payload,
        GenericLinkPayload::UserSettings(ref info) if info.generation == 1
    ));
    let secondary = TestClient::new(&fixture.environment, "passphrase-secondary").unwrap();
    let secondary_host = secondary.probe_and_pin().unwrap().pinned;
    let secondary_credential = DeviceCredential {
        uid: later.credential.uid.clone(),
        seed: SecretSeed::new([0xa1; 32]),
        certificate_chain: later.credential.certificate_chain.clone(),
    };
    assert_eq!(
        secondary
            .foks()
            .verify_passphrase(&secondary_host, &secondary_credential, &first)
            .unwrap()
            .generation,
        1
    );
    let second = Passphrase::new("rotated passphrase two").unwrap();
    assert_eq!(
        fixture
            .client
            .foks()
            .change_passphrase(fixture.host(), &later.credential, &second)
            .unwrap()
            .generation,
        2
    );
    let secondary_user = secondary
        .foks()
        .authenticate_and_pin(&secondary_host, &secondary_credential)
        .unwrap();
    assert!(
        !secondary
            .foks()
            .refresh_passphrase_for_current_puk(
                &secondary_host,
                &secondary_credential,
                &secondary_user,
            )
            .unwrap()
    );
    let settings = fixture
        .client
        .foks()
        .load_generic_chain(
            fixture.host(),
            &later.credential,
            CHAIN_TYPE_USER_SETTINGS,
            1,
        )
        .unwrap();
    assert_eq!(settings.links.len(), 2);
    assert!(matches!(
        settings.links[1].decode_generic().unwrap().payload,
        GenericLinkPayload::UserSettings(ref info) if info.generation == 2
    ));
    assert_eq!(
        fixture
            .client
            .foks()
            .passphrase_salt(fixture.host(), &later.credential)
            .unwrap(),
        fixture
            .client
            .foks()
            .passphrase_metadata(fixture.host(), &later.credential)
            .unwrap()
            .salt
    );
    assert!(fixture
        .client
        .foks()
        .verify_passphrase(fixture.host(), &later.credential, &first)
        .is_err());
    assert_eq!(
        fixture
            .client
            .foks()
            .verify_passphrase(fixture.host(), &later.credential, &second)
            .unwrap()
            .generation,
        2
    );

    let mut protected = fixture.client.open_protected_store().unwrap();
    assert!(fixture
        .client
        .foks()
        .rotate_software_puks(
            fixture.host(),
            &later.credential,
            &[UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::new([0xa2; 32]),
                new_seed: SecretSeed::new([0xb2; 32]),
            }],
            Some(NoPassphraseConfigured),
            &mut protected,
        )
        .is_err());
    let rotated = fixture
        .client
        .foks()
        .rotate_software_puks(
            fixture.host(),
            &later.credential,
            &[UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::new([0xa2; 32]),
                new_seed: SecretSeed::new([0xb2; 32]),
            }],
            None,
            &mut protected,
        )
        .unwrap();
    assert_eq!(
        rotated.verified.shared_key(Role::OWNER).unwrap().generation,
        2
    );
    assert_eq!(
        fixture
            .client
            .foks()
            .verify_passphrase(fixture.host(), &later.credential, &second)
            .unwrap()
            .generation,
        3
    );
    let settings = fixture
        .client
        .foks()
        .load_generic_chain(
            fixture.host(),
            &later.credential,
            CHAIN_TYPE_USER_SETTINGS,
            1,
        )
        .unwrap();
    assert_eq!(settings.links.len(), 3);
    assert!(matches!(
        settings.links[2].decode_generic().unwrap().payload,
        GenericLinkPayload::UserSettings(ref info) if info.generation == 3
    ));
}
