mod common;

use foks_server_db::Error;

#[test]
fn reservations_are_unique_expiring_and_bounded() {
    let mut database = common::TestDatabase::new();
    database
        .database
        .reserve_name(b"name", &[1; 32], 1, 100, 200)
        .unwrap();
    assert!(matches!(
        database
            .database
            .reserve_name(b"name", &[2; 32], 2, 150, 300),
        Err(Error::NameInUse)
    ));
    database
        .database
        .reserve_name(b"name", &[2; 32], 2, 200, 300)
        .unwrap();
    assert_eq!(
        database
            .database
            .cleanup_expired_reservations(300, 1)
            .unwrap(),
        1
    );
}

#[test]
fn expired_reservation_is_rejected_without_cleanup() {
    let mut database = common::TestDatabase::new();
    database
        .database
        .reserve_name(b"fixtureuser", &[0x44; 32], 1, 0, 10)
        .unwrap();
    assert!(matches!(database.commit(None), Err(Error::Reservation)));
    assert!(database.database.identity(&[1; 33]).unwrap().is_none());
}
