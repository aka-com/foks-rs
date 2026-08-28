use foks_client::{NoPassphraseConfigured, Passphrase, UserPukRotation};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::TestAccountSpec;

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
}
