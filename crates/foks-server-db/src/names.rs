use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result};

impl Database {
    pub fn reserve_name(
        &mut self,
        normalized_name: &[u8],
        token: &[u8; 17],
        sequence: u64,
        now: u64,
        expires_at: u64,
    ) -> Result<()> {
        if normalized_name.is_empty()
            || normalized_name.len() > self.config.maximum_name_bytes
            || expires_at <= now
            || sequence == 0
        {
            return Err(Error::Invalid("name reservation"));
        }
        let now = sql_integer(now)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(Option<Vec<u8>>, Option<i64>)> = transaction
            .query_row(
                "SELECT uid, expires_at FROM names WHERE normalized_name = ?1",
                [normalized_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if existing
            .is_some_and(|(uid, expiry)| uid.is_some() || expiry.is_none_or(|value| value > now))
        {
            return Err(Error::NameInUse);
        }
        transaction.execute(
            "INSERT INTO names(normalized_name, reservation_token, reservation_sequence, expires_at, uid)
             VALUES (?1, ?2, ?3, ?4, NULL)
             ON CONFLICT(normalized_name) DO UPDATE SET
                reservation_token = excluded.reservation_token,
                reservation_sequence = excluded.reservation_sequence,
                expires_at = excluded.expires_at,
                uid = NULL",
            params![normalized_name, token, sql_integer(sequence)?, sql_integer(expires_at)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn cleanup_expired_reservations(&mut self, now: u64, maximum: usize) -> Result<usize> {
        let maximum = i64::try_from(maximum).map_err(|_| Error::IntegerRange)?;
        Ok(self.connection.execute(
            "DELETE FROM names WHERE normalized_name IN (
                SELECT normalized_name FROM names
                WHERE uid IS NULL AND expires_at <= ?1 ORDER BY expires_at LIMIT ?2
             )",
            params![sql_integer(now)?, maximum],
        )?)
    }
}

impl ReadDatabase {
    pub fn uid_by_normalized_name(&self, normalized_name: &[u8]) -> Result<Option<Vec<u8>>> {
        uid_by_normalized_name(&self.connection, normalized_name)
    }

    pub fn device_self_token_matches(
        &self,
        uid: &[u8],
        device_id: &[u8],
        self_token: &[u8; 17],
    ) -> Result<bool> {
        device_self_token_matches(&self.connection, uid, device_id, self_token)
    }

    pub fn self_token_matches(&self, uid: &[u8], self_token: &[u8; 17]) -> Result<bool> {
        self_token_matches(&self.connection, uid, self_token)
    }
}

impl ReadSnapshot<'_> {
    pub fn uid_by_normalized_name(&self, normalized_name: &[u8]) -> Result<Option<Vec<u8>>> {
        uid_by_normalized_name(self.connection(), normalized_name)
    }

    pub fn device_self_token_matches(
        &self,
        uid: &[u8],
        device_id: &[u8],
        self_token: &[u8; 17],
    ) -> Result<bool> {
        device_self_token_matches(self.connection(), uid, device_id, self_token)
    }

    pub fn self_token_matches(&self, uid: &[u8], self_token: &[u8; 17]) -> Result<bool> {
        self_token_matches(self.connection(), uid, self_token)
    }
}

fn uid_by_normalized_name(
    connection: &rusqlite::Connection,
    normalized_name: &[u8],
) -> Result<Option<Vec<u8>>> {
    Ok(connection
        .query_row(
            "SELECT uid FROM names WHERE normalized_name = ?1 AND uid IS NOT NULL",
            [normalized_name],
            |row| row.get(0),
        )
        .optional()?)
}

fn device_self_token_matches(
    connection: &rusqlite::Connection,
    uid: &[u8],
    device_id: &[u8],
    self_token: &[u8; 17],
) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM devices
             WHERE uid = ?1 AND device_id = ?2 AND self_token = ?3",
            params![uid, device_id, self_token],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn self_token_matches(
    connection: &rusqlite::Connection,
    uid: &[u8],
    self_token: &[u8; 17],
) -> Result<bool> {
    // Go retains self-view tokens after revocation so the revoked device can
    // still reload and verify the public chain that revoked it.
    Ok(connection
        .query_row(
            "SELECT 1 FROM devices WHERE uid = ?1 AND self_token = ?2",
            params![uid, self_token],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
