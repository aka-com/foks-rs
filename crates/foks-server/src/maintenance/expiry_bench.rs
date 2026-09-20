//! Opt-in expiry workload using the production server writer and cleanup APIs.
//! Copy this unchanged into a pre-change checkout for a source-level baseline.
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use serde_json::{json, Value};

const NOW: u64 = 48 * 60 * 60 * 1_000_000;
const FUTURE: u64 = NOW + 24 * 60 * 60 * 1_000_000;

fn nanos(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn quantiles(mut samples: Vec<u64>) -> Value {
    samples.sort_unstable();
    let percentile = |p: usize| samples[(samples.len() * p).div_ceil(100) - 1];
    json!({"count": samples.len(), "p50": percentile(50), "p95": percentile(95),
        "p99": percentile(99), "max": samples.last()})
}

fn seed(path: &Path, live_rows: usize) {
    drop(foks_server_db::Database::open(path, Default::default()).unwrap());
    let mut connection = rusqlite::Connection::open(path).unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "foreign_keys", |row| row.get(0))
            .unwrap(),
        1
    );
    let transaction = connection.transaction().unwrap();
    for index in 0..live_rows {
        let mut token = [0u8; 17];
        token[..8].copy_from_slice(&(index as u64).to_be_bytes());
        let name = format!("live-{index}");
        for table in ["names", "team_names"] {
            transaction.execute(
                &format!("INSERT INTO {table}(normalized_name,reservation_token,reservation_sequence,expires_at) VALUES(?1,?2,1,?3)"),
                rusqlite::params![name.as_bytes(), token.as_slice(), FUTURE as i64],
            ).unwrap();
        }
        let mut hash = [0u8; 32];
        hash[..8].copy_from_slice(&(index as u64).to_be_bytes());
        transaction.execute(
            "INSERT INTO recovery_challenges(challenge_hash,entity_id,host_id,key_generation,expires_at,consumed) VALUES(?1,zeroblob(33),zeroblob(33),zeroblob(16),?2,0)",
            rusqlite::params![hash.as_slice(), FUTURE as i64],
        ).unwrap();
    }
    transaction.commit().unwrap();
    assert!(connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}

#[test]
#[ignore = "opt-in concurrent foreground/expiry workload; prints JSON observations"]
fn concurrent_expiry_and_foreground_writes() {
    for live_rows in [2_048, 10_000] {
        for repetition in 0..3 {
            let temporary = tempfile::tempdir().unwrap();
            let path = temporary.path().join("expiry.sqlite");
            seed(&path, live_rows);
            let writer = crate::Writer::start(path, Default::default(), 16).unwrap();
            writer
                .call(|db| {
                    db.checkpoint_passive()?;
                    Ok(())
                })
                .unwrap();
            let next = AtomicU64::new(0);
            let gate = Arc::new(Barrier::new(5));
            let started = Instant::now();
            let mut cleanup = Vec::new();
            let mut checkpoints = Vec::new();
            let mut maintenance_queue = Vec::new();
            let samples = std::thread::scope(|scope| {
                let writers: Vec<_> = (0..4)
                    .map(|_| {
                        let gate = Arc::clone(&gate);
                        let writer = &writer;
                        let next = &next;
                        scope.spawn(move || {
                            gate.wait();
                            (0..128)
                                .map(|_| {
                                    let id = next.fetch_add(1, Ordering::Relaxed);
                                    let queued = Instant::now();
                                    let wait = writer
                                        .call(move |db| {
                                            let wait = nanos(queued);
                                            let mut token = [255u8; 17];
                                            token[..8].copy_from_slice(&id.to_be_bytes());
                                            db.reserve_name(
                                                format!("foreground-{id}").as_bytes(),
                                                &token,
                                                1,
                                                NOW,
                                                FUTURE,
                                            )?;
                                            Ok(wait)
                                        })
                                        .unwrap();
                                    (wait, nanos(queued))
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                gate.wait();
                for _ in 0..32 {
                    let queued = Instant::now();
                    let (wait, cleanup_ns, checkpoint_ns, checkpoint) = writer
                        .call(move |db| {
                            let wait = nanos(queued);
                            let started = Instant::now();
                            let report =
                                db.run_maintenance(NOW, NOW - super::ABANDONED_UPLOAD_AGE_MICROS)?;
                            let cleanup_ns = nanos(started);
                            assert_eq!(report, foks_server_db::MaintenanceReport::default());
                            let started = Instant::now();
                            let checkpoint = db.checkpoint_passive()?;
                            Ok((wait, cleanup_ns, nanos(started), checkpoint))
                        })
                        .unwrap();
                    maintenance_queue.push(wait);
                    cleanup.push(cleanup_ns);
                    checkpoints.push(json!({"duration_ns": checkpoint_ns, "busy": checkpoint.busy,
                        "wal_pages": checkpoint.wal_pages, "checkpointed_pages": checkpoint.checkpointed_pages}));
                }
                writers
                    .into_iter()
                    .flat_map(|thread| thread.join().unwrap())
                    .collect::<Vec<_>>()
            });
            println!(
                "EXPIRY_WRITER_BENCH {}",
                json!({
                    "live_rows_per_populated_table": live_rows, "tables": ["names", "team_names", "recovery_challenges"],
                    "synthetic_above_admission_caps": live_rows > 4_096,
                    "repetition": repetition, "sqlite": rusqlite::version(),
                    "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                    "elapsed_ns": nanos(started), "foreground_queue_ns": quantiles(samples.iter().map(|sample| sample.0).collect()),
                    "foreground_completion_ns": quantiles(samples.iter().map(|sample| sample.1).collect()),
                    "maintenance_queue_ns": quantiles(maintenance_queue), "cleanup_transaction_ns": quantiles(cleanup),
                    "cleanup_includes": "all expiry statements and empty upload reclamation; no upload or log payloads",
                    "checkpoints": checkpoints,
                })
            );
            writer.shutdown().unwrap();
        }
    }
}
