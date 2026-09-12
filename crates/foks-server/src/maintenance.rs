use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{Error, Result, ServerMetrics, WriterHandle};

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
    ) -> Self {
        let (stop, receiver) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || loop {
            match receiver.recv_timeout(INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    // Failures and committed reclamation counts are recorded by
                    // the shared entry point; the next tick retries durable work.
                    let _ = run(&writer, Arc::clone(&clock), Arc::clone(&metrics));
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
        let checkpoint = database.checkpoint()?;
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
                Arc::clone(&metrics)
            )
            .is_err());
        }
        let failed = metrics.snapshot();
        assert_eq!(failed.maintenance_attempts, 3);
        assert_eq!(failed.maintenance_failures, 3);
        assert_eq!(failed.maintenance_consecutive_failures, 3);
        assert_eq!(failed.maintenance_successes, 0);
        run(
            &writer.handle(),
            Arc::new(FixedClock(1_000_000)),
            Arc::clone(&metrics),
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
