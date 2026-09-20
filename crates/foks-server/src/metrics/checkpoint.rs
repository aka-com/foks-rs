use std::sync::Mutex;
use std::time::Duration;

use foks_server_db::{CheckpointOutcome, CheckpointReport};

// Eight finite bounds and an implicit +Inf bucket. All counts are cumulative.
pub(crate) const BUCKET_MICROS: [u64; 8] = [
    1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000, 5_000_000,
];

/// Explicit online-maintenance checkpoints only; excludes SQLite auto-checkpoints
/// and direct calls to the database checkpoint APIs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CheckpointMetricsSnapshot {
    pub attempts: u64,
    pub complete: u64,
    pub deferred: u64,
    pub unavailable: u64,
    pub errors: u64,
    pub busy: u64,
    pub duration_buckets: [u64; 9],
    pub duration_microseconds_total: u64,
    pub frame_counts_available: bool,
    /// Last known positions, retained as stale when counts are unavailable.
    pub wal_frames: u64,
    pub backfilled_frames: u64,
    pub remaining_frames: u64,
    pub last_observation_unixtime: u64,
    pub last_complete_unixtime: u64,
}

#[derive(Default)]
pub(super) struct CheckpointMetrics(Mutex<CheckpointMetricsSnapshot>);

impl CheckpointMetrics {
    pub(super) fn snapshot(&self) -> CheckpointMetricsSnapshot {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn attempted(&self) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.attempts += 1;
        state.frame_counts_available = false;
    }

    pub(super) fn finished(
        &self,
        result: &foks_server_db::Result<CheckpointReport>,
        unixtime: u64,
        duration: Duration,
    ) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let micros = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        for (index, bound) in BUCKET_MICROS.iter().enumerate() {
            if duration <= Duration::from_micros(*bound) {
                state.duration_buckets[index] += 1;
            }
        }
        state.duration_buckets[8] += 1;
        state.duration_microseconds_total =
            state.duration_microseconds_total.saturating_add(micros);
        state.last_observation_unixtime = unixtime;
        state.frame_counts_available = false;
        match result {
            Ok(report) => {
                if let (Some(log), Some(done)) = (report.wal_pages, report.checkpointed_pages) {
                    state.frame_counts_available = true;
                    state.wal_frames = log;
                    state.backfilled_frames = done;
                    state.remaining_frames = log - done;
                }
                if report.busy != 0 {
                    state.busy += 1;
                }
                match report.outcome() {
                    CheckpointOutcome::Complete => {
                        state.complete += 1;
                        state.last_complete_unixtime = unixtime;
                    }
                    CheckpointOutcome::Deferred => state.deferred += 1,
                    CheckpointOutcome::Unavailable => state.unavailable += 1,
                }
            }
            Err(_) => state.errors += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_histogram_and_stale_positions_are_coherent() {
        let metrics = CheckpointMetrics::default();
        let observations = [
            (0, Some(0), Some(0)),
            (0, Some(10), Some(4)),
            (1, Some(10), Some(10)),
            (0, Some(10), Some(10)),
            (0, None, None),
            (1, None, None),
        ];
        for (index, (busy, wal_pages, checkpointed_pages)) in observations.into_iter().enumerate() {
            metrics.attempted();
            assert!(!metrics.snapshot().frame_counts_available);
            metrics.finished(
                &Ok(CheckpointReport {
                    busy,
                    wal_pages,
                    checkpointed_pages,
                }),
                index as u64 + 1,
                Duration::from_millis(5),
            );
            let state = metrics.snapshot();
            assert_eq!(
                state.attempts,
                state.complete + state.deferred + state.unavailable + state.errors
            );
            assert_eq!(state.frame_counts_available, wal_pages.is_some());
            assert_eq!(state.last_observation_unixtime, index as u64 + 1);
        }
        metrics.attempted();
        metrics.finished(
            &Err(foks_server_db::Error::Invalid("checkpoint failure")),
            7,
            Duration::from_secs(6),
        );
        let state = metrics.snapshot();
        assert_eq!(
            (
                state.attempts,
                state.complete,
                state.deferred,
                state.unavailable,
                state.errors,
                state.busy
            ),
            (7, 2, 3, 1, 1, 2)
        );
        assert_eq!(state.duration_buckets, [0, 6, 6, 6, 6, 6, 6, 6, 7]);
        assert_eq!(state.duration_microseconds_total, 6_030_000);
        assert!(!state.frame_counts_available);
        assert_eq!(
            (
                state.wal_frames,
                state.backfilled_frames,
                state.remaining_frames
            ),
            (10, 10, 0)
        );
        assert_eq!(state.last_observation_unixtime, 7);
        assert_eq!(state.last_complete_unixtime, 4);

        // A later failure must also preserve a *nonzero* last known backlog.
        metrics.attempted();
        metrics.finished(
            &Ok(CheckpointReport {
                busy: 0,
                wal_pages: Some(9),
                checkpointed_pages: Some(3),
            }),
            8,
            Duration::ZERO,
        );
        for result in [
            Ok(CheckpointReport {
                busy: 0,
                wal_pages: None,
                checkpointed_pages: None,
            }),
            Err(foks_server_db::Error::Invalid("failed")),
        ] {
            metrics.attempted();
            metrics.finished(&result, 9, Duration::ZERO);
            let state = metrics.snapshot();
            assert!(!state.frame_counts_available);
            assert_eq!(
                (
                    state.wal_frames,
                    state.backfilled_frames,
                    state.remaining_frames
                ),
                (9, 3, 6)
            );
            assert_eq!(state.last_complete_unixtime, 4);
        }
    }
}
