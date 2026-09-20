use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{Error, Result, ServerMetrics, WriterHandle};

#[cfg(test)]
mod checkpoint_bench;

const INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const ABANDONED_UPLOAD_AGE_MICROS: u64 = 24 * 60 * 60 * 1_000_000;

pub(crate) struct Maintenance {
    stop: SyncSender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Maintenance {
    pub(crate) fn start(
        writer: WriterHandle,
        clock: Arc<dyn foks_server_db::Clock>,
        metrics: Arc<ServerMetrics>,
        admin: Option<Arc<crate::web_admin::WebAdminService>>,
    ) -> Self {
        let (stop, receiver) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || loop {
            match receiver.recv_timeout(INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    // Failures and committed reclamation counts are recorded by
                    // the shared entry point; the next tick retries durable work.
                    let _ = run(
                        &writer,
                        Arc::clone(&clock),
                        Arc::clone(&metrics),
                        admin.clone(),
                    );
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn shutdown(mut self) -> Result<()> {
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| Error::Thread)?;
        }
        Ok(())
    }
}

impl Drop for Maintenance {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The manual and periodic paths use identical accounting and cleanup policy.
pub(crate) fn run(
    writer: &WriterHandle,
    clock: Arc<dyn foks_server_db::Clock>,
    metrics: Arc<ServerMetrics>,
    admin: Option<Arc<crate::web_admin::WebAdminService>>,
) -> Result<(
    foks_server_db::MaintenanceReport,
    foks_server_db::CheckpointReport,
)> {
    run_with_checkpoint(
        writer,
        clock,
        metrics,
        admin,
        foks_server_db::Database::checkpoint_passive,
    )
}

// A narrow injection point lets tests fail only the checkpoint after real cleanup.
fn run_with_checkpoint(
    writer: &WriterHandle,
    clock: Arc<dyn foks_server_db::Clock>,
    metrics: Arc<ServerMetrics>,
    admin: Option<Arc<crate::web_admin::WebAdminService>>,
    checkpoint: fn(
        &foks_server_db::Database,
    ) -> foks_server_db::Result<foks_server_db::CheckpointReport>,
) -> Result<(
    foks_server_db::MaintenanceReport,
    foks_server_db::CheckpointReport,
)> {
    metrics.maintenance_attempted();
    let worker_metrics = Arc::clone(&metrics);
    let result = writer.call_with_current_time(clock, move |database, now| {
        let cutoff = now.saturating_sub(ABANDONED_UPLOAD_AGE_MICROS);
        let report = database.run_maintenance(now, cutoff)?;
        worker_metrics.uploads_reclaimed(&report);
        if let Some(admin) = admin {
            worker_metrics.admin_cleanup_attempted();
            match admin
                .clock
                .sample()
                .and_then(|now| Ok(database.web_cleanup(now)?))
            {
                Ok(report) => worker_metrics.admin_cleanup_succeeded(&report),
                Err(error) => {
                    worker_metrics.admin_cleanup_failed();
                    return Err(error);
                }
            }
        }
        worker_metrics.checkpoint_attempted();
        let started = Instant::now();
        let checkpoint = checkpoint(database);
        worker_metrics.checkpoint_finished(&checkpoint, now / 1_000_000, started.elapsed());
        let checkpoint = checkpoint?;
        worker_metrics.maintenance_succeeded(now / 1_000_000);
        Ok((report, checkpoint))
    });
    if result.is_err() {
        metrics.maintenance_failed();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedClock(u64);
    impl foks_server_db::Clock for FixedClock {
        fn now_micros(&self) -> foks_server_db::Result<u64> {
            Ok(self.0)
        }
    }

    #[test]
    fn maintenance_and_following_write_finish_with_reader_still_held() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("host.sqlite");
        let writer = crate::Writer::start(
            path.clone(),
            foks_server_db::Config {
                busy_timeout: Duration::from_secs(10),
                ..Default::default()
            },
            4,
        )
        .unwrap();
        writer
            .call(|db| {
                db.reserve_name(b"first", &[1; 17], 1, 1, 10_000_000)?;
                Ok(())
            })
            .unwrap();
        let reader = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        reader.execute_batch("BEGIN; SELECT * FROM names;").unwrap();
        writer
            .call(|db| {
                db.reserve_name(b"second", &[2; 17], 1, 1, 10_000_000)?;
                Ok(())
            })
            .unwrap();
        let metrics = Arc::new(ServerMetrics::default());
        let worker_metrics = Arc::clone(&metrics);
        let handle = writer.handle();
        let (send, receive) = mpsc::sync_channel(1);
        let work = std::thread::spawn(move || {
            let maintenance = run(
                &handle,
                Arc::new(FixedClock(1_000_000)),
                worker_metrics,
                None,
            );
            let following = handle.call(|db| {
                db.reserve_name(b"third", &[3; 17], 1, 1, 10_000_000)?;
                Ok(())
            });
            send.send((maintenance, following)).unwrap();
        });
        let result = receive.recv_timeout(Duration::from_secs(3));
        // Unpin and join even on regression, before failing the watchdog assertion.
        reader.execute_batch("ROLLBACK").unwrap();
        work.join().unwrap();
        let completed = writer.call(|db| Ok(db.checkpoint_passive()?)).unwrap();
        writer.shutdown().unwrap();
        let (maintenance, following) = result.expect("maintenance waited for the held reader");
        let (_, checkpoint) = maintenance.unwrap();
        following.unwrap();
        assert_eq!(checkpoint.busy, 0);
        assert_eq!(
            checkpoint.outcome(),
            foks_server_db::CheckpointOutcome::Deferred
        );
        assert_eq!(
            completed.outcome(),
            foks_server_db::CheckpointOutcome::Complete
        );
        let metrics = metrics.snapshot();
        assert_eq!(metrics.maintenance_successes, 1);
        assert_eq!(metrics.maintenance_failures, 0);
        assert_eq!(metrics.checkpoint.attempts, 1);
        assert_eq!(metrics.checkpoint.deferred, 1);
        assert_eq!(metrics.checkpoint.complete, 0);
    }

    #[test]
    fn checkpoint_failure_keeps_committed_cleanup_accounting() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("host.sqlite");
        drop(foks_server_db::Database::open(&path, Default::default()).unwrap());
        let seed = rusqlite::Connection::open(&path).unwrap();
        seed.execute_batch(
            "PRAGMA foreign_keys=ON;
            INSERT INTO kv_namespaces VALUES(zeroblob(33), 1, zeroblob(33));
            INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at)
                VALUES(zeroblob(33),zeroblob(16),x'00',1,1);
            INSERT INTO kv_file_chunks VALUES(zeroblob(33),zeroblob(16),0,zeroblob(12),0,x'00');",
        )
        .unwrap();
        drop(seed);
        let writer = crate::Writer::start(path.clone(), Default::default(), 4).unwrap();
        let metrics = Arc::new(ServerMetrics::default());
        let result = run_with_checkpoint(
            &writer.handle(),
            Arc::new(FixedClock(ABANDONED_UPLOAD_AGE_MICROS + 2)),
            Arc::clone(&metrics),
            None,
            |_| {
                Err(foks_server_db::Error::Invalid(
                    "injected checkpoint failure",
                ))
            },
        );
        assert!(result.is_err());
        let first = metrics.snapshot();
        assert_eq!(first.maintenance_failures, 1);
        assert_eq!(first.maintenance_successes, 0);
        assert_eq!(first.reclaimed_uploads, 1);
        assert_eq!(first.reclaimed_upload_chunks, 1);
        assert_eq!(first.reclaimed_upload_bytes, 13);
        assert_eq!(first.checkpoint.attempts, 1);
        assert_eq!(first.checkpoint.errors, 1);
        assert_eq!(first.checkpoint.duration_buckets[8], 1);
        assert!(!first.checkpoint.frame_counts_available);
        run(
            &writer.handle(),
            Arc::new(FixedClock(ABANDONED_UPLOAD_AGE_MICROS + 3)),
            Arc::clone(&metrics),
            None,
        )
        .unwrap();
        assert_eq!(metrics.snapshot().reclaimed_uploads, 1);
        let remaining: i64 = rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM kv_file_uploads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
        writer.shutdown().unwrap();
    }

    #[test]
    fn failed_clock_samples_are_counted_and_success_resets_failure_streak() {
        let temporary = tempfile::tempdir().unwrap();
        let writer = crate::Writer::start(
            temporary.path().join("host.sqlite"),
            foks_server_db::Config::default(),
            4,
        )
        .unwrap();
        let metrics = Arc::new(ServerMetrics::default());
        // SQL timestamps reject out-of-range input before any cleanup commits.
        for _ in 0..3 {
            assert!(run(
                &writer.handle(),
                Arc::new(FixedClock(u64::MAX)),
                Arc::clone(&metrics),
                None
            )
            .is_err());
        }
        let failed = metrics.snapshot();
        assert_eq!(failed.maintenance_attempts, 3);
        assert_eq!(failed.maintenance_failures, 3);
        assert_eq!(failed.maintenance_consecutive_failures, 3);
        assert_eq!(failed.maintenance_successes, 0);
        assert_eq!(failed.checkpoint.attempts, 0);
        run(
            &writer.handle(),
            Arc::new(FixedClock(1_000_000)),
            Arc::clone(&metrics),
            None,
        )
        .unwrap();
        let recovered = metrics.snapshot();
        assert_eq!(recovered.maintenance_attempts, 4);
        assert_eq!(recovered.maintenance_successes, 1);
        assert_eq!(recovered.maintenance_failures, 3);
        assert_eq!(recovered.maintenance_consecutive_failures, 0);
        assert_eq!(recovered.last_maintenance_success_unixtime, 1);
        writer.shutdown().unwrap();
    }
}
