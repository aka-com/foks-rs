use crate::allocation::RowAllocationExt as _;
use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result};

impl Database {
    /// New username operations require an active interactive owner. Historical
    /// chain verification deliberately has a different, compatibility contract.
    pub fn require_username_change_signer(
        &self,
        uid: &[u8],
        credential: &[u8],
        signer: &[u8],
    ) -> Result<()> {
        require_username_change_owner_for_signer(&self.connection, uid, credential, Some(signer))
    }

    pub fn reserve_name_for_username_change(
        &mut self,
        uid: &[u8],
        credential: &[u8],
        normalized_name: &[u8],
        token: &[u8; 17],
        now: u64,
        expires_at: u64,
    ) -> Result<()> {
        if normalized_name.is_empty()
            || normalized_name.len() > self.config.maximum_name_bytes
            || expires_at <= now
        {
            return Err(Error::Invalid("name reservation"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_username_change_owner(&transaction, uid, credential)?;
        reserve_on(
            &transaction,
            normalized_name,
            token,
            1,
            sql_integer(now)?,
            expires_at,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn change_username_display(
        &mut self,
        uid: &[u8],
        credential: &[u8],
        expected_name: &[u8],
        expected_display: &[u8],
        display: &[u8],
    ) -> Result<()> {
        if display.is_empty() || display.len() > 4096 {
            return Err(Error::Invalid("username display"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_username_change_owner(&tx, uid, credential)?;
        if tx.execute("UPDATE users SET username_utf8=?1 WHERE uid=?2 AND normalized_name=?3 AND username_utf8=?4", params![display, uid, expected_name, expected_display])? != 1 {
            return Err(Error::StaleRoot);
        }
        tx.commit()?;
        Ok(())
    }

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
        reserve_on(
            &transaction,
            normalized_name,
            token,
            sequence,
            now,
            expires_at,
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
    pub fn require_username_change_owner(&self, uid: &[u8], credential: &[u8]) -> Result<()> {
        require_username_change_owner(&self.connection, uid, credential)
    }

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
            "SELECT uid FROM names WHERE normalized_name = ?1 AND uid IS NOT NULL AND dead = 0",
            [normalized_name],
            |row| row.budgeted_get(0),
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
    // Retain self-view tokens after revocation so a revoked device can
    // reload and verify the chain link that revoked it.
    Ok(connection
        .query_row(
            "SELECT 1 FROM devices WHERE uid = ?1 AND self_token = ?2",
            params![uid, self_token],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

pub(crate) fn reserve_on(
    connection: &rusqlite::Connection,
    normalized_name: &[u8],
    token: &[u8; 17],
    sequence: u64,
    now: i64,
    expires_at: u64,
) -> Result<()> {
    let existing: Option<(Option<Vec<u8>>, Option<i64>)> = connection
        .query_row(
            "SELECT uid, expires_at FROM names WHERE normalized_name = ?1",
            [normalized_name],
            |row| Ok((row.budgeted_get(0)?, row.budgeted_get(1)?)),
        )
        .optional()?;
    if existing.is_some_and(|(uid, expiry)| uid.is_some() || expiry.is_none_or(|value| value > now))
    {
        return Err(Error::NameInUse);
    }
    connection.execute(
            "INSERT INTO names(normalized_name, reservation_token, reservation_sequence, expires_at, uid)
             VALUES (?1, ?2, ?3, ?4, NULL)
             ON CONFLICT(normalized_name) DO UPDATE SET
                reservation_token = excluded.reservation_token,
                reservation_sequence = excluded.reservation_sequence,
                expires_at = excluded.expires_at,
                uid = NULL",
            params![normalized_name, token, sql_integer(sequence)?, sql_integer(expires_at)?],
        )?;
    Ok(())
}

/// The canonical signer may be a Yubi parent; TLS authenticates its delegated
/// subkey. Never authorize a bot/backup merely because it has OWNER role.
pub(crate) fn require_username_change_owner(
    connection: &rusqlite::Connection,
    uid: &[u8],
    credential: &[u8],
) -> Result<()> {
    require_username_change_owner_for_signer(connection, uid, credential, None)
}

pub(crate) fn require_username_change_owner_for_signer(
    connection: &rusqlite::Connection,
    uid: &[u8],
    credential: &[u8],
    signer: Option<&[u8]>,
) -> Result<()> {
    let kind = foks_proto::EntityId::from_bytes(credential.to_vec())
        .map_err(|_| Error::AuthorizationChanged)?
        .entity_type();
    if !matches!(
        kind,
        foks_proto::ENTITY_DEVICE | foks_proto::ENTITY_YUBI | foks_proto::ENTITY_SUBKEY
    ) {
        return Err(Error::AuthorizationChanged);
    }
    let owner: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM devices
         WHERE uid=?1 AND active=1 AND role_type=3 AND visibility=0
           AND (device_id=?2 OR (subkey_id=?2 AND substr(device_id,1,1)=x'08'))
           AND (?3 IS NULL OR device_id=?3))",
        params![uid, credential, signer],
        |row| row.budgeted_get(0),
    )?;
    if !owner {
        return Err(Error::AuthorizationChanged);
    }
    Ok(())
}
