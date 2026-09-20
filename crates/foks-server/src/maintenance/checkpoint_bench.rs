//! Opt-in mechanism/workload comparison on disposable databases, not a capacity claim.
use super::*;
use foks_server_db::{
    CheckpointReport, Config, Database, LogSendBlockMutation, LogSendFileMutation,
};
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Barrier;

type Checkpoint = fn(&Database) -> foks_server_db::Result<CheckpointReport>;

struct StopReaders<'a>(&'a AtomicBool);
impl Drop for StopReaders<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

struct Clock;
impl foks_server_db::Clock for Clock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        Ok(1_000_000)
    }
}

fn quantiles(mut values: Vec<u64>) -> serde_json::Value {
    values.sort_unstable();
    let percentile = |percent: usize| values[(values.len() - 1) * percent / 100];
    json!({"p50": percentile(50), "p95": percentile(95), "p99": percentile(99), "max": values.last()})
}

fn wal_bytes(path: &Path) -> u64 {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    std::fs::metadata(wal).map(|m| m.len()).unwrap_or(0)
}

fn exercise(
    label: &str,
    variant: &str,
    writer: &crate::Writer,
    path: &Path,
    metrics: &Arc<ServerMetrics>,
    next: &AtomicU64,
    checkpoint: Checkpoint,
) {
    let start = Instant::now();
    let gate = Arc::new(Barrier::new(5));
    let mut checkpoint_samples = Vec::new();
    let samples = std::thread::scope(|scope| {
        let writers: Vec<_> = (0..4)
            .map(|_| {
                let gate = Arc::clone(&gate);
                scope.spawn(move || {
                    gate.wait();
                    (0..64)
                        .map(|_| {
                            let id = next.fetch_add(1, Ordering::Relaxed);
                            let queued = Instant::now();
                            let wait = writer
                                .call(move |db| {
                                    let wait = queued.elapsed().as_micros() as u64;
                                    let mut token = [1; 17];
                                    token[1..9].copy_from_slice(&id.to_be_bytes());
                                    db.reserve_name(
                                        format!("bench-{id}").as_bytes(),
                                        &token,
                                        1,
                                        1,
                                        10_000_000,
                                    )?;
                                    Ok(wait)
                                })
                                .unwrap();
                            (wait, queued.elapsed().as_micros() as u64)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        gate.wait();
        for _ in 0..8 {
            let before = metrics.snapshot().checkpoint.duration_microseconds_total;
            let (_, report) = run_with_checkpoint(
                &writer.handle(),
                Arc::new(Clock),
                Arc::clone(metrics),
                None,
                checkpoint,
            )
            .unwrap();
            checkpoint_samples.push(json!({
                "at_us": start.elapsed().as_micros(),
                "duration_us": metrics.snapshot().checkpoint.duration_microseconds_total - before,
                "busy": report.busy, "wal_frames": report.wal_pages,
                "backfilled_frames": report.checkpointed_pages,
                "outcome": format!("{:?}", report.outcome()), "wal_bytes": wal_bytes(path),
            }));
        }
        writers
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    println!(
        "CHECKPOINT_BENCH {}",
        json!({
            "variant": variant, "phase": label, "writes": samples.len(),
            "writes_per_second": samples.len() as f64 / start.elapsed().as_secs_f64(),
            "queue_wait_us": quantiles(samples.iter().map(|s| s.0).collect()),
            "write_latency_us": quantiles(samples.iter().map(|s| s.1).collect()),
            "checkpoints": checkpoint_samples,
        })
    );
}

#[test]
#[ignore = "opt-in WAL checkpoint workload comparison; prints JSON observations"]
fn compare_checkpoint_workloads() {
    let variants: [(&str, u64, Checkpoint); 3] = [
        ("truncate-unlimited", i64::MAX as u64, Database::checkpoint),
        (
            "passive-unlimited",
            i64::MAX as u64,
            Database::checkpoint_passive,
        ),
        (
            "passive-16mib",
            16 * 1024 * 1024,
            Database::checkpoint_passive,
        ),
    ];
    for (variant, limit, checkpoint) in variants {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("host.sqlite");
        let config = Config {
            // Shorten only the benchmark's deliberate old-behavior stalls.
            busy_timeout: Duration::from_millis(250),
            wal_reuse_limit_bytes: limit,
            ..Default::default()
        };
        let writer = crate::Writer::start(path.clone(), config.clone(), 16).unwrap();
        writer
            .call(|db| {
                db.begin_log_send(&[48; 17], None, 1)?;
                db.begin_log_send_file(&LogSendFileMutation {
                    log_send_id: &[48; 17],
                    file_id: 1,
                    filename: "checkpoint-fixture",
                    content_length: 32 * 1024 * 1024,
                    block_count: 8,
                    content_hash: &[0; 32],
                    now: 1,
                })?;
                db.checkpoint()?;
                Ok(())
            })
            .unwrap();
        let reader = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        reader
            .execute_batch("BEGIN; SELECT * FROM log_sends")
            .unwrap();
        for number in 0..8 {
            writer
                .call(move |db| {
                    db.put_log_send_block(&LogSendBlockMutation {
                        log_send_id: &[48; 17],
                        file_id: 1,
                        block_number: number,
                        block: &vec![number as u8; 4 * 1024 * 1024],
                        now: 1,
                    })?;
                    Ok(())
                })
                .unwrap();
        }
        let metrics = Arc::new(ServerMetrics::default());
        let next = AtomicU64::new(0);
        let pragmas = writer.call(|db| Ok(db.pragmas()?)).unwrap();
        println!(
            "CHECKPOINT_BENCH {}",
            json!({
                "variant": variant, "phase": "configuration", "sqlite": rusqlite::version(),
                "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                "parallelism": std::thread::available_parallelism().map(usize::from).unwrap_or(1),
                "busy_timeout_ms": pragmas.busy_timeout_millis, "synchronous": pragmas.synchronous,
                "auto_checkpoint_pages": pragmas.wal_autocheckpoint_pages, "reuse_limit_bytes": limit,
                "initial_wal_bytes": wal_bytes(&path),
            })
        );
        exercise(
            "held-reader",
            variant,
            &writer,
            &path,
            &metrics,
            &next,
            checkpoint,
        );
        reader.execute_batch("ROLLBACK").unwrap();
        exercise(
            "catch-up", variant, &writer, &path, &metrics, &next, checkpoint,
        );

        let stop = AtomicBool::new(false);
        let ready = Barrier::new(3);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    let reader = rusqlite::Connection::open_with_flags(
                        &path,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )
                    .unwrap();
                    reader.execute_batch("BEGIN; SELECT * FROM names").unwrap();
                    ready.wait();
                    while !stop.load(Ordering::Relaxed) {
                        reader
                            .execute_batch("ROLLBACK; BEGIN; SELECT * FROM names")
                            .unwrap();
                        std::thread::yield_now();
                    }
                    reader.execute_batch("ROLLBACK").unwrap();
                });
            }
            ready.wait();
            let stop_on_exit = StopReaders(&stop);
            exercise(
                "overlapping-readers",
                variant,
                &writer,
                &path,
                &metrics,
                &next,
                checkpoint,
            );
            drop(stop_on_exit);
        });

        let backup = temporary.path().join("backup.sqlite");
        let before_backup = next.load(Ordering::Relaxed);
        let ready = Barrier::new(2);
        std::thread::scope(|scope| {
            let backup_task = scope.spawn(|| {
                let source = foks_server_db::ReadDatabase::open(&path, config.clone()).unwrap();
                ready.wait();
                source.online_backup(&backup).unwrap();
            });
            ready.wait();
            exercise(
                "online-backup",
                variant,
                &writer,
                &path,
                &metrics,
                &next,
                checkpoint,
            );
            backup_task.join().unwrap();
        });
        let backed_up = foks_server_db::ReadDatabase::open(&backup, config.clone()).unwrap();
        assert!(backed_up.integrity_check().unwrap());
        let verify = rusqlite::Connection::open_with_flags(
            &backup,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let bytes: i64 = verify
            .query_row(
                "SELECT sum(length(block)) FROM log_send_blocks",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bytes, 32 * 1024 * 1024);
        let backup_names: i64 = verify
            .query_row("SELECT count(*) FROM names", [], |row| row.get(0))
            .unwrap();
        assert!((before_backup..=next.load(Ordering::Relaxed)).contains(&(backup_names as u64)));
        let report = writer.call(|db| Ok(db.checkpoint_passive()?)).unwrap();
        assert_eq!(
            report.outcome(),
            foks_server_db::CheckpointOutcome::Complete
        );
        let before_reuse = wal_bytes(&path);
        writer
            .call(|db| {
                db.reserve_name(b"reuse", &[9; 17], 1, 1, 10_000_000)?;
                Ok(())
            })
            .unwrap();
        println!(
            "CHECKPOINT_BENCH {}",
            json!({"variant": variant, "phase": "reuse", "before_bytes": before_reuse, "after_bytes": wal_bytes(&path)})
        );
        drop(reader);
        writer.shutdown().unwrap();
        let restarted = Database::open_existing(&path, config).unwrap();
        assert!(restarted.integrity_check().unwrap());
        let verify = restarted.open_reader().unwrap();
        let names: i64 = verify
            .query_row("SELECT count(*) FROM names", [], |row| row.get(0))
            .unwrap();
        assert_eq!(names as u64, next.load(Ordering::Relaxed) + 1);
    }
}
