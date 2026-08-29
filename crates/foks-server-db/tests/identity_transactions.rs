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

#[test]
fn active_credentials_resolve_their_own_current_role_parcel() {
    let mut database = common::TestDatabase::new();
    database.reserve(1_000_000);
    database.commit(None).unwrap();
    let connection = rusqlite::Connection::open(&database.path).unwrap();

    connection
        .execute(
            "INSERT INTO shared_keys
             (uid, role_type, visibility, generation, verify_key, exact_hepk)
             VALUES (?1, 3, 0, 2, ?2, ?3)",
            rusqlite::params![[1u8; 33], [0x71u8; 33], b"owner-hepk-generation-two"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO parcels
             (uid, device_id, sender_id, role_type, visibility, generation, exact_parcel)
             VALUES (?1, ?2, ?2, 3, 0, 2, ?3)",
            rusqlite::params![[1u8; 33], [4u8; 33], b"owner-parcel-generation-two"],
        )
        .unwrap();

    const MEMBER: [u8; 33] = [5; 33];
    connection
        .execute(
            "INSERT INTO devices
             (device_id, uid, active, role_type, visibility, subkey_id,
              hepk_fingerprint, exact_hepk, exact_name)
             VALUES (?1, ?2, 1, 1, -7, NULL, ?3, ?4, ?5)",
            rusqlite::params![MEMBER, [1u8; 33], [0x72u8; 32], b"member-hepk", b"member"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO shared_keys
             (uid, role_type, visibility, generation, verify_key, exact_hepk)
             VALUES (?1, 1, -7, 1, ?2, ?3)",
            rusqlite::params![[1u8; 33], [0x73u8; 33], b"member-puk-hepk"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO parcels
             (uid, device_id, sender_id, role_type, visibility, generation, exact_parcel)
             VALUES (?1, ?2, ?3, 1, -7, 1, ?4)",
            rusqlite::params![[1u8; 33], MEMBER, [4u8; 33], b"member-current-role-parcel"],
        )
        .unwrap();
    drop(connection);

    let reader =
        foks_server_db::ReadDatabase::open(&database.path, foks_server_db::Config::default())
            .unwrap();
    let default = reader.identity(&[1; 33]).unwrap().unwrap();
    assert_eq!(default.device_id, [4u8; 33]);
    assert_eq!(default.exact_shared_hepk, b"owner-hepk-generation-two");
    let owner = reader
        .identity_for_active_device(&[1; 33], &[4; 33])
        .unwrap()
        .unwrap();
    assert_eq!(owner.exact_shared_hepk, b"owner-hepk-generation-two");
    assert_eq!(owner.exact_parcel, b"owner-parcel-generation-two");

    let member = reader
        .identity_for_active_device(&[1; 33], &MEMBER)
        .unwrap()
        .unwrap();
    assert_eq!(member.exact_shared_hepk, b"member-puk-hepk");
    assert_eq!(member.exact_parcel, b"member-current-role-parcel");
}
