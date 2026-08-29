use rusqlite::{params, TransactionBehavior};

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
        let reservations = transaction.execute(
            "DELETE FROM names WHERE uid IS NULL AND expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let team_reservations = transaction.execute(
            "DELETE FROM team_names WHERE team_id IS NULL AND expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let receipts = transaction.execute(
            "DELETE FROM request_receipts WHERE expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let challenges = transaction.execute(
            "DELETE FROM recovery_challenges WHERE expires_at <= ?1 OR consumed = 1",
            [sql_integer(now)?],
        )?;
        let team_view_tokens = transaction.execute(
            "DELETE FROM team_view_tokens WHERE expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let team_view_challenges = transaction.execute(
            "DELETE FROM team_view_challenges WHERE expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let team_admin_tokens = transaction.execute(
            "DELETE FROM team_admin_tokens WHERE expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let locks = transaction.execute(
            "DELETE FROM kv_locks WHERE expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let uploads = transaction.execute(
            "DELETE FROM kv_file_uploads
             WHERE updated_at <= ?1
               AND NOT EXISTS (
                   SELECT 1
                   FROM kv_dirent_heads h
                   JOIN kv_dirents d
                     ON d.uid = h.uid
                    AND d.parent_id = h.parent_id
                    AND d.dirent_id = h.dirent_id
                    AND d.version = h.version
                   WHERE d.uid = kv_file_uploads.uid
                     AND substr(d.node_id, 1, 1) = X'02'
                     AND substr(d.node_id, 2, 16) = kv_file_uploads.file_id
               )",
            params![sql_integer(abandon_uploads_before)?],
        )?;
        let federation_user_permissions = transaction.execute(
            "DELETE FROM federation_user_view_permissions WHERE expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let federation_team_permissions = transaction.execute(
            "DELETE FROM federation_team_view_permissions WHERE expires_at <= ?1",
            [sql_integer(now)?],
        )?;
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
            uploads: u64::try_from(uploads).map_err(|_| Error::IntegerRange)?,
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
