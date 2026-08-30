use rusqlite::{params, OptionalExtension as _};

use crate::{Database, Error, Result};

pub const MAXIMUM_LOG_SEND_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAXIMUM_LOG_SEND_FILES: u64 = 16;
pub const MAXIMUM_LOG_SEND_BLOCKS: u64 = 16;
pub const MAXIMUM_LOG_SEND_BLOCK_BYTES: usize = 4 * 1024 * 1024;
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
    pub fn join_waitlist(&mut self, id: &[u8; 13], email: &str, now: u64) -> Result<()> {
        if id[0] != 1 || !valid_email(email) {
            return Err(Error::Invalid("waitlist entry"));
        }
        self.connection
            .execute(
                "INSERT INTO waitlist_entries(waitlist_id, email, created_at)
                 VALUES (?1, ?2, ?3)",
                params![id, email, integer(now)?],
            )
            .map_err(|error| map_duplicate(error, "waitlist entry"))?;
        Ok(())
    }

    pub fn begin_log_send(&mut self, id: &[u8; 17], uid: Option<&[u8]>, now: u64) -> Result<()> {
        if id[0] != 48 || uid.is_some_and(|uid| uid.len() != 33) {
            return Err(Error::Invalid("log-send identity"));
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
        let Some((content_length, block_count)) = metadata else {
            return Err(Error::NotFound("active log-send file"));
        };
        let content_length = crate::error::unsigned(content_length)?;
        let block_count = crate::error::unsigned(block_count)?;
        let expected_length =
            expected_block_length(content_length, block_count, mutation.block_number);
        if mutation.block_number >= block_count
            || (block_count == 0) != (content_length == 0)
            || expected_length != Some(mutation.block.len())
        {
            return Err(Error::Invalid("log-send block position"));
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
        || mutation.content_length > MAXIMUM_LOG_SEND_FILE_BYTES
        || mutation.block_count > MAXIMUM_LOG_SEND_BLOCKS
        || (mutation.block_count == 0) != (mutation.content_length == 0)
        || mutation.block_count
            != mutation
                .content_length
                .div_ceil(MAXIMUM_LOG_SEND_BLOCK_BYTES as u64)
    {
        return Err(Error::Invalid("log-send file metadata"));
    }
    Ok(())
}

fn expected_block_length(
    content_length: u64,
    block_count: u64,
    block_number: u64,
) -> Option<usize> {
    if block_number >= block_count {
        return None;
    }
    if block_number + 1 < block_count {
        return Some(MAXIMUM_LOG_SEND_BLOCK_BYTES);
    }
    let preceding = block_number.checked_mul(MAXIMUM_LOG_SEND_BLOCK_BYTES as u64)?;
    usize::try_from(content_length.checked_sub(preceding)?).ok()
}

fn valid_email(email: &str) -> bool {
    email.len() <= 320
        && email.len() >= 3
        && !email.chars().any(char::is_whitespace)
        && !email.chars().any(char::is_control)
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
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
