use std::cell::Cell;

use rusqlite::{params, OptionalExtension};

use crate::*;

thread_local! {
    /// Set while this thread runs work that reserved one profile only.
    static SINGLE_PROFILE_CLAIM: Cell<bool> = const { Cell::new(false) };
    /// Set by a restricted claim that left a due job for a wider caller.
    static DEFERRED_WIDE_JOB: Cell<bool> = const { Cell::new(false) };
    /// Counts the claims that observed the restriction on this thread. Only
    /// ever incremented, so a nested scope can compare it across its own work
    /// without disturbing an enclosing one.
    static RESTRICTED_CLAIMS: Cell<u64> = const { Cell::new(0) };
}

/// Runs `work` with every [`HardStateStore::claim_due_scheduled_jobs`] call on
/// this thread restricted to jobs whose work cannot leave one profile, and
/// reports whether such a claim left a wider job unclaimed.
///
/// A caller that reserved one profile's admission cannot run a job that reaches
/// a second profile: the second profile's work would proceed outside anything
/// the caller reserved. The restriction is enforced in the claim itself, not by
/// the caller inspecting what is due first, because the durable state can gain
/// a wider job between any such inspection and the claim.
///
/// A deferred job is left exactly as it was found: still due, still unleased,
/// with its failure count and next run time untouched. Deferring therefore
/// costs the job nothing, and the caller answers a `true` flag by re-running
/// the same work under admission wide enough for the job it skipped.
///
/// The restriction is thread-scoped because the claim is issued inside the
/// scheduler the caller drives rather than by the caller. It is restored on
/// unwind, and a nested call cannot lift an outer restriction: an inner
/// deferral is reported to the inner caller and propagated to the outer one.
///
/// Thread scoping is sound only while the claim runs on the thread that set
/// it, which is a property of a caller this module cannot see. So it is not
/// assumed: this counts the claims that actually observed the restriction, and
/// reports a deferral when `work` finished without one. A claim that moved to
/// another thread would otherwise run unrestricted while its caller holds one
/// profile's admission, which is the case the restriction exists to prevent.
/// Reporting a deferral instead costs one re-run under wider admission, which
/// is what every caller did before the restriction existed.
pub fn with_single_profile_claims<T>(work: impl FnOnce() -> T) -> (T, bool) {
    struct Restore(bool, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SINGLE_PROFILE_CLAIM.with(|slot| slot.set(self.0));
            DEFERRED_WIDE_JOB.with(|slot| slot.set(self.1 | slot.get()));
        }
    }
    let _restore = Restore(
        SINGLE_PROFILE_CLAIM.with(|slot| slot.replace(true)),
        DEFERRED_WIDE_JOB.with(|slot| slot.replace(false)),
    );
    let claimed_before = RESTRICTED_CLAIMS.with(Cell::get);
    let value = work();
    let observed = RESTRICTED_CLAIMS.with(Cell::get) != claimed_before;
    (value, !observed || DEFERRED_WIDE_JOB.with(Cell::get))
}

/// The SQL job-kind values a single-profile claim must skip, derived from
/// [`ScheduledJobKind::reaches_other_profiles`] so the predicate cannot drift
/// from the classification. `None` when no kind reaches another profile, which
/// leaves the claim unrestricted rather than emitting an empty `IN ()`.
fn cross_profile_kinds() -> Option<String> {
    let list = ScheduledJobKind::ALL
        .iter()
        .filter(|kind| kind.reaches_other_profiles())
        .map(|kind| (*kind as u8).to_string())
        .collect::<Vec<_>>();
    (!list.is_empty()).then(|| list.join(", "))
}

impl HardStateStore {
    /// Registers a default job only when its durable identity is absent.
    /// Concurrent explicit registration wins without having its interval or
    /// execution state replaced.
    pub fn register_scheduled_job_if_missing(&mut self, job: &ScheduledJob) -> Result<bool> {
        validate_scheduled_job(job)?;
        if job.failure_count != 0
            || job.lease_until.is_some()
            || job.last_completed_at.is_some()
            || job.last_error.is_some()
        {
            return Err(Error::InvalidScheduledJob(
                "new jobs cannot contain execution state",
            ));
        }
        let transaction = self.write_transaction()?;
        let host_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&job.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        let changed = transaction.execute(
            "INSERT INTO scheduled_jobs (
                job_id, job_kind, host_id, scope_id, interval_micros,
                next_run_at, failure_count, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
             ON CONFLICT(job_id) DO NOTHING",
            params![
                job.job_id.as_slice(),
                job.kind as u8,
                job.host_id,
                job.scope_id,
                sqlite_integer("scheduled interval", job.interval_micros)?,
                sqlite_integer("scheduled next run", job.next_run_at)?,
                sqlite_integer("scheduled update time", job.updated_at)?,
            ],
        )?;
        if changed == 0 {
            let binding = transaction.query_row(
                "SELECT job_kind, host_id, scope_id FROM scheduled_jobs WHERE job_id = ?1",
                [job.job_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )?;
            if binding != (job.kind as i64, job.host_id.clone(), job.scope_id.clone()) {
                return Err(Error::InvalidScheduledJob("job ID binding changed"));
            }
        }
        transaction.commit()?;
        Ok(changed == 1)
    }

    /// Registers a resumable job, or refreshes its interval without delaying
    /// work that was already due. A job ID can never be rebound to another
    /// host, scope, or kind.
    pub fn register_scheduled_job(&mut self, job: &ScheduledJob) -> Result<()> {
        validate_scheduled_job(job)?;
        if job.failure_count != 0
            || job.lease_until.is_some()
            || job.last_completed_at.is_some()
            || job.last_error.is_some()
        {
            return Err(Error::InvalidScheduledJob(
                "new jobs cannot contain execution state",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&job.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        let existing = self
            .connection
            .query_row(
                "SELECT job_kind, host_id, scope_id FROM scheduled_jobs WHERE job_id = ?1",
                [job.job_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        if existing.is_some_and(|(kind, host_id, scope_id)| {
            kind != job.kind as i64 || host_id != job.host_id || scope_id != job.scope_id
        }) {
            return Err(Error::InvalidScheduledJob("job ID binding changed"));
        }
        self.connection.execute(
            "INSERT INTO scheduled_jobs (
                job_id, job_kind, host_id, scope_id, interval_micros,
                next_run_at, failure_count, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
             ON CONFLICT(job_id) DO UPDATE SET
                interval_micros = excluded.interval_micros,
                next_run_at = MIN(scheduled_jobs.next_run_at, excluded.next_run_at),
                updated_at = MAX(scheduled_jobs.updated_at, excluded.updated_at)",
            params![
                job.job_id.as_slice(),
                job.kind as u8,
                job.host_id,
                job.scope_id,
                sqlite_integer("scheduled interval", job.interval_micros)?,
                sqlite_integer("scheduled next run", job.next_run_at)?,
                sqlite_integer("scheduled update time", job.updated_at)?,
            ],
        )?;
        Ok(())
    }

    /// Claims due jobs under an expiring lease. An abandoned claim becomes
    /// runnable again after `lease_until`, so only idempotent work belongs in
    /// this scheduler.
    ///
    /// Under [`with_single_profile_claims`] the claim skips any due job whose
    /// kind reaches another profile and records that it did; the skipped job
    /// keeps its schedule, its lease and its failure count.
    pub fn claim_due_scheduled_jobs(
        &mut self,
        now: u64,
        lease_until: u64,
        limit: u64,
    ) -> Result<Vec<ScheduledJob>> {
        if limit == 0 || lease_until <= now {
            return Err(Error::InvalidScheduledJob(
                "claim limit must be non-zero and lease expiration must be in the future",
            ));
        }
        // A skipped kind must be excluded in the selection rather than filtered
        // out of its result: filtering would let a wider job consume the claim
        // limit and starve the narrow jobs this caller may still run.
        let restricted = SINGLE_PROFILE_CLAIM.with(Cell::get);
        if restricted {
            // Recorded before any fallible step, because the question this
            // answers is whether the claim reached the thread that restricted
            // it, not whether the claim then succeeded.
            RESTRICTED_CLAIMS.with(|slot| slot.set(slot.get().wrapping_add(1)));
        }
        let skipped = restricted.then(cross_profile_kinds).flatten();
        let transaction = self.write_transaction()?;
        let now_sql = sqlite_integer("scheduled claim time", now)?;
        let lease_sql = sqlite_integer("scheduled lease time", lease_until)?;
        let limit_sql = sqlite_integer("scheduled claim limit", limit)?;
        let deferred = match &skipped {
            // The due-and-unleased predicate is the one the selection below
            // applies, so this is exactly "a job this claim could have taken
            // had it not been restricted".
            Some(kinds) => transaction.query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM scheduled_jobs
                     WHERE next_run_at <= ?1 AND (lease_until IS NULL OR lease_until <= ?1)
                       AND job_kind IN ({kinds}))"
                ),
                [now_sql],
                |row| row.get::<_, bool>(0),
            )?,
            None => false,
        };
        let ids = {
            let mut statement = transaction.prepare(&format!(
                "SELECT job_id FROM scheduled_jobs
                 WHERE next_run_at <= ?1 AND (lease_until IS NULL OR lease_until <= ?1){}
                 ORDER BY next_run_at, job_id LIMIT ?2",
                match &skipped {
                    Some(kinds) => format!(" AND job_kind NOT IN ({kinds})"),
                    None => String::new(),
                }
            ))?;
            let ids = statement
                .query_map(params![now_sql, limit_sql], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ids
        };
        for id in &ids {
            transaction.execute(
                "UPDATE scheduled_jobs SET lease_until = ?2, updated_at = ?3 WHERE job_id = ?1",
                params![id, lease_sql, now_sql],
            )?;
        }
        let mut jobs = Vec::with_capacity(ids.len());
        for id in ids {
            jobs.push(transaction.query_row(
                "SELECT job_kind, host_id, scope_id, interval_micros, next_run_at,
                        failure_count, lease_until, last_completed_at, last_error, updated_at
                 FROM scheduled_jobs WHERE job_id = ?1",
                [id.as_slice()],
                |row| scheduled_job_from_row(&id, row),
            )?);
        }
        transaction.commit()?;
        // Recorded only once the claim itself is durable: a failed commit
        // leaves the caller with an error rather than a deferral to answer.
        if deferred {
            DEFERRED_WIDE_JOB.with(|slot| slot.set(true));
        }
        Ok(jobs)
    }

    /// The job the next claim would take, read without claiming it and without
    /// taking any lock: the same selection [`Self::claim_due_scheduled_jobs`]
    /// makes, so a caller can read the kind it is about to run before
    /// reserving for it.
    ///
    /// Only the claim is authoritative. The state can gain a due job between
    /// this read and the claim, so a caller that reserved for what it read
    /// here must still be able to answer a claim that lands elsewhere.
    pub fn next_due_scheduled_job(&self, now: u64) -> Result<Option<ScheduledJob>> {
        let now_sql = sqlite_integer("scheduled claim time", now)?;
        let Some(id) = self
            .connection
            .query_row(
                "SELECT job_id FROM scheduled_jobs
                 WHERE next_run_at <= ?1 AND (lease_until IS NULL OR lease_until <= ?1)
                 ORDER BY next_run_at, job_id LIMIT 1",
                [now_sql],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
        else {
            return Ok(None);
        };
        self.connection
            .query_row(
                "SELECT job_kind, host_id, scope_id, interval_micros, next_run_at,
                        failure_count, lease_until, last_completed_at, last_error, updated_at
                 FROM scheduled_jobs WHERE job_id = ?1",
                [id.as_slice()],
                |row| scheduled_job_from_row(&id, row),
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn complete_scheduled_job(
        &mut self,
        job_id: &[u8; 16],
        claimed_until: u64,
        next_run_at: u64,
        completed_at: u64,
    ) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE scheduled_jobs SET
                next_run_at = ?3, failure_count = 0, lease_until = NULL,
                last_completed_at = ?4, last_error = NULL, updated_at = ?4
             WHERE job_id = ?1 AND lease_until = ?2",
            params![
                job_id.as_slice(),
                sqlite_integer("scheduled claimed lease", claimed_until)?,
                sqlite_integer("scheduled next run", next_run_at)?,
                sqlite_integer("scheduled completion time", completed_at)?,
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidScheduledJob("scheduled job lease was lost"));
        }
        Ok(())
    }

    pub fn fail_scheduled_job(
        &mut self,
        job_id: &[u8; 16],
        claimed_until: u64,
        next_run_at: u64,
        failed_at: u64,
        error: &str,
    ) -> Result<()> {
        if error.is_empty() || error.len() > 1024 {
            return Err(Error::InvalidScheduledJob(
                "job error message must be non-empty and at most 1024 bytes",
            ));
        }
        let changed = self.connection.execute(
            "UPDATE scheduled_jobs SET
                next_run_at = ?3, failure_count = failure_count + 1,
                lease_until = NULL, last_error = ?5, updated_at = ?4
             WHERE job_id = ?1 AND lease_until = ?2",
            params![
                job_id.as_slice(),
                sqlite_integer("scheduled claimed lease", claimed_until)?,
                sqlite_integer("scheduled next run", next_run_at)?,
                sqlite_integer("scheduled failure time", failed_at)?,
                error,
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidScheduledJob("scheduled job lease was lost"));
        }
        Ok(())
    }

    pub fn scheduled_job(&self, job_id: &[u8; 16]) -> Result<Option<ScheduledJob>> {
        self.connection
            .query_row(
                "SELECT job_kind, host_id, scope_id, interval_micros, next_run_at,
                        failure_count, lease_until, last_completed_at, last_error, updated_at
                 FROM scheduled_jobs WHERE job_id = ?1",
                [job_id.as_slice()],
                |row| scheduled_job_from_row(job_id, row),
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn next_scheduled_run(&self) -> Result<Option<u64>> {
        let value = self.connection.query_row(
            "SELECT MIN(MAX(next_run_at, COALESCE(lease_until, next_run_at)))
             FROM scheduled_jobs",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        value
            .map(|value| stored_unsigned("next scheduled run", value))
            .transpose()
    }

    pub fn remove_scheduled_job(&mut self, job_id: &[u8; 16]) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM scheduled_jobs WHERE job_id = ?1",
            [job_id.as_slice()],
        )? == 1)
    }
}
