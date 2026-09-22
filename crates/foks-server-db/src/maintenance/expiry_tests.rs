//! Exercise production selectors and DELETEs with foreign keys enabled.
mod benchmark;
mod fixture;
mod work;

use fixture::*;
use rusqlite::params;

use super::expiry;

#[test]
fn every_expiry_kind_preserves_future_rows_and_drains_one_shared_budget() {
    for eligible in [0, 1, 127, 128, 129, 300] {
        let mut f = Fixture::new();
        for statement in expiry::ALL {
            f.seed(statement.table, 0, eligible, cutoff(statement.table) - 1);
            // Equality belongs to the eligible population when there is one.
            if eligible > 0 {
                f.set_expiry(statement.table, 0, cutoff(statement.table));
            }
            f.seed(statement.table, 10_000, 1, cutoff(statement.table) + 1);
        }
        f.integrity();
        let mut remaining = eligible;
        loop {
            let report = f.db.run_maintenance(NOW, 0).unwrap();
            let expected = remaining.min(128) as u64;
            for (table, deleted) in counts(report) {
                assert_eq!(deleted, expected, "{table}: eligible={eligible}");
                assert_eq!(f.count(table), remaining - expected as usize + 1);
            }
            remaining -= expected as usize;
            if expected == 0 {
                break;
            }
        }
        f.integrity();
    }
}

#[test]
fn recovery_disjoint_branches_cover_all_states_without_double_counting_or_sorting() {
    let mut f = Fixture::new();
    for (start, consumed, expires) in [
        (0, 0, NOW - 1),
        (1000, 1, NOW - 1),
        (2000, 1, NOW + 1),
        (3000, 0, NOW + 1),
    ] {
        f.seed("recovery_challenges", start, 150, expires);
        f.db.connection
            .execute(
                "UPDATE recovery_challenges SET consumed=?1 WHERE rowid>?2 AND rowid<=?3",
                params![
                    consumed,
                    sql((start / 1000) * 150),
                    sql((start / 1000 + 1) * 150)
                ],
            )
            .unwrap();
    }
    let work = work::measure(&f.db.connection, expiry::CHALLENGES.select, NOW, false);
    assert_eq!(work.rows, 128);
    assert_eq!((work.sorts, work.auto_indexes), (0, 0));
    assert!(work.vm < 20_000, "{work:?}");
    let selected =
        f.db.connection
            .prepare(expiry::CHALLENGES.select)
            .unwrap()
            .query_map([sql(NOW)], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<std::collections::HashSet<_>, _>>()
            .unwrap();
    assert_eq!(selected.len(), 128);
    for expected in [128, 128, 128, 66, 0] {
        assert_eq!(f.db.run_maintenance(NOW, 0).unwrap().challenges, expected);
    }
    assert_eq!(f.count("recovery_challenges"), 150);
    let survivors: i64 =
        f.db.connection
            .query_row(
                "SELECT count(*) FROM recovery_challenges WHERE consumed=0 AND expires_at>?1",
                [sql(NOW)],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(survivors, 150);
}

#[test]
fn claimed_and_historical_names_and_consumed_view_retry_records_survive() {
    let mut f = Fixture::new();
    f.seed("names", 0, 1, NOW - 1);
    f.seed("team_names", 0, 1, NOW - 1);
    f.db.connection
        .execute(
            "INSERT INTO names VALUES(x'686973746f7279',NULL,1,NULL,1,?1)",
            [USER],
        )
        .unwrap();
    f.db.connection
        .execute(
            "INSERT INTO team_names VALUES(x'636c61696d6564',NULL,1,NULL,?1)",
            [TEAM],
        )
        .unwrap();
    f.seed("team_view_challenges", 0, 1, NOW + 1);
    f.db.connection
        .execute(
            "UPDATE team_view_challenges SET consumed=1,activation_hash=zeroblob(32)",
            [],
        )
        .unwrap();
    let report = f.db.run_maintenance(NOW, 0).unwrap();
    assert_eq!(report.reservations, 1);
    assert_eq!(report.team_reservations, 1);
    assert_eq!(report.team_view_challenges, 0);
    assert_eq!(f.count("names"), 1); // Dead history; fixture count excludes current parent.
    assert_eq!(f.count("team_names"), 1);
    assert_eq!(
        f.db.run_maintenance(NOW + 1, 0)
            .unwrap()
            .team_view_challenges,
        1
    );
    f.integrity();
}

#[test]
fn sso_disabled_policy_states_millisecond_boundary_and_linkage_are_preserved() {
    let mut f = Fixture::new();
    assert_eq!(f.db.run_maintenance(NOW, 0).unwrap().sso_sessions, 0);
    f.db.connection
        .execute("DELETE FROM sso_policy", [])
        .unwrap();
    assert_eq!(f.db.run_maintenance(NOW, 0).unwrap().sso_sessions, 0);
    assert!(f.db.connection.execute(
        "INSERT INTO sso_sessions VALUES(?1,zeroblob(32),zeroblob(32),zeroblob(32),NULL,0,1,1,0,1,zeroblob(57))", [HOST],
    ).is_err());
    let work = work::measure(
        &f.db.connection,
        expiry::SSO_SESSIONS.select,
        NOW / 1000,
        false,
    );
    assert_eq!(work.rows, 0);
    assert_eq!((work.scans, work.sorts, work.auto_indexes), (0, 0, 0));
    assert!(work.vm < 500, "{work:?}");
    f.policy();
    f.db.connection
        .execute("UPDATE sso_policy SET blocked_reason=1", [])
        .unwrap();
    f.seed("sso_sessions", 0, 8, NOW / 1000);
    for state in 0..8 {
        f.db.connection
            .execute(
                "UPDATE sso_sessions SET state=?1 WHERE session_hash=?2",
                params![state, id::<32>(state as u64)],
            )
            .unwrap();
    }
    f.db.connection.execute(
        "INSERT INTO sso_access VALUES(?1,?2,'https://issuer.test','subject',zeroblob(32),1,1,1,0,0,1,zeroblob(57))", params![HOST,USER],
    ).unwrap();
    assert_eq!(f.db.run_maintenance(NOW - 1, 0).unwrap().sso_sessions, 0);
    assert_eq!(f.db.run_maintenance(NOW, 0).unwrap().sso_sessions, 8);
    assert_eq!(f.count("sso_access"), 1);
    f.seed("sso_sessions", 10, 1, NOW / 1000 + 1);
    assert_eq!(f.db.run_maintenance(NOW + 999, 0).unwrap().sso_sessions, 0);
    assert_eq!(f.db.run_maintenance(NOW + 1000, 0).unwrap().sso_sessions, 1);
    // Active policies use the same expiry rules and retain identity linkage.
    f.db.connection
        .execute("UPDATE sso_policy SET mode=1,blocked_reason=NULL", [])
        .unwrap();
    f.seed("sso_sessions", 11, 1, NOW / 1000 + 1);
    assert_eq!(f.db.run_maintenance(NOW + 1001, 0).unwrap().sso_sessions, 1);
    assert_eq!(f.count("sso_access"), 1);
    f.integrity();
}

#[test]
fn late_delete_failure_rolls_back_every_earlier_expiry_kind() {
    let mut f = Fixture::new();
    for statement in expiry::ALL {
        f.seed(statement.table, 0, 1, cutoff(statement.table));
    }
    f.db.connection.execute_batch(
        "CREATE TRIGGER fail_expiry BEFORE DELETE ON log_sends BEGIN SELECT RAISE(ABORT,'injected late failure'); END;",
    ).unwrap();
    assert!(f.db.run_maintenance(NOW, 0).is_err());
    for statement in expiry::ALL {
        assert_eq!(f.count(statement.table), 1, "{}", statement.table);
    }
    f.db.connection
        .execute_batch("DROP TRIGGER fail_expiry")
        .unwrap();
    for (_, count) in counts(f.db.run_maintenance(NOW, 0).unwrap()) {
        assert_eq!(count, 1);
    }
    f.integrity();
}

#[test]
fn saturated_log_cutoff_preserves_foreground_boundary_and_cascades_small_payloads() {
    let mut f = Fixture::new();
    f.seed("log_sends", 0, 1, 0);
    f.seed("log_sends", 1, 1, 1);
    f.db.connection
        .execute(
            "INSERT INTO log_send_files VALUES(?1,0,'small.log',3,1,zeroblob(32),0)",
            [id::<17>(0)],
        )
        .unwrap();
    f.db.connection
        .execute(
            "INSERT INTO log_send_blocks VALUES(?1,0,0,x'616263',0)",
            [id::<17>(0)],
        )
        .unwrap();
    // Foreground reclamation is strict (<), while maintenance retains <=.
    let mut foreground_id = id::<17>(2);
    foreground_id[0] = 48;
    f.db.begin_log_send(&foreground_id, None, 1).unwrap();
    assert_eq!(f.count("log_sends"), 3);
    assert_eq!(f.db.run_maintenance(1, 0).unwrap().log_sends, 1);
    assert_eq!(f.count("log_send_files"), 0);
    assert_eq!(f.count("log_send_blocks"), 0);
    assert_eq!(f.count("log_sends"), 2);
    f.integrity();
}
