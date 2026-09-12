use crate::support::Fixture;
use foks_client::{FederationCredential, SoftwareKeyKind};
use foks_client_db::MutationState;
use foks_proto::Role;
use foks_server_testkit::TestAccountSpec;

#[test]
fn bot_enrollment_and_host_bound_load_use_distinct_credential_identity() {
    let f = Fixture::start("bot-client");
    let created = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("botowner", 0x49))
        .unwrap();
    let token = foks_crypto::BotToken::generate().unwrap();
    let mut protected = f.client.open_protected_store().unwrap();
    let owner = FederationCredential::Software(&created.credential);
    let op = f
        .client
        .foks()
        .prepare_bot_enrollment(f.host(), owner, Role::OWNER, &token, &mut protected)
        .unwrap();
    assert_eq!(op.state, MutationState::Prepared);
    let op = f
        .client
        .foks()
        .bot_enrollment_progress(f.host(), owner, op.operation_id, true, &mut protected)
        .unwrap();
    assert_eq!(op.state, MutationState::RemoteVerified);
    assert_eq!(op.attempt_count, 1);
    let bot = f.client.foks().load_bot_token(f.host(), &token).unwrap();
    assert_eq!(bot.key_kind, SoftwareKeyKind::BotToken);
    assert_eq!(bot.uid, created.credential.uid);
    assert_eq!(f.client.foks().ping(f.host(), &bot).unwrap(), bot.uid);
    let loaded = f
        .client
        .foks()
        .authenticate_and_pin(f.host(), &bot)
        .unwrap();
    assert_eq!(loaded.puks[0].role, Role::OWNER);
    let permanent = f
        .client
        .foks()
        .provision_software_device(
            f.host(),
            &bot,
            foks_client::SoftwareDeviceProvisionRequest {
                device_name: "permanent".into(),
                role: Role::OWNER,
                serial: 1,
            },
            foks_client::NewSoftwareDeviceSecrets::new(
                foks_proto::SecretSeed::new([0x77; 32]),
                None,
                [0x36; 17],
            ),
            &mut protected,
        )
        .unwrap();
    assert_eq!(permanent.credential.key_kind, SoftwareKeyKind::Device);
    let low = foks_crypto::BotToken::generate().unwrap();
    let low_op = f
        .client
        .foks()
        .prepare_bot_enrollment(f.host(), owner, Role::member(0), &low, &mut protected)
        .unwrap();
    assert_eq!(
        f.client
            .foks()
            .bot_enrollment_progress(f.host(), owner, low_op.operation_id, true, &mut protected)
            .unwrap()
            .state,
        MutationState::RemoteVerified
    );
    let low = f.client.foks().load_bot_token(f.host(), &low).unwrap();
    let auth = f
        .client
        .foks()
        .authenticate_and_pin(f.host(), &low)
        .unwrap();
    assert_eq!(auth.puks.last().unwrap().role, Role::member(0));
    assert!(!auth.puks.iter().any(|p| p.role == Role::OWNER));
    assert!(f
        .client
        .foks()
        .provision_software_device(
            f.host(),
            &low,
            foks_client::SoftwareDeviceProvisionRequest {
                device_name: "escalation".into(),
                role: Role::OWNER,
                serial: 1
            },
            foks_client::NewSoftwareDeviceSecrets::new(
                foks_proto::SecretSeed::new([0x78; 32]),
                None,
                [0x37; 17]
            ),
            &mut protected
        )
        .is_err());
    let another = Fixture::start("other-bot-host");
    assert!(another
        .client
        .foks()
        .load_bot_token(another.host(), &token)
        .is_err());
}
