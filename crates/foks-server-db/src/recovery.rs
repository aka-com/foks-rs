use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, Result};

pub(crate) const RECLAIM_CONSUMED: &str = "DELETE FROM recovery_challenges WHERE consumed = 1";
pub(crate) const RECLAIM_EXPIRED: &str =
    "DELETE FROM recovery_challenges WHERE consumed = 0 AND expires_at <= ?1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryCredentialSnapshot {
    pub uid: Vec<u8>,
    pub normalized_name: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub role_type: u64,
    pub visibility: i64,
}

impl Database {
    pub fn issue_recovery_challenge(
        &mut self,
        challenge_hash: &[u8; 32],
        entity_id: &[u8],
        host_id: &[u8],
        key_generation: &[u8; 16],
        expires_at: u64,
        now: u64,
    ) -> Result<()> {
        if !matches!(entity_id.len(), 33 | 34) || host_id.len() != 33 || expires_at <= now {
            return Err(Error::Invalid("recovery challenge"));
        }
        // Validate timestamps before deleting anything; both range deletes and
        // issuance remain in the same transaction, with complete reclamation.
        let now = sql_integer(now)?;
        let expires_at = sql_integer(expires_at)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(RECLAIM_CONSUMED, [])?;
        transaction.execute(RECLAIM_EXPIRED, [now])?;
        let global: i64 =
            transaction.query_row("SELECT count(*) FROM recovery_challenges", [], |row| {
                row.get(0)
            })?;
        let per_entity: i64 = transaction.query_row(
            "SELECT count(*) FROM recovery_challenges WHERE entity_id = ?1",
            [entity_id],
            |row| row.get(0),
        )?;
        if usize::try_from(global).unwrap_or(usize::MAX)
            >= self.config.maximum_active_recovery_challenges
            || usize::try_from(per_entity).unwrap_or(usize::MAX)
                >= self.config.maximum_recovery_challenges_per_entity
        {
            return Err(Error::QuotaExceeded);
        }
        transaction.execute(
            "INSERT INTO recovery_challenges
             (challenge_hash, entity_id, host_id, key_generation, expires_at, consumed)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![
                challenge_hash,
                entity_id,
                host_id,
                key_generation,
                expires_at
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Atomically consumes a fresh challenge and resolves only an exact,
    /// currently active recovery credential. A missing/replayed challenge and
    /// an unknown/revoked credential are intentionally indistinguishable.
    pub fn consume_recovery_challenge(
        &mut self,
        challenge_hash: &[u8; 32],
        entity_id: &[u8],
        host_id: &[u8],
        key_generation: &[u8; 16],
        now: u64,
    ) -> Result<Option<RecoveryCredentialSnapshot>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "UPDATE recovery_challenges SET consumed = 1
             WHERE challenge_hash = ?1 AND entity_id = ?2 AND host_id = ?3
               AND key_generation = ?4 AND consumed = 0 AND expires_at > ?5",
            params![
                challenge_hash,
                entity_id,
                host_id,
                key_generation,
                sql_integer(now)?
            ],
        )?;
        let credential = if updated == 1 {
            transaction
                .query_row(
                    "SELECT u.uid, u.normalized_name, u.username_utf8,
                            d.role_type, d.visibility
                     FROM devices d JOIN users u ON u.uid = d.uid
                     WHERE d.device_id = ?1 AND d.active = 1",
                    [entity_id],
                    |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, Vec<u8>>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()?
                .map(
                    |(uid, normalized_name, username_utf8, role_type, visibility)| -> Result<_> {
                        Ok(RecoveryCredentialSnapshot {
                            uid,
                            normalized_name,
                            username_utf8,
                            role_type: crate::error::unsigned(role_type)?,
                            visibility,
                        })
                    },
                )
                .transpose()?
        } else {
            None
        };
        transaction.commit()?;
        Ok(credential)
    }
}
