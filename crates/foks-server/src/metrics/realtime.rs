use foks_server_db::{RealtimeReconcileOutcome, RealtimeReconcileReport};
use std::{sync::Mutex, time::Duration};

pub(crate) use super::checkpoint::BUCKET_MICROS;

#[derive(Clone, Copy)]
pub(crate) enum ReconcileSource {
    Poll,
    Delta,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReconcileMetricsSnapshot {
    pub state_reads: u64,
    pub clean_skips: u64,
    pub submissions: u64,
    pub submission_failures: u64,
    pub execution_failures: u64,
    pub already_clean: u64,
    pub complete: u64,
    pub incomplete: u64,
    pub restarted: u64,
    pub candidates: u64,
    pub accessibility_changes: u64,
    pub queue_wait_buckets: [u64; 9],
    pub execution_buckets: [u64; 9],
    pub queue_wait_microseconds_total: u64,
    pub execution_microseconds_total: u64,
}
#[derive(Clone, Copy)]
pub(crate) enum FanoutOperation {
    Registration,
    Recipients,
    Membership,
}
#[derive(Default)]
pub(crate) struct FanoutWork {
    pub recipient_keys: u64,
    pub entries_examined: u64,
    pub expired_entries_removed: u64,
    pub listeners_woken: u64,
    pub mutex_wait: Duration,
    pub mutex_hold: Duration,
    pub wake: Duration,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FanoutMetricsSnapshot {
    pub calls: u64,
    pub recipient_keys: u64,
    pub entries_examined: u64,
    pub expired_entries_removed: u64,
    pub listeners_woken: u64,
    pub mutex_wait_buckets: [u64; 9],
    pub mutex_hold_buckets: [u64; 9],
    pub wake_buckets: [u64; 9],
    pub mutex_wait_microseconds_total: u64,
    pub mutex_hold_microseconds_total: u64,
    pub wake_microseconds_total: u64,
    /// Preserve sub-microsecond samples before deriving the compatibility totals.
    pub mutex_wait_nanoseconds_total: u64,
    pub mutex_hold_nanoseconds_total: u64,
    pub wake_nanoseconds_total: u64,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RealtimeMetricsSnapshot {
    pub registration: FanoutMetricsSnapshot,
    pub notification: FanoutMetricsSnapshot,
    pub membership: FanoutMetricsSnapshot,
    pub poll: ReconcileMetricsSnapshot,
    pub delta: ReconcileMetricsSnapshot,
    pub hint_wakes: u64,
    pub fallback_wakes: u64,
    pub bumped_polls: u64,
    pub timed_out_polls: u64,
    pub failed_polls: u64,
    pub cancelled_polls: u64,
}
#[derive(Default)]
pub(crate) struct RealtimeMetrics(Mutex<RealtimeMetricsSnapshot>);
impl RealtimeMetrics {
    pub(crate) fn fanout(&self, operation: FanoutOperation, work: FanoutWork) {
        self.update(|s| {
            let s = match operation {
                FanoutOperation::Registration => &mut s.registration,
                FanoutOperation::Recipients => &mut s.notification,
                FanoutOperation::Membership => &mut s.membership,
            };
            s.calls += 1;
            s.recipient_keys += work.recipient_keys;
            s.entries_examined += work.entries_examined;
            s.expired_entries_removed += work.expired_entries_removed;
            s.listeners_woken += work.listeners_woken;
            for (duration, buckets, nanoseconds, microseconds) in [
                (
                    work.mutex_wait,
                    &mut s.mutex_wait_buckets,
                    &mut s.mutex_wait_nanoseconds_total,
                    &mut s.mutex_wait_microseconds_total,
                ),
                (
                    work.mutex_hold,
                    &mut s.mutex_hold_buckets,
                    &mut s.mutex_hold_nanoseconds_total,
                    &mut s.mutex_hold_microseconds_total,
                ),
                (
                    work.wake,
                    &mut s.wake_buckets,
                    &mut s.wake_nanoseconds_total,
                    &mut s.wake_microseconds_total,
                ),
            ] {
                for (i, bound) in BUCKET_MICROS.iter().enumerate() {
                    buckets[i] += u64::from(duration <= Duration::from_micros(*bound));
                }
                buckets[8] += 1;
                *nanoseconds = nanoseconds
                    .saturating_add(u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX));
                *microseconds = *nanoseconds / 1_000;
            }
        });
    }
    pub(crate) fn snapshot(&self) -> RealtimeMetricsSnapshot {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(crate) fn update(&self, f: impl FnOnce(&mut RealtimeMetricsSnapshot)) {
        f(&mut self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner));
    }
    pub(crate) fn source(
        &self,
        source: ReconcileSource,
        f: impl FnOnce(&mut ReconcileMetricsSnapshot),
    ) {
        self.update(|s| {
            f(match source {
                ReconcileSource::Poll => &mut s.poll,
                ReconcileSource::Delta => &mut s.delta,
            })
        });
    }
    pub(crate) fn report(&self, source: ReconcileSource, r: RealtimeReconcileReport) {
        self.source(source, |s| {
            match r.outcome {
                RealtimeReconcileOutcome::AlreadyClean => s.already_clean += 1,
                RealtimeReconcileOutcome::PageComplete => s.complete += 1,
                RealtimeReconcileOutcome::PageIncomplete => s.incomplete += 1,
            }
            s.restarted += u64::from(r.restarted);
            s.candidates += r.candidates as u64;
            s.accessibility_changes += r.accessibility_changes as u64;
        });
    }
    pub(crate) fn execution(
        &self,
        source: ReconcileSource,
        queue: Duration,
        execution: Duration,
        failed: bool,
    ) {
        self.source(source, |s| {
            s.execution_failures += u64::from(failed);
            for (duration, buckets, total) in [
                (
                    queue,
                    &mut s.queue_wait_buckets,
                    &mut s.queue_wait_microseconds_total,
                ),
                (
                    execution,
                    &mut s.execution_buckets,
                    &mut s.execution_microseconds_total,
                ),
            ] {
                for (i, bound) in BUCKET_MICROS.iter().enumerate() {
                    buckets[i] += u64::from(duration <= Duration::from_micros(*bound));
                }
                buckets[8] += 1;
                *total =
                    total.saturating_add(u64::try_from(duration.as_micros()).unwrap_or(u64::MAX));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fanout_totals_preserve_submicrosecond_work_per_operation() {
        let metrics = RealtimeMetrics::default();
        for operation in [
            FanoutOperation::Registration,
            FanoutOperation::Recipients,
            FanoutOperation::Membership,
        ] {
            for _ in 0..3 {
                metrics.fanout(
                    operation,
                    FanoutWork {
                        mutex_wait: Duration::from_nanos(400),
                        mutex_hold: Duration::from_nanos(250),
                        wake: Duration::from_nanos(100),
                        ..Default::default()
                    },
                );
            }
        }
        let snapshot = metrics.snapshot();
        for sample in [
            snapshot.registration,
            snapshot.notification,
            snapshot.membership,
        ] {
            assert_eq!(sample.mutex_wait_nanoseconds_total, 1_200);
            assert_eq!(sample.mutex_wait_microseconds_total, 1);
            assert_eq!(sample.mutex_hold_nanoseconds_total, 750);
            assert_eq!(sample.mutex_hold_microseconds_total, 0);
            assert_eq!(sample.wake_nanoseconds_total, 300);
            assert_eq!(sample.wake_microseconds_total, 0);
            assert_eq!(sample.mutex_wait_buckets, [3; 9]);
            assert_eq!(sample.mutex_hold_buckets, [3; 9]);
            assert_eq!(sample.wake_buckets, [3; 9]);
        }
    }

    #[test]
    fn sources_outcomes_and_worker_histograms_are_independent() {
        let m = RealtimeMetrics::default();
        m.source(ReconcileSource::Poll, |s| s.submissions += 3);
        for outcome in [
            RealtimeReconcileOutcome::AlreadyClean,
            RealtimeReconcileOutcome::PageComplete,
            RealtimeReconcileOutcome::PageIncomplete,
        ] {
            m.report(
                ReconcileSource::Poll,
                RealtimeReconcileReport {
                    outcome,
                    restarted: true,
                    candidates: 4,
                    accessibility_changes: 2,
                },
            );
        }
        m.execution(
            ReconcileSource::Poll,
            Duration::from_millis(5),
            Duration::from_millis(1),
            false,
        );
        m.execution(
            ReconcileSource::Delta,
            Duration::from_secs(6),
            Duration::from_millis(10),
            true,
        );
        let s = m.snapshot();
        assert_eq!(
            (
                s.poll.already_clean,
                s.poll.complete,
                s.poll.incomplete,
                s.poll.restarted
            ),
            (1, 1, 1, 3)
        );
        assert_eq!((s.poll.candidates, s.poll.accessibility_changes), (12, 6));
        assert_eq!(s.poll.queue_wait_buckets, [0, 1, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(s.delta.queue_wait_buckets, [0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(
            (s.poll.execution_failures, s.delta.execution_failures),
            (0, 1)
        );
        assert_eq!(
            (
                s.poll.execution_microseconds_total,
                s.delta.execution_microseconds_total
            ),
            (1000, 10000)
        );
        assert_eq!(s.delta.complete, 0);
    }
}
