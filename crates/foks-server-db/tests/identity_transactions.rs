mod common;

use foks_server_db::{CommitOutcome, FailurePoint};

#[test]
fn every_injected_stage_rolls_back_all_authoritative_state() {
    for point in [
        FailurePoint::Invite,
        FailurePoint::Name,
        FailurePoint::User,
        FailurePoint::Device,
        FailurePoint::Chain,
        FailurePoint::MerkleNodes,
        FailurePoint::MerkleRoot,
        FailurePoint::Receipt,
    ] {
        let mut database = common::TestDatabase::new();
        database.reserve(1_000_000);
        assert!(database.commit(Some(point)).is_err(), "{point:?}");
        assert!(database.database.identity(&[1; 33]).unwrap().is_none());
        assert!(database.database.current_root().unwrap().is_none());
        assert!(matches!(
            database.commit(None).unwrap(),
            CommitOutcome::Committed(_)
        ));
    }
}

#[test]
fn complete_identity_and_root_are_visible_together() {
    let mut database = common::TestDatabase::new();
    database.reserve(1_000_000);
    database.commit(None).unwrap();
    let identity = database.database.identity(&[1; 33]).unwrap().unwrap();
    assert_eq!(identity.exact_link, b"exact-link");
    assert_eq!(identity.exact_parcel, b"parcel");
    assert_eq!(identity.username_utf8, b"Fixture User");
    assert_eq!(identity.username_sequence, 1);
    assert_eq!(identity.exact_device_name, b"device-name");
    let reader =
        foks_server_db::ReadDatabase::open(&database.path, foks_server_db::Config::default())
            .unwrap();
    assert!(reader
        .identity_for_active_device(&[1; 33], &[5; 33])
        .unwrap()
        .is_none());
    assert!(reader
        .identity_for_active_device(&[1; 33], &[4; 33])
        .unwrap()
        .is_some());
    assert!(reader
        .identity_by_active_device(&[5; 33])
        .unwrap()
        .is_none());
    let root = database.database.current_root().unwrap().unwrap();
    assert_eq!(root.epoch, 1);
    assert_eq!(root.exact_signed_root, b"signed-root");
}
