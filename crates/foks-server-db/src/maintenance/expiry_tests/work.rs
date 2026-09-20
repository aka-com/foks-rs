use rusqlite::{Connection, StatementStatus};

use super::{expiry, fixture::*};

#[derive(Clone, Copy, Debug)]
pub struct Work {
    pub rows: usize,
    pub vm: i32,
    pub scans: i32,
    pub sorts: i32,
    pub auto_indexes: i32,
}

pub fn measure(connection: &Connection, sql: &str, cutoff: u64, delete: bool) -> Work {
    let mut statement = connection.prepare(sql).unwrap();
    measure_prepared(&mut statement, cutoff, delete)
}

fn measure_prepared(statement: &mut rusqlite::Statement<'_>, cutoff: u64, delete: bool) -> Work {
    for status in [
        StatementStatus::VmStep,
        StatementStatus::FullscanStep,
        StatementStatus::Sort,
        StatementStatus::AutoIndex,
    ] {
        statement.reset_status(status);
    }
    let rows = if delete {
        statement.execute([super::fixture::sql(cutoff)]).unwrap()
    } else {
        let mut rows = statement.query([super::fixture::sql(cutoff)]).unwrap();
        let mut count = 0;
        while rows.next().unwrap().is_some() {
            count += 1;
        }
        count
    };
    Work {
        rows,
        vm: statement.get_status(StatementStatus::VmStep),
        scans: statement.get_status(StatementStatus::FullscanStep),
        sorts: statement.get_status(StatementStatus::Sort),
        auto_indexes: statement.get_status(StatementStatus::AutoIndex),
    }
}

fn plan(connection: &Connection, sql: &str, cutoff: u64) -> Vec<String> {
    connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map([super::fixture::sql(cutoff)], |r| r.get(3))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

pub fn statistics(f: &Fixture, mode: &str) {
    match mode {
        "none" => {}
        "fresh" => f.db.connection.execute_batch("ANALYZE").unwrap(),
        "stale" => {
            // Adversarial consumed/claimed distributions, then reuse those stale
            // statistics for a live unconsumed/reserved population.
            f.db.connection.execute_batch(
                "UPDATE recovery_challenges SET consumed=1;
                 UPDATE names SET uid=CAST(reservation_token||zeroblob(16) AS BLOB),reservation_token=NULL,expires_at=NULL WHERE uid IS NULL;
                 UPDATE team_names SET team_id=CAST(reservation_token||zeroblob(16) AS BLOB),reservation_token=NULL,expires_at=NULL WHERE team_id IS NULL;
                 ANALYZE;",
            ).unwrap();
            f.db.connection
                .execute("UPDATE recovery_challenges SET consumed=0", [])
                .unwrap();
            f.db.connection.execute("UPDATE names SET reservation_token=substr(uid,1,17),uid=NULL,expires_at=?1 WHERE normalized_name<>x'706172656e74'",[super::fixture::sql(NOW+1)]).unwrap();
            f.db.connection.execute("UPDATE team_names SET reservation_token=substr(team_id,1,17),team_id=NULL,expires_at=?1",[super::fixture::sql(NOW+1)]).unwrap();
        }
        _ => panic!("unknown statistics mode"),
    }
}

#[test]
fn indexed_noops_have_bounded_work_with_fresh_absent_and_stale_statistics() {
    for live in [256, 2048] {
        for mode in ["none", "fresh", "stale"] {
            let mut f = Fixture::new();
            for statement in expiry::ALL {
                f.seed(statement.table, 0, live, cutoff(statement.table) + 1);
            }
            statistics(&f, mode);
            for statement in expiry::ALL {
                for (sql, delete, budget) in [
                    (statement.select, false, 500),
                    (statement.delete, true, 2500),
                ] {
                    let work = measure(&f.db.connection, sql, cutoff(statement.table), delete);
                    assert_eq!(work.rows, 0, "{} {mode} {live}", statement.table);
                    assert_eq!(
                        (work.scans, work.sorts, work.auto_indexes),
                        (0, 0, 0),
                        "{} {work:?} {:?}",
                        statement.table,
                        plan(&f.db.connection, sql, cutoff(statement.table))
                    );
                    assert!(
                        work.vm < budget,
                        "{} {mode} {live}: {work:?} {:?}",
                        statement.table,
                        plan(&f.db.connection, sql, cutoff(statement.table))
                    );
                }
            }
            f.integrity();
        }
    }
}

#[test]
fn eligible_tail_and_fixed_backlog_work_do_not_scale_with_unrelated_live_rows() {
    for eligible in [3, 300] {
        let mut previous = Vec::new();
        for live in [256, 2048] {
            let mut f = Fixture::new();
            for statement in expiry::ALL {
                f.seed(statement.table, 0, live, cutoff(statement.table) + 1);
            }
            statistics(&f, "stale");
            for statement in expiry::ALL {
                f.seed(statement.table, 10_000, eligible, cutoff(statement.table));
            }
            for (i, statement) in expiry::ALL.into_iter().enumerate() {
                let mut measurements = Vec::new();
                for (sql, delete) in [(statement.select, false), (statement.delete, true)] {
                    f.db.connection
                        .execute_batch("SAVEPOINT measure_expiry")
                        .unwrap();
                    let work = measure(&f.db.connection, sql, cutoff(statement.table), delete);
                    f.db.connection
                        .execute_batch("ROLLBACK TO measure_expiry; RELEASE measure_expiry")
                        .unwrap();
                    assert_eq!(work.rows, eligible.min(128), "{} {work:?}", statement.table);
                    assert_eq!(
                        (work.sorts, work.auto_indexes),
                        (0, 0),
                        "{} {work:?} {:?}",
                        statement.table,
                        plan(&f.db.connection, sql, cutoff(statement.table))
                    );
                    assert!(work.vm < 100_000, "{} {work:?}", statement.table);
                    measurements.push(work.vm);
                }
                if live == 256 {
                    previous.push(measurements);
                } else {
                    for (small, large) in previous[i].iter().zip(measurements) {
                        assert!(
                            large <= small * 2 + 500,
                            "{} grew with unrelated live rows: {small} -> {large}",
                            statement.table
                        );
                    }
                }
                assert_eq!(f.count(statement.table), live + eligible);
            }
            f.integrity();
        }
    }
}

#[test]
fn prepared_selectors_and_deletes_reuse_changed_cutoffs_and_statistics() {
    let mut f = Fixture::new();
    for statement in expiry::ALL {
        let cutoff = cutoff(statement.table);
        f.seed(statement.table, 0, 3, cutoff + 1);
        for (sql, delete) in [(statement.select, false), (statement.delete, true)] {
            let mut prepared = f.db.connection.prepare(sql).unwrap();
            assert_eq!(measure_prepared(&mut prepared, cutoff, delete).rows, 0);
            f.db.connection
                .execute_batch("ANALYZE; SAVEPOINT reused_expiry")
                .unwrap();
            assert_eq!(measure_prepared(&mut prepared, cutoff + 1, delete).rows, 3);
            f.db.connection
                .execute_batch("ROLLBACK TO reused_expiry; RELEASE reused_expiry")
                .unwrap();
            assert_eq!(measure_prepared(&mut prepared, cutoff, delete).rows, 0);
        }
    }
}

#[test]
fn mostly_claimed_names_do_not_add_noop_work() {
    let mut f = Fixture::new();
    f.seed("names", 0, 2048, NOW + 1);
    f.db.connection.execute("UPDATE names SET uid=?1,dead=1,reservation_token=NULL,expires_at=NULL WHERE rowid>1 AND rowid<=2000",[USER]).unwrap();
    f.db.connection.execute_batch("ANALYZE").unwrap();
    for sql in [expiry::RESERVATIONS.select, expiry::RESERVATIONS.delete] {
        let work = measure(&f.db.connection, sql, NOW, sql.starts_with("DELETE"));
        assert_eq!(work.rows, 0);
        assert!(work.vm < 500, "{work:?}");
        assert_eq!((work.scans, work.sorts, work.auto_indexes), (0, 0, 0));
    }
    f.integrity();
}
