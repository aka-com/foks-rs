use rusqlite::{params, Connection, OptionalExtension as _};

use crate::{error::sql_integer, Database, Error, ReadDatabase, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyRecord {
    pub response: Vec<u8>,
    pub created_at: u64,
    pub expires_at: u64,
}

impl Database {
    pub fn idempotency_record(
        &self,
        idempotency_key: &[u8],
        request_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<IdempotencyRecord>> {
        lookup(&self.connection, idempotency_key, request_hash, now)
    }
}

impl ReadDatabase {
    pub fn idempotency_record(
        &self,
        idempotency_key: &[u8],
        request_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<IdempotencyRecord>> {
        lookup(&self.connection, idempotency_key, request_hash, now)
    }
}

pub(crate) fn lookup(
    connection: &Connection,
    idempotency_key: &[u8],
    request_hash: &[u8; 32],
    now: u64,
) -> Result<Option<IdempotencyRecord>> {
    let row: Option<(Vec<u8>, Vec<u8>, i64, i64)> = connection
        .query_row(
            "SELECT request_hash, response_blob, created_at, expires_at
             FROM request_receipts WHERE idempotency_key = ?1",
            [idempotency_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((stored_hash, response, created_at, expires_at)) = row else {
        return Ok(None);
    };
    if expires_at <= sql_integer(now)? {
        return Err(Error::OperationExpired);
    }
    if stored_hash.as_slice() != request_hash {
        return Err(Error::OperationConflict);
    }
    Ok(Some(IdempotencyRecord {
        response,
        created_at: crate::error::unsigned(created_at)?,
        expires_at: crate::error::unsigned(expires_at)?,
    }))
}

pub(crate) fn insert(
    connection: &Connection,
    idempotency_key: &[u8],
    request_hash: &[u8; 32],
    response: &[u8],
    created_at: u64,
    expires_at: u64,
) -> Result<()> {
    if !(16..=64).contains(&idempotency_key.len()) || expires_at <= created_at {
        return Err(Error::Invalid("idempotency record"));
    }
    connection.execute(
        "INSERT INTO request_receipts
         (idempotency_key, request_hash, response_blob, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            idempotency_key,
            request_hash,
            response,
            sql_integer(created_at)?,
            sql_integer(expires_at)?
        ],
    )?;
    Ok(())
}
