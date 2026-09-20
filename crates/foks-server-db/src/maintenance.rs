use rusqlite::TransactionBehavior;

#[cfg(test)]
mod checkpoint_tests;
mod uploads;

use crate::{error::sql_integer, Database, Error, Result};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MaintenanceReport {
    pub challenges: u64,
    pub team_reservations: u64,
    pub team_view_challenges: u64,
    pub team_view_tokens: u64,
    pub team_admin_tokens: u64,
    pub reservations: u64,
    pub receipts: u64,
    pub locks: u64,
    pub uploads: u64,
    pub upload_candidates_examined: u64,
    pub uploads_marked_reclaiming: u64,
    pub upload_chunks: u64,
    pub upload_bytes: u64,
    pub upload_cleanup_deferred: bool,
    pub log_sends: u64,
    pub federation_user_permissions: u64,
    pub federation_team_permissions: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageReport {
    pub database_bytes: u64,
    pub wal_bytes: u64,
}

/// WAL frame positions, including frames backfilled by earlier checkpoints.
/// These are observations, not additive counts of work done by this call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointReport {
    /// SQLite's busy flag (0 or 1). Zero alone does not imply completion.
    pub busy: u64,
    /// Total WAL frames. Both counts are `None` when SQLite cannot observe them.
    pub wal_pages: Option<u64>,
    /// Frames already backfilled, at most `wal_pages` when counts are known.
    pub checkpointed_pages: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointOutcome {
    Complete,
    Deferred,
    Unavailable,
}

impl CheckpointReport {
    /// Completion requires known equal positions and no busy flag. An unavailable
    /// observation without contention proves neither completion nor backlog.
    pub fn outcome(&self) -> CheckpointOutcome {
        if self.busy != 0 {
            return CheckpointOutcome::Deferred;
        }
        match (self.wal_pages, self.checkpointed_pages) {
            (Some(log), Some(done)) if log == done => CheckpointOutcome::Complete,
            (Some(_), Some(_)) => CheckpointOutcome::Deferred,
            _ => CheckpointOutcome::Unavailable,
        }
    }

    fn decode(busy: i64, log: i64, done: i64) -> Result<Self> {
        if !matches!(busy, 0 | 1) {
            return Err(Error::Invalid("invalid checkpoint busy flag"));
        }
        let (wal_pages, checkpointed_pages) = match (log, done) {
            (-1, -1) => (None, None),
            (log, done) if log >= 0 && done >= 0 && done <= log => {
                (Some(log as u64), Some(done as u64))
            }
            _ => return Err(Error::Invalid("invalid checkpoint frame counts")),
        };
        Ok(Self {
            busy: busy as u64,
            wal_pages,
            checkpointed_pages,
        })
    }
}

impl Database {
    pub fn run_maintenance(
        &mut self,
        now: u64,
        abandon_uploads_before: u64,
    ) -> Result<MaintenanceReport> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM sso_sessions WHERE rowid IN (SELECT rowid FROM sso_sessions WHERE expires_at_ms<=?1 LIMIT 128)",
            [sql_integer(now / 1000)?],
        )?;
        let reservations = transaction.execute(
            "DELETE FROM names WHERE rowid IN (SELECT rowid FROM names WHERE uid IS NULL AND expires_at <= ?1 LIMIT 128)",
            [sql_integer(now)?],
        )?;
        let team_reservations = transaction.execute(
            "DELETE FROM team_names WHERE rowid IN (SELECT rowid FROM team_names WHERE team_id IS NULL AND expires_at <= ?1 LIMIT 128)",
            [sql_integer(now)?],
        )?;
        let receipts = transaction.execute(
            "DELETE FROM request_receipts WHERE rowid IN (SELECT rowid FROM request_receipts WHERE expires_at <= ?1 LIMIT 128)",
            [sql_integer(now)?],
        )?;
        let challenges = transaction.execute(
            "DELETE FROM recovery_challenges WHERE rowid IN (SELECT rowid FROM recovery_challenges WHERE expires_at <= ?1 OR consumed = 1 LIMIT 128)",
            [sql_integer(now)?],
        )?;
        let team_view_tokens = transaction.execute(
            "DELETE FROM team_view_tokens WHERE rowid IN (SELECT rowid FROM team_view_tokens WHERE expires_at <= ?1 LIMIT 128)",
            [sql_integer(now)?],
        )?;
        let team_view_challenges = transaction.execute(
            "DELETE FROM team_view_challenges WHERE rowid IN (SELECT rowid FROM team_view_challenges WHERE expires_at <= ?1 LIMIT 128)",
            [sql_integer(now)?],
        )?;
        let team_admin_tokens = transaction.execute(
            "DELETE FROM team_admin_tokens WHERE rowid IN (SELECT rowid FROM team_admin_tokens WHERE expires_at <= ?1 LIMIT 128)",
            [sql_integer(now)?],
        )?;
        // Lock expiry is evaluated against the timeout supplied by the next
        // acquirer, matching go-foks. There is no absolute expiry to reap.
        let locks = 0;
        let uploads = uploads::reclaim(&transaction, abandon_uploads_before)?;
        let log_sends = transaction.execute(
            "DELETE FROM log_sends WHERE rowid IN (SELECT rowid FROM log_sends WHERE created_at <= ?1 LIMIT 128)",
            [sql_integer(now.saturating_sub(24 * 60 * 60 * 1_000_000))?],
        )?;
        // Active federation rows remain renewable after bearer expiry, and revoked
        // rows are retained as tombstones. Neither category is removed during routine
        // expiration sweeps.
        let federation_user_permissions = 0;
        let federation_team_permissions = 0;
        transaction.commit()?;
        Ok(MaintenanceReport {
            challenges: u64::try_from(challenges).map_err(|_| Error::IntegerRange)?,
            team_reservations: u64::try_from(team_reservations).map_err(|_| Error::IntegerRange)?,
            team_view_challenges: u64::try_from(team_view_challenges)
                .map_err(|_| Error::IntegerRange)?,
            team_view_tokens: u64::try_from(team_view_tokens).map_err(|_| Error::IntegerRange)?,
            team_admin_tokens: u64::try_from(team_admin_tokens).map_err(|_| Error::IntegerRange)?,
            reservations: u64::try_from(reservations).map_err(|_| Error::IntegerRange)?,
            receipts: u64::try_from(receipts).map_err(|_| Error::IntegerRange)?,
            locks: u64::try_from(locks).map_err(|_| Error::IntegerRange)?,
            uploads: uploads.uploads,
            upload_candidates_examined: uploads.examined,
            uploads_marked_reclaiming: uploads.marked,
            upload_chunks: uploads.chunks,
            upload_bytes: uploads.bytes,
            upload_cleanup_deferred: uploads.deferred,
            log_sends: u64::try_from(log_sends).map_err(|_| Error::IntegerRange)?,
            federation_user_permissions: u64::try_from(federation_user_permissions)
                .map_err(|_| Error::IntegerRange)?,
            federation_team_permissions: u64::try_from(federation_team_permissions)
                .map_err(|_| Error::IntegerRange)?,
        })
    }

    /// Explicitly wait for readers through the busy handler and truncate the WAL.
    /// Online maintenance should use `checkpoint_passive` instead.
    pub fn checkpoint(&self) -> Result<CheckpointReport> {
        self.checkpoint_sql("PRAGMA main.wal_checkpoint(TRUNCATE)")
    }

    /// Backfill available frames without invoking SQLite's busy handler.
    /// May leave work pending and still performs I/O on the calling thread.
    pub fn checkpoint_passive(&self) -> Result<CheckpointReport> {
        self.checkpoint_sql("PRAGMA main.wal_checkpoint(PASSIVE)")
    }

    fn checkpoint_sql(&self, sql: &str) -> Result<CheckpointReport> {
        let (busy, log, done) = self
            .connection
            .query_row(sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        CheckpointReport::decode(busy, log, done)
    }

    pub fn storage_report(&self) -> Result<StorageReport> {
        let database_bytes = std::fs::metadata(&self.path)?.len();
        let mut wal_path = self.path.as_os_str().to_os_string();
        wal_path.push("-wal");
        let wal_path = std::path::PathBuf::from(wal_path);
        let wal_bytes = match std::fs::metadata(wal_path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        Ok(StorageReport {
            database_bytes,
            wal_bytes,
        })
    }
}
