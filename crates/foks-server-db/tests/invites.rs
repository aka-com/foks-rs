mod common;

use foks_server_db::{
    Error, FailurePoint, InviteConsumption, InviteKind, InviteRegime, ReadDatabase,
};

#[test]
fn invite_policy_availability_and_lifecycle_are_explicit() {
    let mut fixture = common::TestDatabase::new();
    assert_eq!(
        fixture.database.invite_policy().unwrap().regime,
        InviteRegime::Optional
    );
    fixture
        .database
        .set_invite_regime(InviteRegime::Required)
        .unwrap();
    assert_eq!(
        ReadDatabase::open(&fixture.path, foks_server_db::Config::default())
            .unwrap()
            .invite_policy()
            .unwrap()
            .regime,
        InviteRegime::Required
    );

    let hash = [0x71; 32];
    fixture
        .database
        .issue_invite(
            &[0x72; 16],
            &hash,
            InviteKind::MultiUse,
            None,
            Some(2),
            Some(1_000_100),
            1_000_000,
        )
        .unwrap();
    let reader = ReadDatabase::open(&fixture.path, foks_server_db::Config::default()).unwrap();
    assert!(reader
        .invite_available(&hash, InviteKind::MultiUse, 1_000_099)
        .unwrap());
    assert!(!reader
        .invite_available(&hash, InviteKind::MultiUse, 1_000_100)
        .unwrap());
    assert!(fixture.database.disable_invite(&hash, 1_000_050).unwrap());
    assert!(!fixture.database.disable_invite(&hash, 1_000_051).unwrap());
    let snapshot = fixture.database.invites().unwrap().pop().unwrap();
    assert!(!snapshot.active);
    assert_eq!(snapshot.disabled_at, Some(1_000_050));
}

#[test]
fn invite_consumption_rolls_back_with_the_identity_transaction() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture
        .database
        .set_invite_regime(InviteRegime::Required)
        .unwrap();
    let hash = [0x81; 32];
    fixture
        .database
        .issue_invite(
            &[0x82; 16],
            &hash,
            InviteKind::Standard,
            None,
            Some(1),
            None,
            999_999,
        )
        .unwrap();

    common::commit_with_request_at_and_invite(
        &mut fixture.database,
        Some(FailurePoint::User),
        [0x83; 32],
        1_000_000,
        InviteConsumption::Code {
            code_hash: &hash,
            kind: InviteKind::Standard,
        },
    )
    .unwrap_err();
    assert_eq!(fixture.database.invites().unwrap()[0].use_count, 0);

    common::commit_with_request_at_and_invite(
        &mut fixture.database,
        None,
        [0x83; 32],
        1_000_000,
        InviteConsumption::Code {
            code_hash: &hash,
            kind: InviteKind::Standard,
        },
    )
    .unwrap();
    assert_eq!(fixture.database.invites().unwrap()[0].use_count, 1);
    assert!(
        !ReadDatabase::open(&fixture.path, foks_server_db::Config::default())
            .unwrap()
            .invite_available(&hash, InviteKind::Standard, 1_000_001)
            .unwrap()
    );
}

#[test]
fn required_policy_rejects_empty_signup_without_claiming_the_reservation() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture
        .database
        .set_invite_regime(InviteRegime::Required)
        .unwrap();
    assert!(matches!(fixture.commit(None), Err(Error::BadInvite)));
    fixture
        .database
        .set_invite_regime(InviteRegime::Optional)
        .unwrap();
    fixture.commit(None).unwrap();
}
