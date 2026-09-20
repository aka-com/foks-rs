//! Opt-in comparisons; no wall-clock assertions belong in normal CI.
use std::time::Instant;

use super::{
    expiry,
    fixture::*,
    work::{measure, Work},
};

const DROP_INDEXES: &str = "
DROP INDEX names_reservation_expiry;
DROP INDEX team_names_reservation_expiry;
DROP INDEX recovery_challenges_cleanup;
DROP INDEX team_view_tokens_expiry;
DROP INDEX team_view_challenges_expiry;
DROP INDEX team_admin_tokens_expiry;
DROP INDEX log_sends_created_at;
VACUUM;";

fn baseline_select(statement: expiry::ExpiryStatement) -> &'static str {
    match statement.table {
        "recovery_challenges" => {
            "SELECT rowid FROM recovery_challenges WHERE expires_at <= ?1 OR consumed = 1 LIMIT 128"
        }
        "sso_sessions" => "SELECT rowid FROM sso_sessions WHERE expires_at_ms <= ?1 LIMIT 128",
        _ => statement.select,
    }
}

fn size(table: &str, synthetic: bool) -> usize {
    if synthetic {
        return 10_000;
    }
    match table {
        "sso_sessions" => 1000,
        "log_sends" => 128,
        "team_view_tokens" | "team_view_challenges" | "team_admin_tokens" => 16_384,
        // Names and receipts have no matching global admission cap; 4096 is a
        // representative comparison size, not a claim about their limits.
        _ => 4096,
    }
}

fn emit_work(
    mode: &str,
    scale: &str,
    table: &str,
    phase: &str,
    live: usize,
    work: Work,
    elapsed_ns: u128,
) {
    println!("{{\"benchmark\":\"expiry\",\"mode\":\"{mode}\",\"scale\":\"{scale}\",\"table\":\"{table}\",\"phase\":\"{phase}\",\"live_rows\":{live},\"affected_or_selected\":{},\"vm_steps\":{},\"fullscan_steps\":{},\"sorts\":{},\"auto_indexes\":{},\"elapsed_ns\":{elapsed_ns}}}",work.rows,work.vm,work.scans,work.sorts,work.auto_indexes);
}

#[test]
#[ignore = "release-mode expiry selection, write-cost and allocation comparison"]
fn expiry_scale_and_write_cost() {
    for synthetic in [false, true] {
        let scale = if synthetic {
            "synthetic10000"
        } else {
            "representative_caps"
        };
        for baseline in [true, false] {
            let mode = if baseline { "baseline" } else { "indexed" };
            let mut f = Fixture::new();
            if baseline {
                f.db.connection.execute_batch(DROP_INDEXES).unwrap();
            }
            f.db.checkpoint().unwrap();
            let initial = f.db.storage_report().unwrap();
            for statement in expiry::ALL {
                let count = size(statement.table, synthetic);
                let start = Instant::now();
                f.seed(statement.table, 0, count, cutoff(statement.table) + 1);
                println!("{{\"benchmark\":\"expiry\",\"mode\":\"{mode}\",\"scale\":\"{scale}\",\"table\":\"{}\",\"phase\":\"batched_insert\",\"rows\":{count},\"elapsed_ns\":{}}}",statement.table,start.elapsed().as_nanos());
            }
            f.integrity();
            let allocated = f.db.storage_report().unwrap();
            f.db.checkpoint().unwrap();
            let checkpointed = f.db.storage_report().unwrap();
            println!("{{\"benchmark\":\"expiry\",\"mode\":\"{mode}\",\"scale\":\"{scale}\",\"phase\":\"allocation\",\"initial_database_bytes\":{},\"database_bytes_before_checkpoint\":{},\"wal_bytes_before_checkpoint\":{},\"database_bytes_after_checkpoint\":{}}}",initial.database_bytes,allocated.database_bytes,allocated.wal_bytes,checkpointed.database_bytes);
            for eligible in [0, 3, 300] {
                for statement in expiry::ALL {
                    if eligible > 0 {
                        f.seed(statement.table, 20_000, eligible, cutoff(statement.table));
                    }
                    let select = if baseline {
                        baseline_select(statement)
                    } else {
                        statement.select
                    };
                    let delete = if baseline {
                        format!("DELETE FROM {} WHERE rowid IN ({select})", statement.table)
                    } else {
                        statement.delete.to_owned()
                    };
                    for (sql, is_delete) in [(select, false), (delete.as_str(), true)] {
                        f.db.connection
                            .execute_batch("SAVEPOINT bench_expiry")
                            .unwrap();
                        let mut elapsed = Vec::new();
                        let mut last = None;
                        // Roll back each sample so candidate populations agree.
                        for _ in 0..5 {
                            let start = Instant::now();
                            let work =
                                measure(&f.db.connection, sql, cutoff(statement.table), is_delete);
                            elapsed.push(start.elapsed().as_nanos());
                            assert_eq!(work.rows, eligible.min(128));
                            last = Some(work);
                            f.db.connection
                                .execute_batch("ROLLBACK TO bench_expiry")
                                .unwrap();
                        }
                        f.db.connection
                            .execute_batch("RELEASE bench_expiry")
                            .unwrap();
                        elapsed.sort_unstable();
                        let phase = format!(
                            "{}_eligible{eligible}",
                            if is_delete { "delete" } else { "select" }
                        );
                        emit_work(
                            mode,
                            scale,
                            statement.table,
                            &phase,
                            size(statement.table, synthetic),
                            last.unwrap(),
                            elapsed[2],
                        );
                    }
                    if eligible > 0 {
                        f.db.connection
                            .execute(
                                &format!(
                                    "DELETE FROM {} WHERE {}<=?1",
                                    statement.table,
                                    expiry_column(statement.table)
                                ),
                                [super::fixture::sql(cutoff(statement.table))],
                            )
                            .unwrap();
                    }
                }
            }
            // The composite recovery index is maintained when a challenge is
            // consumed. Record that write cost, outside insertion and cleanup.
            let mut elapsed = Vec::new();
            for _ in 0..5 {
                f.db.connection
                    .execute_batch("SAVEPOINT consume_bench")
                    .unwrap();
                let start = Instant::now();
                let rows =
                    f.db.connection
                        .execute(
                            "UPDATE recovery_challenges SET consumed=1 WHERE rowid<=128",
                            [],
                        )
                        .unwrap();
                elapsed.push(start.elapsed().as_nanos());
                assert_eq!(rows, 128);
                f.db.connection
                    .execute_batch("ROLLBACK TO consume_bench; RELEASE consume_bench")
                    .unwrap();
            }
            elapsed.sort_unstable();
            println!("{{\"benchmark\":\"expiry\",\"mode\":\"{mode}\",\"scale\":\"{scale}\",\"table\":\"recovery_challenges\",\"phase\":\"consume128\",\"rows\":128,\"elapsed_ns\":{}}}",elapsed[2]);
            // Complete foreground reclamation keeps its unbounded semantics.
            for eligible in [0, 300] {
                if eligible > 0 {
                    f.seed("recovery_challenges", 20_000, eligible, NOW);
                }
                f.db.connection
                    .execute_batch("SAVEPOINT foreground_bench")
                    .unwrap();
                let start = Instant::now();
                let (rows, vm) = if baseline {
                    let work = measure(
                        &f.db.connection,
                        "DELETE FROM recovery_challenges WHERE expires_at<=?1 OR consumed=1",
                        NOW,
                        true,
                    );
                    (work.rows, work.vm)
                } else {
                    let mut consumed =
                        f.db.connection
                            .prepare(crate::recovery::RECLAIM_CONSUMED)
                            .unwrap();
                    let rows = consumed.execute([]).unwrap();
                    let vm = consumed.get_status(rusqlite::StatementStatus::VmStep);
                    let expired = measure(
                        &f.db.connection,
                        crate::recovery::RECLAIM_EXPIRED,
                        NOW,
                        true,
                    );
                    (rows + expired.rows, vm + expired.vm)
                };
                let elapsed = start.elapsed().as_nanos();
                assert_eq!(rows, eligible);
                println!("{{\"benchmark\":\"expiry\",\"mode\":\"{mode}\",\"scale\":\"{scale}\",\"table\":\"recovery_challenges\",\"phase\":\"foreground_reclaim\",\"rows\":{rows},\"vm_steps\":{vm},\"elapsed_ns\":{elapsed}}}");
                f.db.connection
                    .execute_batch("ROLLBACK TO foreground_bench; RELEASE foreground_bench")
                    .unwrap();
            }
            f.integrity();
        }
    }
}
