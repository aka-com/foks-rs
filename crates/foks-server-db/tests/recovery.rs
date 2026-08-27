mod common;

use foks_server_db::{Config, Error};

#[test]
fn recovery_challenges_are_bounded_expiring_and_one_time() {
    let mut test = common::TestDatabase::with_config(Config {
        maximum_active_recovery_challenges: 2,
        maximum_recovery_challenges_per_entity: 1,
        ..Config::default()
    });
    let entity_a = [16; 33];
    let entity_b = {
        let mut entity = [16; 33];
        entity[32] = 17;
        entity
    };
    let host = [2; 33];
    let generation = [3; 16];
    test.database
        .issue_recovery_challenge(&[4; 32], &entity_a, &host, &generation, 20, 10)
        .unwrap();
    assert!(matches!(
        test.database
            .issue_recovery_challenge(&[5; 32], &entity_a, &host, &generation, 20, 10),
        Err(Error::QuotaExceeded)
    ));
    test.database
        .issue_recovery_challenge(&[6; 32], &entity_b, &host, &generation, 20, 10)
        .unwrap();

    // An unknown credential does not disclose whether the challenge existed,
    // but the one-time challenge is still consumed.
    assert!(test
        .database
        .consume_recovery_challenge(&[4; 32], &entity_a, &host, &generation, 11)
        .unwrap()
        .is_none());
    assert!(test
        .database
        .consume_recovery_challenge(&[4; 32], &entity_a, &host, &generation, 11)
        .unwrap()
        .is_none());
    assert!(test
        .database
        .consume_recovery_challenge(&[6; 32], &entity_b, &host, &generation, 20)
        .unwrap()
        .is_none());
    assert_eq!(test.database.run_maintenance(20, 0).unwrap().challenges, 2);
}
