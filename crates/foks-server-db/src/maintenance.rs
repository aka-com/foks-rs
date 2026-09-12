use rusqlite::TransactionBehavior;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointReport {
    pub busy: u64,
    pub wal_pages: u64,
    pub checkpointed_pages: u64,
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

    pub fn checkpoint(&self) -> Result<CheckpointReport> {
        let (busy, wal_pages, checkpointed_pages): (i64, i64, i64) =
            self.connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
        Ok(CheckpointReport {
            busy: u64::try_from(busy).map_err(|_| Error::IntegerRange)?,
            wal_pages: u64::try_from(wal_pages).map_err(|_| Error::IntegerRange)?,
            checkpointed_pages: u64::try_from(checkpointed_pages)
                .map_err(|_| Error::IntegerRange)?,
        })
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
