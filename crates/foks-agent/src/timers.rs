//! Tracks timing and outcomes for agent background loops. Retention, scheduling,
//! compatibility refresh, and ownership checks share profile admission and
//! locks with foreground requests, so their timings are included in diagnostics.
//! `AgentStatus` reports the most recent execution and skipped-tick count for
//! each loop without profile, account, or error details.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use foks_agent_proto::TimerStatus;

pub const RETENTION: &str = "retention";
pub const SCHEDULER: &str = "scheduler";
pub const COMPATIBILITY: &str = "compatibility";
pub const OWNERSHIP: &str = "ownership";

/// Outcome of a loop execution. Dropping an unfinished execution records
/// `interrupted`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Ok,
    Error,
    Interrupted,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Default)]
struct Record {
    started_at_ms: Option<u64>,
    duration_ms: Option<u32>,
    outcome: Option<&'static str>,
    next_due_at_ms: Option<u64>,
    runs: u64,
    skips: u64,
}

/// The loops' bookkeeping, in the order they are reported.
#[derive(Default)]
pub struct Timers {
    records: Mutex<BTreeMap<&'static str, Record>>,
}

/// Returns the process-wide background-loop timing registry.
pub fn timers() -> &'static Timers {
    static TIMERS: OnceLock<Timers> = OnceLock::new();
    TIMERS.get_or_init(Timers::default)
}

/// Returns Unix time in milliseconds, or zero if the system clock predates the
/// epoch. The backend omits zero timestamps.
pub fn now_milliseconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

impl Timers {
    fn with<T>(&self, name: &'static str, apply: impl FnOnce(&mut Record) -> T) -> T {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        apply(records.entry(name).or_default())
    }

    /// Records that a tick fired and when the next one is due.
    pub fn tick(&self, name: &'static str, now_ms: u64, period: Duration) {
        let due = now_ms.saturating_add(millis(period));
        self.with(name, |record| record.next_due_at_ms = Some(due));
    }

    /// Records a tick that did no work: the agent was not ready yet, the
    /// previous pass was still running, or nothing was due.
    pub fn skipped(&self, name: &'static str) {
        self.with(name, |record| record.skips = record.skips.saturating_add(1));
    }

    /// Starts a loop execution. `finish` records its explicit outcome; dropping
    /// it first records `interrupted`.
    pub fn begin(&'static self, name: &'static str, now_ms: u64) -> TimerRun {
        TimerRun {
            timers: self,
            name,
            started_at_ms: now_ms,
            reported: false,
        }
    }

    fn complete(&self, name: &'static str, started_at_ms: u64, taken: Duration, outcome: Outcome) {
        let duration = u32::try_from(taken.as_millis()).unwrap_or(u32::MAX);
        self.with(name, |record| {
            record.started_at_ms = Some(started_at_ms);
            record.duration_ms = Some(duration);
            record.outcome = Some(outcome.as_str());
            record.runs = record.runs.saturating_add(1);
        });
    }

    /// Returns the most recent status for each loop. `next_due_in_ms` is relative
    /// to `now_ms` and is zero for overdue ticks.
    pub fn snapshot(&self, now_ms: u64) -> Vec<TimerStatus> {
        let records = self
            .records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        records
            .iter()
            .map(|(name, record)| TimerStatus {
                name: (*name).to_owned(),
                started_at_ms: record.started_at_ms,
                duration_ms: record.duration_ms,
                outcome: record.outcome.map(str::to_owned),
                next_due_in_ms: record.next_due_at_ms.map(|due| due.saturating_sub(now_ms)),
                runs: record.runs,
                skips: record.skips,
            })
            .collect()
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Tracks one background-loop execution until completion or cancellation.
pub struct TimerRun {
    timers: &'static Timers,
    name: &'static str,
    started_at_ms: u64,
    reported: bool,
}

impl TimerRun {
    /// Records the execution outcome once.
    pub fn finish(mut self, outcome: Outcome) {
        self.report(outcome);
    }

    fn report(&mut self, outcome: Outcome) {
        if self.reported {
            return;
        }
        self.reported = true;
        let taken = Duration::from_millis(
            now_milliseconds()
                .saturating_sub(self.started_at_ms)
                .min(u64::from(u32::MAX)),
        );
        self.timers
            .complete(self.name, self.started_at_ms, taken, outcome);
    }
}

impl Drop for TimerRun {
    fn drop(&mut self) {
        self.report(Outcome::Interrupted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> &'static Timers {
        // Each test needs its own registry: the reported state is per loop
        // name, and the process-wide one is shared with the running agent.
        Box::leak(Box::new(Timers::default()))
    }

    #[test]
    fn a_pass_reports_its_start_duration_outcome_and_next_due_time() {
        let timers = registry();
        assert!(timers.snapshot(0).is_empty());
        timers.tick(SCHEDULER, 1_000, Duration::from_secs(30));
        let reported = timers.snapshot(1_000);
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].name, SCHEDULER);
        assert_eq!(reported[0].next_due_in_ms, Some(30_000));
        assert_eq!(reported[0].started_at_ms, None);
        assert_eq!(reported[0].runs, 0);

        let started = now_milliseconds();
        timers.begin(SCHEDULER, started).finish(Outcome::Ok);
        let reported = timers.snapshot(11_000);
        assert_eq!(reported[0].started_at_ms, Some(started));
        assert_eq!(reported[0].outcome.as_deref(), Some("ok"));
        assert!(reported[0].duration_ms.is_some());
        assert_eq!(reported[0].runs, 1);
        assert_eq!(reported[0].next_due_in_ms, Some(20_000));
        // An overdue tick reports zero rather than wrapping.
        assert_eq!(timers.snapshot(60_000)[0].next_due_in_ms, Some(0));
    }

    #[test]
    fn skipped_ticks_are_counted_without_replacing_the_last_pass() {
        let timers = registry();
        timers.begin(RETENTION, 5_000).finish(Outcome::Error);
        timers.skipped(RETENTION);
        timers.skipped(RETENTION);
        let reported = timers.snapshot(5_000);
        assert_eq!(reported[0].skips, 2);
        assert_eq!(reported[0].runs, 1);
        assert_eq!(reported[0].started_at_ms, Some(5_000));
        assert_eq!(reported[0].outcome.as_deref(), Some("error"));
    }

    #[test]
    fn a_dropped_pass_is_reported_as_interrupted_and_reports_once() {
        let timers = registry();
        drop(timers.begin(COMPATIBILITY, 7_000));
        assert_eq!(
            timers.snapshot(7_000)[0].outcome.as_deref(),
            Some("interrupted")
        );
        let run = timers.begin(COMPATIBILITY, 8_000);
        run.finish(Outcome::Ok);
        let reported = timers.snapshot(8_000);
        assert_eq!(reported[0].outcome.as_deref(), Some("ok"));
        // The finished pass was not counted again when its run was dropped.
        assert_eq!(reported[0].runs, 2);
    }

    #[test]
    fn every_loop_is_reported_under_its_own_name() {
        let timers = registry();
        for name in [RETENTION, SCHEDULER, COMPATIBILITY, OWNERSHIP] {
            timers.begin(name, 1).finish(Outcome::Ok);
        }
        timers.tick(SCHEDULER, 1_000, Duration::from_millis(250));
        let reported = timers.snapshot(1_000);
        assert_eq!(
            reported
                .iter()
                .map(|timer| timer.name.clone())
                .collect::<Vec<_>>(),
            vec![COMPATIBILITY, OWNERSHIP, RETENTION, SCHEDULER]
        );
        assert_eq!(
            reported
                .iter()
                .find(|timer| timer.name == SCHEDULER)
                .and_then(|timer| timer.next_due_in_ms),
            Some(250)
        );
    }
}
