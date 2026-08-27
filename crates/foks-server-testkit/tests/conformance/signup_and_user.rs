use foks_server_testkit::TestAccountSpec;

use crate::support::Fixture;

#[test]
pub(crate) fn signup_and_user_success() {
    let fixture = Fixture::start("signup-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("signupuser", 0x71))
        .unwrap();
    assert_eq!(created.kv_projection.len(), 1);
    assert!(created.kv_projection[0].entries.is_empty());
    assert_eq!(created.authenticated.verified.chain_seqno(), 1);
    assert_eq!(created.authenticated.verified.username(), b"signupuser");
    assert_eq!(created.authenticated.puks.len(), 1);
    assert_eq!(created.authenticated.puks[0].seed.as_slice(), &[0x72; 32]);

    let authenticated = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &created.credential)
        .unwrap();
    assert_eq!(authenticated.verified, created.authenticated.verified);
    assert_eq!(authenticated.puks[0].seed.as_slice(), &[0x72; 32]);
}
