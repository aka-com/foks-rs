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

fn seed_reclamation_fixture(path: &std::path::Path) {
    let mut connection = rusqlite::Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let tx = connection.transaction().unwrap();
    for id in 0_u64..602 {
        let mut hash = [0; 32];
        hash[..8].copy_from_slice(&id.to_be_bytes());
        let mut entity = [1; 33];
        entity[..8].copy_from_slice(&id.to_be_bytes());
        let (expires, consumed) = if id < 300 {
            (10, 0)
        } else if id < 600 {
            (100, 1)
        } else {
            (100, 0)
        };
        tx.execute(
            "INSERT INTO recovery_challenges VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![hash, entity, [2_u8; 33], [3_u8; 16], expires, consumed],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn foreground_recovery_reclaims_the_entire_eligible_backlog_before_quotas() {
    let mut test = common::TestDatabase::with_config(Config {
        maximum_active_recovery_challenges: 3,
        maximum_recovery_challenges_per_entity: 1,
        ..Default::default()
    });
    seed_reclamation_fixture(&test.path);
    test.database
        .issue_recovery_challenge(&[7; 32], &[8; 33], &[2; 33], &[3; 16], 100, 10)
        .unwrap();
    let connection = rusqlite::Connection::open(&test.path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM recovery_challenges", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM recovery_challenges WHERE consumed=1 OR expires_at<=10",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert!(matches!(
        test.database
            .issue_recovery_challenge(&[9; 32], &[9; 33], &[2; 33], &[3; 16], 100, 10),
        Err(Error::QuotaExceeded)
    ));
}

#[test]
fn second_recovery_delete_failure_rolls_back_consumed_reclamation() {
    let mut test = common::TestDatabase::new();
    seed_reclamation_fixture(&test.path);
    let connection = rusqlite::Connection::open(&test.path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_expired BEFORE DELETE ON recovery_challenges
         WHEN OLD.consumed=0 BEGIN SELECT RAISE(ABORT,'injected second delete failure'); END;",
        )
        .unwrap();
    assert!(test
        .database
        .issue_recovery_challenge(&[7; 32], &[8; 33], &[2; 33], &[3; 16], 100, 10)
        .is_err());
    let counts: (i64, i64) = connection
        .query_row(
            "SELECT count(*),sum(consumed) FROM recovery_challenges",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(counts, (602, 300));
    connection
        .execute_batch("DROP TRIGGER fail_expired")
        .unwrap();
    test.database
        .issue_recovery_challenge(&[7; 32], &[8; 33], &[2; 33], &[3; 16], 100, 10)
        .unwrap();
}

#[test]
fn invalid_recovery_timestamp_does_not_reclaim_any_rows() {
    let mut test = common::TestDatabase::new();
    seed_reclamation_fixture(&test.path);
    assert!(matches!(
        test.database.issue_recovery_challenge(
            &[7; 32],
            &[8; 33],
            &[2; 33],
            &[3; 16],
            u64::MAX,
            i64::MAX as u64 + 1
        ),
        Err(Error::IntegerRange)
    ));
    let connection = rusqlite::Connection::open(&test.path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM recovery_challenges", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        602
    );
}
