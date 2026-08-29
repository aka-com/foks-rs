use rusqlite::{params, Connection, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, error::unsigned, Database, Error, ReadSnapshot, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YubiManagementKeySnapshot {
    pub parent_id: Vec<u8>,
    pub exact_box: Vec<u8>,
    pub generation: u64,
    pub role_type: u64,
    pub visibility: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubkeyChallengeResult {
    Expired,
    NotFound,
    Found(Vec<u8>),
}

impl Database {
    /// Burns the challenge before resolving the parent, including on a miss.
    pub fn consume_subkey_challenge(
        &mut self,
        challenge_hash: &[u8; 32],
        parent_id: &[u8],
        host_id: &[u8],
        key_generation: &[u8; 16],
        now: u64,
    ) -> Result<SubkeyChallengeResult> {
        if parent_id.len() != 34 || parent_id.first() != Some(&foks_proto::ENTITY_YUBI) {
            return Err(Error::Invalid("Yubi parent ID"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "UPDATE recovery_challenges SET consumed = 1
             WHERE challenge_hash = ?1 AND entity_id = ?2 AND host_id = ?3
               AND key_generation = ?4 AND consumed = 0 AND expires_at > ?5",
            params![
                challenge_hash,
                parent_id,
                host_id,
                key_generation,
                sql_integer(now)?
            ],
        )?;
        let exact = if updated == 1 {
            transaction
                .query_row(
                    "SELECT b.exact_box FROM yubi_subkey_boxes b
                     JOIN devices d ON d.device_id = b.parent_id
                     WHERE b.parent_id = ?1 AND d.active = 1",
                    [parent_id],
                    |row| row.get(0),
                )
                .optional()?
        } else {
            None
        };
        transaction.commit()?;
        Ok(match (updated, exact) {
            (0, _) => SubkeyChallengeResult::Expired,
            (1, Some(exact)) => SubkeyChallengeResult::Found(exact),
            (1, None) => SubkeyChallengeResult::NotFound,
            _ => return Err(Error::Invalid("challenge update count")),
        })
    }

    pub fn put_yubi_management_key(
        &mut self,
        uid: &[u8],
        credential_id: &[u8],
        value: &YubiManagementKeySnapshot,
        now: u64,
    ) -> Result<()> {
        validate_management_key(uid, value, self.config.maximum_blob_bytes)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = crate::certificates::active_credential_owner(&transaction, uid, credential_id)?
            .ok_or(Error::AuthorizationChanged)?;
        let owner_role: Option<(i64, i64)> = transaction
            .query_row(
                "SELECT role_type, visibility FROM devices
                 WHERE uid = ?1 AND device_id = ?2 AND active = 1",
                params![uid, owner],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let owner_role = owner_role.ok_or(Error::AuthorizationChanged)?;
        if unsigned(owner_role.0)? < value.role_type
            || (unsigned(owner_role.0)? == value.role_type && owner_role.1 < value.visibility)
        {
            return Err(Error::AuthorizationChanged);
        }
        let current_generation: Option<i64> = transaction
            .query_row(
                "SELECT MAX(generation) FROM shared_keys
                 WHERE uid = ?1 AND role_type = ?2 AND visibility = ?3",
                params![uid, sql_integer(value.role_type)?, value.visibility],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if current_generation.map(unsigned).transpose()? != Some(value.generation) {
            return Err(Error::Invalid("stale Yubi management-key generation"));
        }
        let parent_active = transaction
            .query_row(
                "SELECT active FROM devices WHERE uid = ?1 AND device_id = ?2",
                params![uid, value.parent_id],
                |row| Ok(row.get::<_, i64>(0)? == 1),
            )
            .optional()?
            .unwrap_or(false);
        if !parent_active {
            return Err(Error::AuthorizationChanged);
        }
        // Physical management-key rotation republishes fresh opaque ciphertext
        // under the same current PUK. v0.1.9 gives the server no way to validate
        // that ciphertext, so preserve same-role/current-generation replacement
        // while rejecting role downgrades and stale generations.
        let changed = transaction.execute(
            "INSERT INTO yubi_management_keys
             (uid, parent_id, exact_box, generation, role_type, visibility, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(uid, parent_id) DO UPDATE SET
               exact_box = excluded.exact_box,
               generation = excluded.generation,
               role_type = excluded.role_type,
               visibility = excluded.visibility,
               updated_at = excluded.updated_at
             WHERE (yubi_management_keys.role_type < excluded.role_type)
                OR (yubi_management_keys.role_type = excluded.role_type
                    AND yubi_management_keys.generation <= excluded.generation)",
            params![
                uid,
                value.parent_id,
                value.exact_box,
                sql_integer(value.generation)?,
                sql_integer(value.role_type)?,
                value.visibility,
                sql_integer(now)?
            ],
        )?;
        if changed != 1 {
            return Err(Error::Invalid("stale Yubi management-key generation"));
        }
        transaction.commit()?;
        Ok(())
    }
}

impl ReadSnapshot<'_> {
    pub fn yubi_management_key_for_credential(
        &self,
        uid: &[u8],
        credential_id: &[u8],
        parent_id: &[u8],
    ) -> Result<Option<YubiManagementKeySnapshot>> {
        if crate::certificates::active_credential_owner(self.connection(), uid, credential_id)?
            .is_none()
        {
            return Err(Error::AuthorizationChanged);
        }
        management_key(self.connection(), uid, parent_id)
    }

    pub fn all_yubi_management_keys_for_credential(
        &self,
        uid: &[u8],
        credential_id: &[u8],
    ) -> Result<Vec<YubiManagementKeySnapshot>> {
        if crate::certificates::active_credential_owner(self.connection(), uid, credential_id)?
            .is_none()
        {
            return Err(Error::AuthorizationChanged);
        }
        all_management_keys(self.connection(), uid)
    }
}

fn validate_management_key(
    uid: &[u8],
    value: &YubiManagementKeySnapshot,
    maximum_blob_bytes: usize,
) -> Result<()> {
    if uid.len() != 33
        || value.parent_id.len() != 34
        || value.parent_id.first() != Some(&foks_proto::ENTITY_YUBI)
        || value.exact_box.is_empty()
        || value.exact_box.len() > maximum_blob_bytes
        || value.generation == 0
        || !matches!(value.role_type, 2 | 3)
        || value.visibility != 0
    {
        return Err(Error::Invalid("Yubi management key"));
    }
    Ok(())
}

type StoredManagementKey = (Vec<u8>, Vec<u8>, i64, i64, i64);

fn decode_management_key(stored: StoredManagementKey) -> Result<YubiManagementKeySnapshot> {
    Ok(YubiManagementKeySnapshot {
        parent_id: stored.0,
        exact_box: stored.1,
        generation: unsigned(stored.2)?,
        role_type: unsigned(stored.3)?,
        visibility: stored.4,
    })
}

fn management_key(
    connection: &Connection,
    uid: &[u8],
    parent_id: &[u8],
) -> Result<Option<YubiManagementKeySnapshot>> {
    let stored = connection
        .query_row(
            "SELECT k.parent_id, k.exact_box, k.generation, k.role_type, k.visibility
             FROM yubi_management_keys k
             JOIN devices d ON d.uid = k.uid AND d.device_id = k.parent_id AND d.active = 1
             WHERE k.uid = ?1 AND k.parent_id = ?2",
            params![uid, parent_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    stored.map(decode_management_key).transpose()
}

fn all_management_keys(
    connection: &Connection,
    uid: &[u8],
) -> Result<Vec<YubiManagementKeySnapshot>> {
    let mut statement = connection.prepare(
        "SELECT k.parent_id, k.exact_box, k.generation, k.role_type, k.visibility
         FROM yubi_management_keys k
         JOIN devices d ON d.uid = k.uid AND d.device_id = k.parent_id AND d.active = 1
         WHERE k.uid = ?1 ORDER BY k.parent_id",
    )?;
    let result = statement
        .query_map([uid], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .map(|row| decode_management_key(row?))
        .collect();
    result
}
