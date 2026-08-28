use rusqlite::{params, Connection, OptionalExtension as _, Transaction, TransactionBehavior};

use crate::{error::sql_integer, error::unsigned, Database, Error, ReadDatabase, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub enum HostKeyGenerationState {
    Staged = 1,
    Active = 2,
    Retiring = 3,
    Revoked = 4,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostKeyGeneration {
    pub generation_id: [u8; 16],
    pub encrypted_file_name: String,
    pub public_entity_id: Vec<u8>,
    pub state: HostKeyGenerationState,
    pub created_at: u64,
    pub activated_hostchain_seqno: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub enum HostRotationPhase {
    Staged = 1,
    Published = 2,
    Complete = 3,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostRotationOperation {
    pub operation_id: [u8; 16],
    pub old_generation_id: [u8; 16],
    pub new_generation_id: [u8; 16],
    pub phase: HostRotationPhase,
    pub add_link_seqno: Option<u64>,
    pub add_published_at: Option<u64>,
    pub revoke_link_seqno: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostRotationPublication {
    pub operation_id: [u8; 16],
    pub expected_root_epoch: u64,
    pub expected_root_hash: [u8; 32],
    pub hostchain_seqno: u64,
    pub hostchain_link_hash: [u8; 32],
    pub exact_hostchain_link: Vec<u8>,
    pub root_epoch: u64,
    pub root_hash: [u8; 32],
    pub root_node: [u8; 32],
    pub exact_root: Vec<u8>,
    pub exact_signed_root: Vec<u8>,
    pub back_pointers: Vec<(u64, [u8; 32])>,
    pub probe_response: Vec<u8>,
    pub now: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredHostchainLink {
    pub seqno: u64,
    pub link_hash: [u8; 32],
    pub exact_link: Vec<u8>,
}

impl Database {
    pub fn host_key_generations(&self) -> Result<Vec<HostKeyGeneration>> {
        generations(&self.connection)
    }

    pub fn active_host_rotation(&self) -> Result<Option<HostRotationOperation>> {
        active_operation(&self.connection)
    }

    pub fn host_rotation_operation(
        &self,
        operation_id: [u8; 16],
    ) -> Result<Option<HostRotationOperation>> {
        operation(&self.connection, operation_id)
    }

    pub fn hostchain_links(&self) -> Result<Vec<StoredHostchainLink>> {
        hostchain_links(&self.connection)
    }

    pub fn stage_host_key_rotation(
        &mut self,
        operation_id: [u8; 16],
        new_generation_id: [u8; 16],
        encrypted_file_name: &str,
        public_entity_id: &[u8],
        now: u64,
    ) -> Result<()> {
        if operation_id == [0; 16]
            || new_generation_id == [0; 16]
            || public_entity_id.len() != 33
            || !valid_generation_file_name(encrypted_file_name, &new_generation_id)
        {
            return Err(Error::Invalid("host key rotation staging"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if active_operation_transaction(&transaction)?.is_some() {
            return Err(Error::Invalid("host key rotation already in progress"));
        }
        let old_generation: Vec<u8> = transaction.query_row(
            "SELECT generation_id FROM host_key_generations
             WHERE purpose = 1 AND state = 2",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO host_key_generations
             (generation_id, purpose, encrypted_file_name, public_entity_id, state, created_at)
             VALUES (?1, 1, ?2, ?3, 1, ?4)",
            params![
                new_generation_id,
                encrypted_file_name,
                public_entity_id,
                sql_integer(now)?
            ],
        )?;
        transaction.execute(
            "INSERT INTO host_rotation_operations
             (operation_id, purpose, old_generation_id, new_generation_id, phase,
              created_at, updated_at)
             VALUES (?1, 1, ?2, ?3, 1, ?4, ?4)",
            params![
                operation_id,
                old_generation,
                new_generation_id,
                sql_integer(now)?
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn publish_host_key_addition(
        &mut self,
        publication: &HostRotationPublication,
    ) -> Result<()> {
        self.publish_host_rotation(publication, false)
    }

    pub fn publish_host_key_revocation(
        &mut self,
        publication: &HostRotationPublication,
    ) -> Result<()> {
        self.publish_host_rotation(publication, true)
    }

    fn publish_host_rotation(
        &mut self,
        publication: &HostRotationPublication,
        revocation: bool,
    ) -> Result<()> {
        validate_publication(publication, self.config.maximum_blob_bytes)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let operation = operation_transaction(&transaction, publication.operation_id)?
            .ok_or(Error::Invalid("missing host key rotation"))?;
        let expected_phase = if revocation {
            HostRotationPhase::Published
        } else {
            HostRotationPhase::Staged
        };
        if operation.phase != expected_phase {
            return Err(Error::Invalid("host key rotation phase"));
        }
        let head: (i64, Vec<u8>) = transaction.query_row(
            "SELECT epoch, root_hash FROM merkle_root_heads WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if head
            != (
                sql_integer(publication.expected_root_epoch)?,
                publication.expected_root_hash.to_vec(),
            )
            || publication.expected_root_epoch.checked_add(1) != Some(publication.root_epoch)
        {
            return Err(Error::StaleRoot);
        }
        let next_link: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(seqno), 0) + 1 FROM hostchain_links",
            [],
            |row| row.get(0),
        )?;
        if next_link != sql_integer(publication.hostchain_seqno)? {
            return Err(Error::Invalid("hostchain sequence"));
        }
        let expected_pointer_epochs =
            foks_merkle_store::back_pointer_sequence(publication.root_epoch);
        if publication
            .back_pointers
            .iter()
            .map(|(epoch, _)| *epoch)
            .ne(expected_pointer_epochs)
        {
            return Err(Error::Invalid("Merkle back-pointer sequence"));
        }
        for (epoch, hash) in &publication.back_pointers {
            let found: Vec<u8> = transaction.query_row(
                "SELECT root_hash FROM merkle_roots WHERE epoch = ?1",
                [sql_integer(*epoch)?],
                |row| row.get(0),
            )?;
            if found != *hash {
                return Err(Error::Invalid("Merkle back-pointer hash"));
            }
        }
        transaction.execute(
            "INSERT INTO hostchain_links(seqno, link_hash, exact_link) VALUES (?1, ?2, ?3)",
            params![
                sql_integer(publication.hostchain_seqno)?,
                publication.hostchain_link_hash,
                publication.exact_hostchain_link
            ],
        )?;
        transaction.execute(
            "INSERT INTO merkle_roots
             (epoch, root_hash, root_node, exact_root, exact_signed_root, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                sql_integer(publication.root_epoch)?,
                publication.root_hash,
                publication.root_node,
                publication.exact_root,
                publication.exact_signed_root,
                sql_integer(publication.now)?
            ],
        )?;
        for (ordinal, (epoch, hash)) in publication.back_pointers.iter().enumerate() {
            transaction.execute(
                "INSERT INTO merkle_back_pointers
                 (root_epoch, target_epoch, target_hash, ordinal) VALUES (?1, ?2, ?3, ?4)",
                params![
                    sql_integer(publication.root_epoch)?,
                    sql_integer(*epoch)?,
                    hash,
                    i64::try_from(ordinal).map_err(|_| Error::IntegerRange)?
                ],
            )?;
        }
        transaction.execute(
            "UPDATE merkle_root_heads SET epoch = ?1, root_hash = ?2 WHERE singleton = 1",
            params![sql_integer(publication.root_epoch)?, publication.root_hash],
        )?;
        transaction.execute(
            "UPDATE host_metadata SET bootstrap_blob = ?1 WHERE singleton = 1",
            [publication.probe_response.as_slice()],
        )?;
        if revocation {
            require_one(transaction.execute(
                "UPDATE host_key_generations SET state = 4
                 WHERE generation_id = ?1 AND state = 3",
                [operation.old_generation_id],
            )?)?;
            require_one(transaction.execute(
                "UPDATE host_rotation_operations
                 SET phase = 3, revoke_link_seqno = ?1, updated_at = ?2
                 WHERE operation_id = ?3 AND phase = 2",
                params![
                    sql_integer(publication.hostchain_seqno)?,
                    sql_integer(publication.now)?,
                    publication.operation_id
                ],
            )?)?;
        } else {
            require_one(transaction.execute(
                "UPDATE host_key_generations SET state = 3
                 WHERE generation_id = ?1 AND state = 2",
                [operation.old_generation_id],
            )?)?;
            require_one(transaction.execute(
                "UPDATE host_key_generations
                 SET state = 2, activated_hostchain_seqno = ?1
                 WHERE generation_id = ?2 AND state = 1",
                params![
                    sql_integer(publication.hostchain_seqno)?,
                    operation.new_generation_id
                ],
            )?)?;
            require_one(transaction.execute(
                "UPDATE host_rotation_operations
                 SET phase = 2, add_link_seqno = ?1, add_published_at = ?2, updated_at = ?2
                 WHERE operation_id = ?3 AND phase = 1",
                params![
                    sql_integer(publication.hostchain_seqno)?,
                    sql_integer(publication.now)?,
                    publication.operation_id
                ],
            )?)?;
        }
        transaction.commit()?;
        Ok(())
    }
}

impl ReadDatabase {
    pub fn host_key_generations(&self) -> Result<Vec<HostKeyGeneration>> {
        generations(&self.connection)
    }

    pub fn active_host_rotation(&self) -> Result<Option<HostRotationOperation>> {
        active_operation(&self.connection)
    }

    pub fn host_rotation_operation(
        &self,
        operation_id: [u8; 16],
    ) -> Result<Option<HostRotationOperation>> {
        operation(&self.connection, operation_id)
    }

    pub fn hostchain_links(&self) -> Result<Vec<StoredHostchainLink>> {
        hostchain_links(&self.connection)
    }
}

fn hostchain_links(connection: &Connection) -> Result<Vec<StoredHostchainLink>> {
    connection
        .prepare("SELECT seqno, link_hash, exact_link FROM hostchain_links ORDER BY seqno")?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?
        .map(|row| {
            let (seqno, hash, exact_link) = row?;
            Ok(StoredHostchainLink {
                seqno: unsigned(seqno)?,
                link_hash: hash
                    .try_into()
                    .map_err(|_| Error::Invalid("stored hostchain link hash"))?,
                exact_link,
            })
        })
        .collect()
}

fn generations(connection: &Connection) -> Result<Vec<HostKeyGeneration>> {
    let mut statement = connection.prepare(
        "SELECT generation_id, encrypted_file_name, public_entity_id, state, created_at,
                activated_hostchain_seqno
         FROM host_key_generations ORDER BY created_at, generation_id",
    )?;
    let generations = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })?
        .map(|row| {
            let (generation, file, entity, state, created_at, activated) = row?;
            Ok(HostKeyGeneration {
                generation_id: generation
                    .try_into()
                    .map_err(|_| Error::Invalid("stored host key generation"))?,
                encrypted_file_name: file,
                public_entity_id: entity,
                state: generation_state(state)?,
                created_at: unsigned(created_at)?,
                activated_hostchain_seqno: activated.map(unsigned).transpose()?,
            })
        })
        .collect();
    generations
}

fn active_operation(connection: &Connection) -> Result<Option<HostRotationOperation>> {
    operation_query(
        connection,
        "SELECT operation_id, old_generation_id, new_generation_id, phase,
                add_link_seqno, add_published_at, revoke_link_seqno, created_at, updated_at
         FROM host_rotation_operations WHERE purpose = 1 AND phase != 3",
        [],
    )
}

fn operation(
    connection: &Connection,
    operation_id: [u8; 16],
) -> Result<Option<HostRotationOperation>> {
    operation_query(
        connection,
        "SELECT operation_id, old_generation_id, new_generation_id, phase,
                add_link_seqno, add_published_at, revoke_link_seqno, created_at, updated_at
         FROM host_rotation_operations WHERE operation_id = ?1",
        [operation_id],
    )
}

fn active_operation_transaction(
    transaction: &Transaction<'_>,
) -> Result<Option<HostRotationOperation>> {
    operation_query(
        transaction,
        "SELECT operation_id, old_generation_id, new_generation_id, phase,
                add_link_seqno, add_published_at, revoke_link_seqno, created_at, updated_at
         FROM host_rotation_operations WHERE purpose = 1 AND phase != 3",
        [],
    )
}

fn operation_transaction(
    transaction: &Transaction<'_>,
    operation_id: [u8; 16],
) -> Result<Option<HostRotationOperation>> {
    operation_query(
        transaction,
        "SELECT operation_id, old_generation_id, new_generation_id, phase,
                add_link_seqno, add_published_at, revoke_link_seqno, created_at, updated_at
         FROM host_rotation_operations WHERE operation_id = ?1",
        [operation_id],
    )
}

fn operation_query<P: rusqlite::Params>(
    connection: &Connection,
    sql: &str,
    params: P,
) -> Result<Option<HostRotationOperation>> {
    let row = connection
        .query_row(sql, params, |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })
        .optional()?;
    row.map(
        |(operation, old, new, phase, add, add_published_at, revoke, created_at, updated_at)| {
            Ok(HostRotationOperation {
                operation_id: operation
                    .try_into()
                    .map_err(|_| Error::Invalid("stored host rotation operation"))?,
                old_generation_id: old
                    .try_into()
                    .map_err(|_| Error::Invalid("stored old host generation"))?,
                new_generation_id: new
                    .try_into()
                    .map_err(|_| Error::Invalid("stored new host generation"))?,
                phase: rotation_phase(phase)?,
                add_link_seqno: add.map(unsigned).transpose()?,
                add_published_at: add_published_at.map(unsigned).transpose()?,
                revoke_link_seqno: revoke.map(unsigned).transpose()?,
                created_at: unsigned(created_at)?,
                updated_at: unsigned(updated_at)?,
            })
        },
    )
    .transpose()
}

fn generation_state(value: i64) -> Result<HostKeyGenerationState> {
    match value {
        1 => Ok(HostKeyGenerationState::Staged),
        2 => Ok(HostKeyGenerationState::Active),
        3 => Ok(HostKeyGenerationState::Retiring),
        4 => Ok(HostKeyGenerationState::Revoked),
        _ => Err(Error::Invalid("stored host key generation state")),
    }
}

fn rotation_phase(value: i64) -> Result<HostRotationPhase> {
    match value {
        1 => Ok(HostRotationPhase::Staged),
        2 => Ok(HostRotationPhase::Published),
        3 => Ok(HostRotationPhase::Complete),
        _ => Err(Error::Invalid("stored host key rotation phase")),
    }
}

fn validate_publication(
    publication: &HostRotationPublication,
    maximum_blob_bytes: usize,
) -> Result<()> {
    if publication.operation_id == [0; 16]
        || publication.hostchain_seqno < 2
        || publication.root_epoch < 2
        || publication.exact_hostchain_link.is_empty()
        || publication.exact_root.is_empty()
        || publication.exact_signed_root.is_empty()
        || publication.probe_response.is_empty()
        || [
            publication.exact_hostchain_link.len(),
            publication.exact_root.len(),
            publication.exact_signed_root.len(),
            publication.probe_response.len(),
        ]
        .into_iter()
        .any(|length| length > maximum_blob_bytes)
    {
        return Err(Error::Invalid("host key rotation publication"));
    }
    Ok(())
}

fn valid_generation_file_name(name: &str, generation: &[u8; 16]) -> bool {
    let hex = generation
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    name == format!("host.{hex}.key")
}

fn require_one(changed: usize) -> Result<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(Error::Invalid("host key rotation ledger transition"))
    }
}
