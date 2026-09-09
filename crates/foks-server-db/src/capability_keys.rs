use rusqlite::{params, Connection, OptionalExtension as _, TransactionBehavior};

use crate::{
    error::sql_integer, error::unsigned, Database, Error, ReadDatabase, ReadSnapshot, Result,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub enum CapabilityKeyGenerationState {
    Active = 1,
    Retiring = 2,
    Revoked = 3,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityKeyGeneration {
    pub generation_id: [u8; 16],
    pub encrypted_file_name: String,
    pub state: CapabilityKeyGenerationState,
    pub created_at: u64,
    pub retire_after: Option<u64>,
}

impl Database {
    pub fn capability_key_generations(&self) -> Result<Vec<CapabilityKeyGeneration>> {
        generations(&self.connection)
    }

    pub fn active_capability_key_generation(&self) -> Result<[u8; 16]> {
        active_generation(&self.connection)
    }

    /// Atomically selects a new generation and retains the old key through
    /// every currently-live capability that references it.
    pub fn rotate_capability_key(
        &mut self,
        new_generation_id: [u8; 16],
        encrypted_file_name: &str,
        now: u64,
    ) -> Result<()> {
        if new_generation_id == [0; 16] || !valid_file_name(encrypted_file_name, &new_generation_id)
        {
            return Err(Error::Invalid("capability key rotation"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (old, old_created_at): (Vec<u8>, i64) = transaction.query_row(
            "SELECT generation_id, created_at FROM capability_key_generations WHERE state = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if now < unsigned(old_created_at)? {
            return Err(Error::Invalid(
                "capability key rotation time moved backwards",
            ));
        }
        let maximum_expiry: Option<i64> = transaction.query_row(
            "SELECT max(expires_at) FROM (
                 SELECT expires_at FROM team_view_challenges WHERE key_generation = ?1
                 UNION ALL
                 SELECT expires_at FROM federation_user_view_permissions
                    WHERE key_generation = ?1 AND state = 1
                 UNION ALL
                 SELECT expires_at FROM federation_team_view_permissions
                    WHERE key_generation = ?1 AND state = 1
             )",
            [&old],
            |row| row.get(0),
        )?;
        let retire_after = maximum_expiry
            .map(unsigned)
            .transpose()?
            .unwrap_or(now)
            .max(now);
        require_one(transaction.execute(
            "UPDATE capability_key_generations
             SET state = 2, retire_after = ?1 WHERE generation_id = ?2 AND state = 1",
            params![sql_integer(retire_after)?, old],
        )?)?;
        transaction.execute(
            "INSERT INTO capability_key_generations
             (generation_id, encrypted_file_name, state, created_at, retire_after)
             VALUES (?1, ?2, 1, ?3, NULL)",
            params![new_generation_id, encrypted_file_name, sql_integer(now)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Marks retired key generations as revoked. Key-file deletion is performed
    /// separately by the key provider.
    pub fn revoke_retired_capability_keys(&mut self, now: u64) -> Result<Vec<[u8; 16]>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidates = transaction
            .prepare(
                "SELECT generation_id FROM capability_key_generations
                 WHERE state = 2 AND retire_after <= ?1 ORDER BY generation_id",
            )?
            .query_map([sql_integer(now)?], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut revoked = Vec::with_capacity(candidates.len());
        for generation in candidates {
            let generation: [u8; 16] = generation
                .try_into()
                .map_err(|_| Error::Invalid("stored capability key generation"))?;
            let references: i64 = transaction.query_row(
                "SELECT
                    (SELECT count(*) FROM team_view_challenges
                       WHERE key_generation = ?1 AND expires_at > ?2) +
                    (SELECT count(*) FROM federation_user_view_permissions
                       WHERE key_generation = ?1 AND state = 1) +
                    (SELECT count(*) FROM federation_team_view_permissions
                       WHERE key_generation = ?1 AND state = 1)",
                params![generation, sql_integer(now)?],
                |row| row.get(0),
            )?;
            if references == 0 {
                require_one(transaction.execute(
                    "UPDATE capability_key_generations SET state = 3
                     WHERE generation_id = ?1 AND state = 2",
                    [generation],
                )?)?;
                revoked.push(generation);
            }
        }
        transaction.commit()?;
        Ok(revoked)
    }
}

impl ReadDatabase {
    pub fn capability_key_generations(&self) -> Result<Vec<CapabilityKeyGeneration>> {
        generations(&self.connection)
    }

    pub fn active_capability_key_generation(&self) -> Result<[u8; 16]> {
        active_generation(&self.connection)
    }
}

impl ReadSnapshot<'_> {
    pub fn active_capability_key_generation(&self) -> Result<[u8; 16]> {
        active_generation(self.connection())
    }
}

fn active_generation(connection: &Connection) -> Result<[u8; 16]> {
    connection
        .query_row(
            "SELECT generation_id FROM capability_key_generations WHERE state = 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )?
        .try_into()
        .map_err(|_| Error::Invalid("stored active capability key generation"))
}

pub(crate) fn require_active_generation(
    connection: &Connection,
    generation: &[u8; 16],
) -> Result<()> {
    let active: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM capability_key_generations
             WHERE generation_id = ?1 AND state = 1",
            [generation],
            |row| row.get(0),
        )
        .optional()?;
    if active == Some(1) {
        Ok(())
    } else {
        Err(Error::Invalid("inactive capability key generation"))
    }
}

fn generations(connection: &Connection) -> Result<Vec<CapabilityKeyGeneration>> {
    connection
        .prepare(
            "SELECT generation_id, encrypted_file_name, state, created_at, retire_after
             FROM capability_key_generations ORDER BY created_at, generation_id",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })?
        .map(|row| {
            let (generation, file, state, created_at, retire_after) = row?;
            Ok(CapabilityKeyGeneration {
                generation_id: generation
                    .try_into()
                    .map_err(|_| Error::Invalid("stored capability key generation"))?,
                encrypted_file_name: file,
                state: match state {
                    1 => CapabilityKeyGenerationState::Active,
                    2 => CapabilityKeyGenerationState::Retiring,
                    3 => CapabilityKeyGenerationState::Revoked,
                    _ => return Err(Error::Invalid("stored capability key state")),
                },
                created_at: unsigned(created_at)?,
                retire_after: retire_after.map(unsigned).transpose()?,
            })
        })
        .collect()
}

fn valid_file_name(file: &str, generation: &[u8; 16]) -> bool {
    let expected = generation
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    file == format!("capability.{expected}.key")
}

fn require_one(changed: usize) -> Result<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(Error::AuthorizationChanged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_retains_old_generation_through_live_permission_expiry() {
        let temporary = tempfile::tempdir().unwrap();
        let mut database = Database::open(
            temporary.path().join("server.sqlite3"),
            crate::Config::default(),
        )
        .unwrap();
        database
            .connection
            .execute(
                "INSERT INTO capability_key_generations
                 (generation_id, encrypted_file_name, state, created_at, retire_after)
                 VALUES (?1, 'capability.key', 1, 1, NULL)",
                [[1; 16]],
            )
            .unwrap();
        let mut uid = vec![1; 33];
        uid[0] = foks_proto::ENTITY_USER;
        database
            .connection
            .execute(
                "INSERT INTO names
                 (normalized_name, reservation_token, reservation_sequence, expires_at, uid)
                 VALUES (x'61', NULL, 1, NULL, ?1)",
                [&uid],
            )
            .unwrap();
        database
            .connection
            .execute(
                "INSERT INTO users
                 (uid, normalized_name, username_utf8, username_sequence,
                  username_commitment_key, created_at)
                 VALUES (?1, x'61', x'61', 1, zeroblob(16), 1)",
                [&uid],
            )
            .unwrap();
        database
            .connection
            .execute(
                "INSERT INTO federation_user_view_permissions
                 (target_user_id, viewer_party_id, viewer_host_id, token_hash,
                  token_nonce, token_ciphertext, key_generation, state, issued_at,
                  updated_at, expires_at, revoked_at)
                 VALUES (?1, ?2, ?3, zeroblob(32), zeroblob(24), zeroblob(33),
                         ?4, 1, 2, 2, 100, NULL)",
                params![
                    uid,
                    [foks_proto::ENTITY_USER; 33],
                    [foks_proto::ENTITY_HOST; 33],
                    [1_u8; 16]
                ],
            )
            .unwrap();

        assert!(matches!(
            database.rotate_capability_key(
                [3; 16],
                "capability.03030303030303030303030303030303.key",
                0,
            ),
            Err(Error::Invalid(
                "capability key rotation time moved backwards"
            ))
        ));

        database
            .rotate_capability_key(
                [2; 16],
                "capability.02020202020202020202020202020202.key",
                10,
            )
            .unwrap();
        let generations = database.capability_key_generations().unwrap();
        assert_eq!(generations[0].state, CapabilityKeyGenerationState::Retiring);
        assert_eq!(generations[0].retire_after, Some(100));
        assert_eq!(
            database.active_capability_key_generation().unwrap(),
            [2; 16]
        );
        assert!(database
            .revoke_retired_capability_keys(99)
            .unwrap()
            .is_empty());
        assert_eq!(
            database.revoke_retired_capability_keys(100).unwrap(),
            Vec::<[u8; 16]>::new()
        );
        assert!(database
            .revoke_remote_user_view_permission(
                &uid,
                &[foks_proto::ENTITY_USER; 33],
                &[foks_proto::ENTITY_HOST; 33],
                100,
            )
            .unwrap());
        assert_eq!(
            database.revoke_retired_capability_keys(101).unwrap(),
            vec![[1; 16]]
        );
    }
}
