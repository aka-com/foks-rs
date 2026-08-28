use rusqlite::{params, OptionalExtension};

use crate::*;

impl HardStateStore {
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
    pub fn claim_due_scheduled_jobs(
        &mut self,
        now: u64,
        lease_until: u64,
        limit: u64,
    ) -> Result<Vec<ScheduledJob>> {
        if limit == 0 || lease_until <= now {
            return Err(Error::InvalidScheduledJob("invalid claim bounds"));
        }
        let transaction = self.write_transaction()?;
        let now_sql = sqlite_integer("scheduled claim time", now)?;
        let lease_sql = sqlite_integer("scheduled lease time", lease_until)?;
        let limit_sql = sqlite_integer("scheduled claim limit", limit)?;
        let ids = {
            let mut statement = transaction.prepare(
                "SELECT job_id FROM scheduled_jobs
                 WHERE next_run_at <= ?1 AND (lease_until IS NULL OR lease_until <= ?1)
                 ORDER BY next_run_at, job_id LIMIT ?2",
            )?;
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
        Ok(jobs)
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
            return Err(Error::InvalidScheduledJob("invalid scheduled job error"));
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
