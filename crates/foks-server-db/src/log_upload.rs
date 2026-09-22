use rusqlite::{params, OptionalExtension as _};

use crate::{Database, Error, Result};

pub const MAXIMUM_LOG_SEND_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAXIMUM_LOG_SEND_FILES: u64 = 16;
pub const MAXIMUM_LOG_SEND_BLOCKS: u64 = 16;
pub const MAXIMUM_LOG_SEND_BLOCK_BYTES: usize = 4 * 1024 * 1024;
pub const MAXIMUM_ACTIVE_LOG_SEND_SESSIONS: u64 = 128;
pub const MAXIMUM_ACTIVE_LOG_SEND_BYTES: u64 = 512 * 1024 * 1024;
const LOG_SEND_SESSION_LIFETIME_MICROS: u64 = 24 * 60 * 60 * 1_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogSendFileMutation<'a> {
    pub log_send_id: &'a [u8; 17],
    pub file_id: u64,
    pub filename: &'a str,
    pub content_length: u64,
    pub block_count: u64,
    pub content_hash: &'a [u8; 32],
    pub now: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogSendBlockMutation<'a> {
    pub log_send_id: &'a [u8; 17],
    pub file_id: u64,
    pub block_number: u64,
    pub block: &'a [u8],
    pub now: u64,
}

impl Database {
    pub fn begin_log_send(&mut self, id: &[u8; 17], uid: Option<&[u8]>, now: u64) -> Result<()> {
        if id[0] != 48 || uid.is_some_and(|uid| uid.len() != 33) {
            return Err(Error::Invalid("log-send identity"));
        }
        let oldest_active = integer(now.saturating_sub(LOG_SEND_SESSION_LIFETIME_MICROS))?;
        self.connection.execute(
            "DELETE FROM log_sends WHERE created_at < ?1",
            [oldest_active],
        )?;
        let active: i64 = self.connection.query_row(
            "SELECT count(*) FROM log_sends WHERE created_at >= ?1",
            [oldest_active],
            |row| row.get(0),
        )?;
        if crate::error::unsigned(active)? >= MAXIMUM_ACTIVE_LOG_SEND_SESSIONS {
            return Err(Error::Capacity("active log-send sessions"));
        }
        self.connection
            .execute(
                "INSERT INTO log_sends(log_send_id, uid, created_at) VALUES (?1, ?2, ?3)",
                params![id, uid, integer(now)?],
            )
            .map_err(|error| map_duplicate(error, "log-send session"))?;
        Ok(())
    }

    pub fn begin_log_send_file(&mut self, mutation: &LogSendFileMutation<'_>) -> Result<()> {
        validate_file(mutation)?;
        let oldest_active = mutation
            .now
            .saturating_sub(LOG_SEND_SESSION_LIFETIME_MICROS);
        let oldest_active = integer(oldest_active)?;
        let session_created = self
            .connection
            .query_row(
                "SELECT created_at FROM log_sends WHERE log_send_id = ?1",
                params![mutation.log_send_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if session_created.is_none_or(|created| created < oldest_active) {
            return Err(Error::NotFound("active log-send session"));
        }
        let file_exists: bool = self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM log_send_files WHERE log_send_id = ?1 AND file_id = ?2
             )",
            params![mutation.log_send_id, integer(mutation.file_id)?],
            |row| row.get(0),
        )?;
        if file_exists {
            return Err(Error::Duplicate("log-send file"));
        }
        let file_count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM log_send_files WHERE log_send_id = ?1",
            params![mutation.log_send_id],
            |row| row.get(0),
        )?;
        if crate::error::unsigned(file_count)? >= MAXIMUM_LOG_SEND_FILES {
            return Err(Error::Capacity("log-send file count"));
        }
        let inserted = self
            .connection
            .execute(
                "INSERT INTO log_send_files(
                     log_send_id, file_id, filename, content_length, block_count,
                     content_hash, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    mutation.log_send_id,
                    integer(mutation.file_id)?,
                    mutation.filename,
                    integer(mutation.content_length)?,
                    integer(mutation.block_count)?,
                    mutation.content_hash,
                    integer(mutation.now)?,
                ],
            )
            .map_err(|error| map_duplicate(error, "log-send file"))?;
        if inserted != 1 {
            return Err(Error::Invalid("log-send file insert"));
        }
        Ok(())
    }

    pub fn put_log_send_block(&mut self, mutation: &LogSendBlockMutation<'_>) -> Result<()> {
        if mutation.block.len() > MAXIMUM_LOG_SEND_BLOCK_BYTES {
            return Err(Error::Capacity("log-send block"));
        }
        let metadata = self
            .connection
            .query_row(
                "SELECT f.content_length, f.block_count
                 FROM log_send_files AS f
                 JOIN log_sends AS s ON s.log_send_id = f.log_send_id
                 WHERE f.log_send_id = ?1 AND f.file_id = ?2
                   AND s.created_at >= ?3",
                params![
                    mutation.log_send_id,
                    integer(mutation.file_id)?,
                    integer(
                        mutation
                            .now
                            .saturating_sub(LOG_SEND_SESSION_LIFETIME_MICROS),
                    )?,
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((_content_length, block_count)) = metadata else {
            return Err(Error::NotFound("active log-send file"));
        };
        let block_count = crate::error::unsigned(block_count)?;
        if mutation.block_number >= block_count
            || mutation.block.is_empty()
            || (mutation.block_number + 1 < block_count
                && mutation.block.len() != MAXIMUM_LOG_SEND_BLOCK_BYTES)
        {
            return Err(Error::Invalid("log-send block position"));
        }
        let (file_bytes, active_bytes): (i64, i64) = self.connection.query_row(
            "SELECT
                 COALESCE((SELECT sum(length(block)) FROM log_send_blocks
                           WHERE log_send_id = ?1 AND file_id = ?2), 0),
                 COALESCE((SELECT sum(length(b.block))
                           FROM log_send_blocks b
                           JOIN log_sends s ON s.log_send_id = b.log_send_id
                           WHERE s.created_at >= ?3), 0)",
            params![
                mutation.log_send_id,
                integer(mutation.file_id)?,
                integer(
                    mutation
                        .now
                        .saturating_sub(LOG_SEND_SESSION_LIFETIME_MICROS)
                )?,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let block_bytes = u64::try_from(mutation.block.len()).map_err(|_| Error::IntegerRange)?;
        if crate::error::unsigned(file_bytes)?
            .checked_add(block_bytes)
            .is_none_or(|bytes| bytes > MAXIMUM_LOG_SEND_FILE_BYTES)
            || crate::error::unsigned(active_bytes)?
                .checked_add(block_bytes)
                .is_none_or(|bytes| bytes > MAXIMUM_ACTIVE_LOG_SEND_BYTES)
        {
            return Err(Error::Capacity("log-send bytes"));
        }
        self.connection
            .execute(
                "INSERT INTO log_send_blocks(
                     log_send_id, file_id, block_number, block, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    mutation.log_send_id,
                    integer(mutation.file_id)?,
                    integer(mutation.block_number)?,
                    mutation.block,
                    integer(mutation.now)?,
                ],
            )
            .map_err(|error| map_duplicate(error, "log-send block"))?;
        Ok(())
    }
}

fn validate_file(mutation: &LogSendFileMutation<'_>) -> Result<()> {
    if mutation.log_send_id[0] != 48
        || mutation.filename.is_empty()
        || mutation.filename.len() > 255
        || mutation.filename.contains(['/', '\\', '\0'])
        || mutation.block_count > MAXIMUM_LOG_SEND_BLOCKS
    {
        return Err(Error::Invalid("log-send file metadata"));
    }
    Ok(())
}

fn integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::Invalid("integer exceeds SQLite range"))
}

fn map_duplicate(error: rusqlite::Error, subject: &'static str) -> Error {
    if matches!(
        &error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                || inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    ) {
        Error::Duplicate(subject)
    } else {
        Error::Sql(error)
    }
}
