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
        FailurePoint::IdempotencyRecord,
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
fn device_subkey_ids_are_unique_across_users() {
    let mut database = common::TestDatabase::new();
    database.reserve(1_000_000);
    database.commit(None).unwrap();
    let connection = rusqlite::Connection::open(&database.path).unwrap();
    const SUBKEY: [u8; 33] = [0x31; 33];
    connection
        .execute(
            "UPDATE devices SET subkey_id = ?1 WHERE device_id = ?2",
            rusqlite::params![SUBKEY, [4u8; 33]],
        )
        .unwrap();

    const OTHER_UID: [u8; 33] = [0x21; 33];
    connection
        .execute(
            "INSERT INTO names
             (normalized_name, reservation_token, reservation_sequence, expires_at, uid)
             VALUES (?1, NULL, 1, NULL, ?2)",
            rusqlite::params![b"other-user", OTHER_UID],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO users
             (uid, normalized_name, username_utf8, username_sequence,
              username_commitment_key, created_at)
             VALUES (?1, ?2, ?3, 1, ?4, 1)",
            rusqlite::params![OTHER_UID, b"other-user", b"Other User", [0x22u8; 16]],
        )
        .unwrap();
    let error = connection
        .execute(
            "INSERT INTO devices
             (device_id, uid, active, role_type, visibility, subkey_id,
              hepk_fingerprint, self_token, exact_hepk, exact_name)
             VALUES (?1, ?2, 1, 3, 0, ?3, ?4, zeroblob(17), ?5, ?6)",
            rusqlite::params![
                [0x23u8; 33],
                OTHER_UID,
                SUBKEY,
                [0x24u8; 32],
                b"other-hepk",
                b"other-device"
            ],
        )
        .unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation)
    );
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
              hepk_fingerprint, self_token, exact_hepk, exact_name)
             VALUES (?1, ?2, 1, 1, -7, NULL, ?3, zeroblob(17), ?4, ?5)",
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
