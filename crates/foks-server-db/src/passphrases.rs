use rusqlite::{params, Connection, OptionalExtension as _, Transaction, TransactionBehavior};

use crate::{
    error::sql_integer, error::unsigned, Config, Database, Error, ReadDatabase, ReadSnapshot,
    Result,
};

#[derive(Clone, Copy)]
pub struct PassphraseMutation<'a> {
    pub verify_key: &'a [u8],
    pub salt: &'a [u8; 16],
    pub generation: u64,
    pub exact_skmwk_box: &'a [u8],
    pub exact_passphrase_box: &'a [u8],
    pub exact_puk_box: Option<&'a [u8]>,
    pub puk_generation: Option<u64>,
    pub stretch_version: u64,
    pub now: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PassphraseSnapshot {
    pub verify_key: Vec<u8>,
    pub salt: [u8; 16],
    pub generation: u64,
    pub exact_skmwk_box: Vec<u8>,
    pub exact_passphrase_box: Vec<u8>,
    pub exact_puk_box: Option<Vec<u8>>,
    pub puk_generation: Option<u64>,
    pub stretch_version: u64,
}

impl Database {
    /// Installs the first passphrase generation only while the authenticated
    /// credential remains active at this mutation's serialization point.
    pub fn set_passphrase(
        &mut self,
        uid: &[u8],
        credential_id: &[u8],
        mutation: PassphraseMutation<'_>,
    ) -> Result<()> {
        validate_mutation(&self.config, uid, mutation)?;
        if mutation.generation != 1 {
            return Err(Error::PassphraseGeneration);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if crate::certificates::active_credential_owner(&transaction, uid, credential_id)?.is_none()
        {
            return Err(Error::AuthorizationChanged);
        }
        if snapshot(&transaction, uid)?.is_some() {
            return Err(Error::PassphraseGeneration);
        }
        ensure_user(&transaction, uid)?;
        insert_salt(&transaction, uid, mutation)?;
        insert_box(&transaction, uid, mutation)?;
        transaction.commit()?;
        Ok(())
    }

    /// Advances the passphrase generation only while the authenticated
    /// credential remains active at this mutation's serialization point.
    pub fn change_passphrase(
        &mut self,
        uid: &[u8],
        credential_id: &[u8],
        mutation: PassphraseMutation<'_>,
    ) -> Result<()> {
        validate_mutation(&self.config, uid, mutation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if crate::certificates::active_credential_owner(&transaction, uid, credential_id)?.is_none()
        {
            return Err(Error::AuthorizationChanged);
        }
        let current = snapshot(&transaction, uid)?.ok_or(Error::PassphraseNotFound)?;
        if current.salt != *mutation.salt
            || current.stretch_version != mutation.stretch_version
            || current
                .generation
                .checked_add(1)
                .is_none_or(|next| next != mutation.generation)
        {
            return Err(Error::PassphraseGeneration);
        }
        insert_box(&transaction, uid, mutation)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn passphrase(&self, uid: &[u8]) -> Result<Option<PassphraseSnapshot>> {
        snapshot(&self.connection, uid)
    }

    pub fn issue_passphrase_challenge(
        &mut self,
        challenge_hash: &[u8; 32],
        uid: &[u8],
        host_id: &[u8],
        key_generation: &[u8; 16],
        expires_at: u64,
        now: u64,
    ) -> Result<()> {
        if uid.len() != 33 || host_id.len() != 33 || expires_at <= now {
            return Err(Error::Invalid("passphrase login challenge"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune_attempts(&transaction, &self.config, now)?;
        if rate_limited(&transaction, &self.config, uid, now)? {
            return Err(Error::PassphraseRateLimited);
        }
        transaction.execute(
            "DELETE FROM passphrase_login_challenges
             WHERE expires_at <= ?1 OR consumed = 1",
            [sql_integer(now)?],
        )?;
        let global: i64 = transaction.query_row(
            "SELECT count(*) FROM passphrase_login_challenges",
            [],
            |row| row.get(0),
        )?;
        let per_user: i64 = transaction.query_row(
            "SELECT count(*) FROM passphrase_login_challenges WHERE uid = ?1",
            [uid],
            |row| row.get(0),
        )?;
        if usize::try_from(global).unwrap_or(usize::MAX)
            >= self.config.maximum_active_passphrase_challenges
            || usize::try_from(per_user).unwrap_or(usize::MAX)
                >= self.config.maximum_passphrase_challenges_per_user
        {
            return Err(Error::QuotaExceeded);
        }
        transaction.execute(
            "INSERT INTO passphrase_login_challenges
             (challenge_hash, uid, host_id, key_generation, expires_at, consumed)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![
                challenge_hash,
                uid,
                host_id,
                key_generation,
                sql_integer(expires_at)?
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn passphrase_for_login(&self, uid: &[u8], now: u64) -> Result<Option<PassphraseSnapshot>> {
        if rate_limited(&self.connection, &self.config, uid, now)? {
            return Err(Error::PassphraseRateLimited);
        }
        snapshot(&self.connection, uid)
    }

    pub fn record_bad_passphrase(&mut self, uid: &[u8], now: u64) -> Result<()> {
        if uid.len() != 33 {
            return Err(Error::Invalid("passphrase login UID"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune_attempts(&transaction, &self.config, now)?;
        if rate_limited(&transaction, &self.config, uid, now)? {
            return Err(Error::PassphraseRateLimited);
        }
        transaction.execute(
            "INSERT INTO bad_passphrase_attempts(uid, attempted_at) VALUES (?1, ?2)",
            params![uid, sql_integer(now)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Burns an exact challenge and returns the current PPE boxes only if the
    /// verify key checked by the caller is still authoritative.
    pub fn consume_passphrase_challenge(
        &mut self,
        challenge_hash: &[u8; 32],
        uid: &[u8],
        host_id: &[u8],
        key_generation: &[u8; 16],
        expected_verify_key: &[u8],
        now: u64,
    ) -> Result<Option<PassphraseSnapshot>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if rate_limited(&transaction, &self.config, uid, now)? {
            return Err(Error::PassphraseRateLimited);
        }
        let updated = transaction.execute(
            "UPDATE passphrase_login_challenges SET consumed = 1
             WHERE challenge_hash = ?1 AND uid = ?2 AND host_id = ?3
               AND key_generation = ?4 AND consumed = 0 AND expires_at > ?5",
            params![
                challenge_hash,
                uid,
                host_id,
                key_generation,
                sql_integer(now)?
            ],
        )?;
        let result = if updated == 1 {
            snapshot(&transaction, uid)?.filter(|state| state.verify_key == expected_verify_key)
        } else {
            None
        };
        if result.is_some() {
            transaction.execute("DELETE FROM bad_passphrase_attempts WHERE uid = ?1", [uid])?;
        }
        transaction.commit()?;
        Ok(result)
    }

    pub(crate) fn insert_identity_passphrase(
        transaction: &Transaction<'_>,
        config: &Config,
        uid: &[u8],
        mutation: PassphraseMutation<'_>,
    ) -> Result<()> {
        validate_mutation(config, uid, mutation)?;
        if mutation.generation != 1 {
            return Err(Error::PassphraseGeneration);
        }
        insert_salt(transaction, uid, mutation)?;
        insert_box(transaction, uid, mutation)
    }
}

impl ReadDatabase {
    pub fn passphrase(&self, uid: &[u8]) -> Result<Option<PassphraseSnapshot>> {
        snapshot(&self.connection, uid)
    }
}

impl ReadSnapshot<'_> {
    pub fn passphrase(&self, uid: &[u8]) -> Result<Option<PassphraseSnapshot>> {
        snapshot(self.connection(), uid)
    }
}

fn ensure_user(transaction: &Transaction<'_>, uid: &[u8]) -> Result<()> {
    let present: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE uid = ?1)",
        [uid],
        |row| row.get(0),
    )?;
    if present {
        Ok(())
    } else {
        Err(Error::Invalid("passphrase user does not exist"))
    }
}

fn insert_salt(
    transaction: &Transaction<'_>,
    uid: &[u8],
    mutation: PassphraseMutation<'_>,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO passphrase_salts(uid, salt, created_at) VALUES (?1, ?2, ?3)",
        params![uid, mutation.salt, sql_integer(mutation.now)?],
    )?;
    Ok(())
}

pub(crate) fn insert_box(
    transaction: &Transaction<'_>,
    uid: &[u8],
    mutation: PassphraseMutation<'_>,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO passphrase_boxes
         (uid, generation, verify_key, exact_skmwk_box, exact_passphrase_box,
          exact_puk_box, puk_generation, stretch_version, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            uid,
            sql_integer(mutation.generation)?,
            mutation.verify_key,
            mutation.exact_skmwk_box,
            mutation.exact_passphrase_box,
            mutation.exact_puk_box,
            mutation.puk_generation.map(sql_integer).transpose()?,
            sql_integer(mutation.stretch_version)?,
            sql_integer(mutation.now)?
        ],
    )?;
    Ok(())
}

pub(crate) fn snapshot(connection: &Connection, uid: &[u8]) -> Result<Option<PassphraseSnapshot>> {
    type Row = (
        Vec<u8>,
        Vec<u8>,
        i64,
        Vec<u8>,
        Vec<u8>,
        Option<Vec<u8>>,
        Option<i64>,
        i64,
    );
    let stored: Option<Row> = connection
        .query_row(
            "SELECT b.verify_key, s.salt, b.generation, b.exact_skmwk_box,
                    b.exact_passphrase_box, b.exact_puk_box, b.puk_generation,
                    b.stretch_version
             FROM passphrase_salts s JOIN passphrase_boxes b ON b.uid = s.uid
             WHERE s.uid = ?1 ORDER BY b.generation DESC LIMIT 1",
            [uid],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()?;
    stored
        .map(
            |(
                verify_key,
                salt,
                generation,
                exact_skmwk_box,
                exact_passphrase_box,
                exact_puk_box,
                puk_generation,
                stretch_version,
            )| {
                Ok(PassphraseSnapshot {
                    verify_key,
                    salt: salt
                        .try_into()
                        .map_err(|_| Error::Invalid("stored passphrase salt"))?,
                    generation: unsigned(generation)?,
                    exact_skmwk_box,
                    exact_passphrase_box,
                    exact_puk_box,
                    puk_generation: puk_generation.map(unsigned).transpose()?,
                    stretch_version: unsigned(stretch_version)?,
                })
            },
        )
        .transpose()
}

pub(crate) fn validate_mutation(
    config: &Config,
    uid: &[u8],
    mutation: PassphraseMutation<'_>,
) -> Result<()> {
    let blobs = [mutation.exact_skmwk_box, mutation.exact_passphrase_box];
    if uid.len() != 33
        || mutation.verify_key.len() != 33
        || mutation.verify_key[0] != foks_proto::ENTITY_PASSPHRASE_KEY
        || mutation.salt == &[0; 16]
        || mutation.generation == 0
        || mutation.generation > config.maximum_passphrase_generations
        || mutation.stretch_version != 1
        || blobs
            .iter()
            .any(|blob| blob.is_empty() || blob.len() > config.maximum_blob_bytes)
        || mutation
            .exact_puk_box
            .is_some_and(|blob| blob.is_empty() || blob.len() > config.maximum_blob_bytes)
        || mutation.exact_puk_box.is_some() != mutation.puk_generation.is_some()
        || mutation.puk_generation == Some(0)
    {
        return Err(Error::Invalid("passphrase mutation"));
    }
    Ok(())
}

pub(crate) fn apply_owner_rotation(
    transaction: &Transaction<'_>,
    config: &Config,
    uid: &[u8],
    owner_generation: Option<u64>,
    mutation: Option<PassphraseMutation<'_>>,
) -> Result<()> {
    let current = snapshot(transaction, uid)?;
    match (owner_generation, current, mutation) {
        (None, _, None) => Ok(()),
        (None, _, Some(_)) => Err(Error::Invalid(
            "passphrase annex without an owner PUK rotation",
        )),
        (Some(_), None, None) => Ok(()),
        (Some(_), None, Some(_)) => Err(Error::PassphraseNotFound),
        (Some(_), Some(_), None) => Err(Error::Invalid(
            "owner PUK rotation omitted the configured passphrase annex",
        )),
        (Some(owner_generation), Some(current), Some(mutation)) => {
            validate_mutation(config, uid, mutation)?;
            if current.salt != *mutation.salt
                || current.stretch_version != mutation.stretch_version
                || current
                    .generation
                    .checked_add(1)
                    .is_none_or(|next| next != mutation.generation)
                || mutation.puk_generation != Some(owner_generation)
            {
                return Err(Error::PassphraseGeneration);
            }
            insert_box(transaction, uid, mutation)
        }
    }
}

fn prune_attempts(transaction: &Transaction<'_>, config: &Config, now: u64) -> Result<()> {
    let cutoff = login_cutoff(config, now)?;
    transaction.execute(
        "DELETE FROM bad_passphrase_attempts WHERE attempted_at <= ?1",
        [sql_integer(cutoff)?],
    )?;
    Ok(())
}

fn rate_limited(connection: &Connection, config: &Config, uid: &[u8], now: u64) -> Result<bool> {
    let cutoff = login_cutoff(config, now)?;
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM bad_passphrase_attempts
         WHERE uid = ?1 AND attempted_at > ?2",
        params![uid, sql_integer(cutoff)?],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(count).unwrap_or(usize::MAX) >= config.maximum_bad_passphrase_attempts)
}

fn login_cutoff(config: &Config, now: u64) -> Result<u64> {
    let window =
        u64::try_from(config.bad_passphrase_window.as_micros()).map_err(|_| Error::IntegerRange)?;
    Ok(now.saturating_sub(window))
}
