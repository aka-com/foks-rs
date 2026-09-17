//! Durable scheduling for periodic user refresh and mutation reconciliation.

use std::path::{Path, PathBuf};

use foks_client_db::{HardStateStore, ScheduledJob, ScheduledJobKind};
use foks_crypto::prefixed_hash;

use crate::{Error, Result};

const SCHEDULER_JITTER_TYPE_ID: u64 = 0xf6a4_446a_c45b_a435;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerConfig {
    pub lease_micros: u64,
    pub base_backoff_micros: u64,
    pub max_backoff_micros: u64,
    pub jitter_percent: u8,
    pub claim_limit: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            lease_micros: 5 * 60 * 1_000_000,
            base_backoff_micros: 5 * 1_000_000,
            max_backoff_micros: 60 * 60 * 1_000_000,
            jitter_percent: 20,
            claim_limit: 16,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduledRunStatus {
    Completed,
    Failed { error: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledRun {
    pub job_id: [u8; 16],
    pub kind: ScheduledJobKind,
    pub status: ScheduledRunStatus,
    pub next_run_at: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SchedulerRunReport {
    pub runs: Vec<ScheduledRun>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledJobRegistration {
    pub job_id: [u8; 16],
    pub kind: ScheduledJobKind,
    pub host_id: Vec<u8>,
    pub scope_id: Vec<u8>,
    pub interval_micros: u64,
    pub first_run_at: u64,
    pub registered_at: u64,
}

/// Runtime-neutral durable scheduler.
///
/// The application owns the timer and calls [`Self::run_due`] on its blocking
/// storage worker. Handlers receive public job identity only and supply their
/// credentials from the application's protected vault. Handlers must be
/// idempotent: a crash after their side effect but before lease completion can
/// cause the job to run again.
pub struct FoksScheduler {
    hard_database: PathBuf,
    config: SchedulerConfig,
}

impl FoksScheduler {
    pub fn new(hard_database: impl AsRef<Path>, config: SchedulerConfig) -> Result<Self> {
        validate_config(config)?;
        Ok(Self {
            hard_database: hard_database.as_ref().to_path_buf(),
            config,
        })
    }

    pub fn register(&self, registration: ScheduledJobRegistration) -> Result<()> {
        HardStateStore::open(&self.hard_database)?.register_scheduled_job(&ScheduledJob {
            job_id: registration.job_id,
            kind: registration.kind,
            host_id: registration.host_id,
            scope_id: registration.scope_id,
            interval_micros: registration.interval_micros,
            next_run_at: registration.first_run_at,
            failure_count: 0,
            lease_until: None,
            last_completed_at: None,
            last_error: None,
            updated_at: registration.registered_at,
        })?;
        Ok(())
    }

    /// Registers a default job without replacing an operator-selected
    /// interval or execution state on an existing, identically bound job.
    pub fn register_if_missing(&self, registration: ScheduledJobRegistration) -> Result<bool> {
        let mut store = HardStateStore::open(&self.hard_database)?;
        store
            .register_scheduled_job_if_missing(&ScheduledJob {
                job_id: registration.job_id,
                kind: registration.kind,
                host_id: registration.host_id,
                scope_id: registration.scope_id,
                interval_micros: registration.interval_micros,
                next_run_at: registration.first_run_at,
                failure_count: 0,
                lease_until: None,
                last_completed_at: None,
                last_error: None,
                updated_at: registration.registered_at,
            })
            .map_err(Into::into)
    }

    pub fn unregister(&self, job_id: &[u8; 16]) -> Result<bool> {
        HardStateStore::open(&self.hard_database)?
            .remove_scheduled_job(job_id)
            .map_err(Error::from)
    }

    pub fn next_run_at(&self) -> Result<Option<u64>> {
        HardStateStore::open(&self.hard_database)?
            .next_scheduled_run()
            .map_err(Error::from)
    }

    /// Claims and runs one bounded batch. Handler failures are persisted and
    /// returned in the report; storage or lease-integrity failures abort the
    /// batch with an error.
    pub fn run_due(
        &self,
        now: u64,
        mut handler: impl FnMut(&ScheduledJob) -> std::result::Result<(), String>,
    ) -> Result<SchedulerRunReport> {
        let claimed_until = now
            .checked_add(self.config.lease_micros)
            .ok_or(Error::Scheduler("lease timestamp overflow"))?;
        let mut store = HardStateStore::open(&self.hard_database)?;
        let jobs = store.claim_due_scheduled_jobs(now, claimed_until, self.config.claim_limit)?;
        let mut report = SchedulerRunReport {
            runs: Vec::with_capacity(jobs.len()),
        };
        for job in jobs {
            let lease = job
                .lease_until
                .ok_or(Error::Scheduler("claimed job has no lease"))?;
            match handler(&job) {
                Ok(()) => {
                    let delay = jittered_delay(
                        job.interval_micros,
                        self.config.jitter_percent,
                        &job,
                        job.last_completed_at.unwrap_or(0),
                    )?;
                    let next_run_at = now
                        .checked_add(delay)
                        .ok_or(Error::Scheduler("next run timestamp overflow"))?;
                    store.complete_scheduled_job(&job.job_id, lease, next_run_at, now)?;
                    report.runs.push(ScheduledRun {
                        job_id: job.job_id,
                        kind: job.kind,
                        status: ScheduledRunStatus::Completed,
                        next_run_at,
                    });
                }
                Err(error) => {
                    let error = bounded_error(error);
                    let security_backoff_cap = match job.kind {
                        ScheduledJobKind::UserRefresh | ScheduledJobKind::TeamRefresh => {
                            self.config.max_backoff_micros.min(job.interval_micros)
                        }
                        _ => self.config.max_backoff_micros,
                    };
                    let exponential = self
                        .config
                        .base_backoff_micros
                        .saturating_mul(1u64 << job.failure_count.min(62))
                        .min(security_backoff_cap);
                    let delay = jittered_delay(
                        exponential,
                        self.config.jitter_percent,
                        &job,
                        job.failure_count,
                    )?
                    .min(security_backoff_cap);
                    let next_run_at = now
                        .checked_add(delay)
                        .ok_or(Error::Scheduler("retry timestamp overflow"))?;
                    store.fail_scheduled_job(&job.job_id, lease, next_run_at, now, &error)?;
                    report.runs.push(ScheduledRun {
                        job_id: job.job_id,
                        kind: job.kind,
                        status: ScheduledRunStatus::Failed { error },
                        next_run_at,
                    });
                }
            }
        }
        Ok(report)
    }
}

fn validate_config(config: SchedulerConfig) -> Result<()> {
    if config.lease_micros == 0
        || config.base_backoff_micros == 0
        || config.max_backoff_micros < config.base_backoff_micros
        || config.jitter_percent > 100
        || config.claim_limit == 0
    {
        return Err(Error::Scheduler(
            "invalid scheduler configuration: parameters must be non-zero and base backoff must not exceed max backoff",
        ));
    }
    Ok(())
}

fn jittered_delay(
    delay: u64,
    jitter_percent: u8,
    job: &ScheduledJob,
    run_marker: u64,
) -> Result<u64> {
    let span = delay
        .checked_mul(u64::from(jitter_percent))
        .ok_or(Error::Scheduler("jitter range overflow"))?
        / 100;
    if span == 0 {
        return Ok(delay);
    }
    let mut input = Vec::with_capacity(24);
    input.extend_from_slice(&job.job_id);
    input.extend_from_slice(&run_marker.to_be_bytes());
    let hash = prefixed_hash(SCHEDULER_JITTER_TYPE_ID, &input);
    let sample = u64::from_be_bytes(hash[..8].try_into().expect("eight-byte hash prefix"));
    delay
        .checked_add(sample % (span + 1))
        .ok_or(Error::Scheduler("jittered delay overflow"))
}

fn bounded_error(mut error: String) -> String {
    if error.is_empty() {
        return "scheduled job failed without an error message".to_owned();
    }
    if error.len() <= 1024 {
        return error;
    }
    let mut boundary = 1024;
    while !error.is_char_boundary(boundary) {
        boundary -= 1;
    }
    error.truncate(boundary);
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_verify::verify_public_host;

    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );

    fn scheduler() -> (tempfile::TempDir, FoksScheduler, Vec<u8>) {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("hard.sqlite");
        let host = verify_public_host("foks.app", PROBE).unwrap();
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&host.snapshot)
            .unwrap();
        let scheduler = FoksScheduler::new(
            &database,
            SchedulerConfig {
                lease_micros: 100,
                base_backoff_micros: 10,
                max_backoff_micros: 80,
                jitter_percent: 0,
                claim_limit: 4,
            },
        )
        .unwrap();
        (directory, scheduler, host.snapshot.host_id().to_vec())
    }

    #[test]
    fn failures_back_off_durably_and_success_restores_interval() {
        let (_directory, scheduler, host_id) = scheduler();
        let job_id = [3; 16];
        scheduler
            .register(ScheduledJobRegistration {
                job_id,
                kind: ScheduledJobKind::UserRefresh,
                host_id,
                scope_id: vec![4; 33],
                interval_micros: 1_000,
                first_run_at: 50,
                registered_at: 40,
            })
            .unwrap();

        let failed = scheduler
            .run_due(50, |_| Err("offline".to_owned()))
            .unwrap();
        assert_eq!(failed.runs[0].next_run_at, 60);
        assert!(matches!(
            failed.runs[0].status,
            ScheduledRunStatus::Failed { ref error } if error == "offline"
        ));
        assert!(scheduler.run_due(59, |_| Ok(())).unwrap().runs.is_empty());

        let completed = scheduler.run_due(60, |_| Ok(())).unwrap();
        assert_eq!(completed.runs[0].next_run_at, 1_060);
        let stored = HardStateStore::open(&scheduler.hard_database)
            .unwrap()
            .scheduled_job(&job_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.failure_count, 0);
        assert_eq!(stored.last_completed_at, Some(60));
        assert_eq!(stored.last_error, None);
    }

    #[test]
    fn abandoned_leases_are_recovered_without_concurrent_duplicate_runs() {
        let (_directory, scheduler, host_id) = scheduler();
        let job_id = [5; 16];
        scheduler
            .register(ScheduledJobRegistration {
                job_id,
                kind: ScheduledJobKind::MutationReconcile,
                host_id,
                scope_id: vec![6; 16],
                interval_micros: 1_000,
                first_run_at: 10,
                registered_at: 10,
            })
            .unwrap();
        let claimed = HardStateStore::open(&scheduler.hard_database)
            .unwrap()
            .claim_due_scheduled_jobs(10, 110, 1)
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(scheduler.run_due(109, |_| Ok(())).unwrap().runs.is_empty());
        assert_eq!(scheduler.run_due(110, |_| Ok(())).unwrap().runs.len(), 1);
    }

    #[test]
    fn error_messages_are_utf8_safely_bounded_for_public_storage() {
        let error = bounded_error("é".repeat(600));
        assert!(error.len() <= 1024);
        assert!(error.is_char_boundary(error.len()));
    }

    #[test]
    fn default_team_refresh_registration_preserves_operator_configuration() {
        let (_directory, scheduler, host_id) = scheduler();
        let registration = ScheduledJobRegistration {
            job_id: [9; 16],
            kind: ScheduledJobKind::TeamRefresh,
            host_id,
            scope_id: vec![8; 33],
            interval_micros: 17 * 60 * 1_000_000,
            first_run_at: 1_020,
            registered_at: 1,
        };
        assert!(scheduler.register_if_missing(registration.clone()).unwrap());
        let mut later = registration.clone();
        later.interval_micros = 1;
        later.first_run_at = 2;
        assert!(!scheduler.register_if_missing(later).unwrap());
        let stored = HardStateStore::open(&scheduler.hard_database)
            .unwrap()
            .scheduled_job(&registration.job_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.kind, ScheduledJobKind::TeamRefresh);
        assert_eq!(stored.interval_micros, registration.interval_micros);
        assert_eq!(stored.next_run_at, registration.first_run_at);
    }

    #[test]
    fn security_refresh_failures_never_back_off_past_their_cadence() {
        let (_directory, mut scheduler, host_id) = scheduler();
        scheduler.config.jitter_percent = 100;
        let job_id = [10; 16];
        scheduler
            .register(ScheduledJobRegistration {
                job_id,
                kind: ScheduledJobKind::TeamRefresh,
                host_id,
                scope_id: vec![11; 33],
                interval_micros: 30,
                first_run_at: 10,
                registered_at: 1,
            })
            .unwrap();
        let mut now = 10;
        for _ in 0..8 {
            let failed = scheduler
                .run_due(now, |_| Err("one stored team is inaccessible".to_owned()))
                .unwrap();
            assert_eq!(failed.runs.len(), 1);
            assert!(failed.runs[0].next_run_at - now <= 30);
            now = failed.runs[0].next_run_at;
        }
    }
}
