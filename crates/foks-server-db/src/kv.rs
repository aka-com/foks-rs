use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, ReadDatabase, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredKvRoot {
    pub version: u64,
    pub directory_id: [u8; 16],
    pub directory_version: u64,
    pub exact: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredKvDirectory {
    pub id: [u8; 16],
    pub version: u64,
    pub exact: Vec<u8>,
}

pub struct KvDirectoryMutation<'a> {
    pub uid: &'a [u8],
    pub id: &'a [u8; 16],
    pub version: u64,
    pub key_role: u64,
    pub key_visibility: i64,
    pub key_generation: u64,
    pub status: u64,
    pub exact: &'a [u8],
}

pub struct KvRootMutation<'a> {
    pub uid: &'a [u8],
    pub version: u64,
    pub directory_id: &'a [u8; 16],
    pub directory_version: u64,
    pub key_role: u64,
    pub key_visibility: i64,
    pub key_generation: u64,
    pub exact: &'a [u8],
}

pub struct KvNodeMutation<'a> {
    pub uid: &'a [u8],
    pub id: &'a [u8; 17],
    pub node_type: u64,
    pub exact: &'a [u8],
}

pub struct KvDirentMutation<'a> {
    pub parent: &'a [u8; 16],
    pub id: &'a [u8; 16],
    pub version: u64,
    pub directory_version: u64,
    pub node_id: &'a [u8; 17],
    pub name_mac: &'a [u8; 32],
    pub creation_time: u64,
    pub exact: &'a [u8],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredKvNode {
    pub id: [u8; 17],
    pub node_type: u64,
    pub exact: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredKvDirent {
    pub exact: Vec<u8>,
    pub node_id: [u8; 17],
}

pub struct KvFileChunkMutation<'a> {
    pub uid: &'a [u8],
    pub file_id: &'a [u8; 16],
    pub exact_metadata: Option<&'a [u8]>,
    pub offset: u64,
    pub ciphertext: &'a [u8],
    pub final_size: Option<u64>,
    pub exact_chunk: &'a [u8],
    pub now: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredKvFile {
    pub exact_metadata: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredKvFileChunk {
    pub ciphertext: Vec<u8>,
    pub offset: u64,
    pub final_chunk: bool,
}

impl Database {
    /// Materializes a party-scoped KV namespace only for a local user or team.
    /// The polymorphic party binding is resolved from authoritative identity
    /// tables rather than accepted from the caller.
    pub fn ensure_kv_namespace(&mut self, namespace_id: &[u8]) -> Result<bool> {
        if namespace_id.len() != 33 {
            return Err(Error::Invalid("KV namespace ID"));
        }
        let party: Option<(i64, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT party_kind, host_id FROM (
                     -- Identity-only DB tests intentionally have no host
                     -- bootstrap; production always selects host_metadata.
                     SELECT 1 AS party_kind,
                            COALESCE((SELECT host_id FROM host_metadata WHERE singleton = 1),
                                     zeroblob(33)) AS host_id
                       FROM users u WHERE u.uid = ?1
                     UNION ALL
                     SELECT t.team_kind AS party_kind, t.host_id AS host_id
                       FROM teams t WHERE t.team_id = ?1
                 ) LIMIT 1",
                [namespace_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((party_kind, host_id)) = party else {
            return Ok(false);
        };
        let changed = self.connection.execute(
            "INSERT INTO kv_namespaces(namespace_id, party_kind, host_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(namespace_id) DO UPDATE SET
               party_kind = excluded.party_kind,
               host_id = excluded.host_id
             WHERE kv_namespaces.party_kind = excluded.party_kind
               AND kv_namespaces.host_id = excluded.host_id",
            params![namespace_id, party_kind, host_id],
        )?;
        Ok(changed == 1)
    }

    pub fn put_kv_directory(&mut self, mutation: &KvDirectoryMutation<'_>) -> Result<()> {
        self.put_kv_directory_with_precondition(mutation, None)
    }

    pub fn put_kv_directory_with_precondition(
        &mut self,
        mutation: &KvDirectoryMutation<'_>,
        precondition: Option<&foks_proto::KvPathVersionVector>,
    ) -> Result<()> {
        validate_directory(self, mutation)?;
        if !self.ensure_kv_namespace(mutation.uid)? {
            return Err(Error::Invalid("unknown KV namespace"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(i64, Vec<u8>)> = transaction
            .query_row(
                "SELECT h.version, d.exact_directory
                 FROM kv_directory_heads h JOIN kv_directories d
                   ON d.uid = h.uid AND d.directory_id = h.directory_id
                  AND d.version = h.version
                 WHERE h.uid = ?1 AND h.directory_id = ?2",
                params![mutation.uid, mutation.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match existing {
            Some((version, exact))
                if crate::error::unsigned(version)? == mutation.version
                    && exact == mutation.exact =>
            {
                return Ok(())
            }
            Some(_) => return Err(Error::KvConflict),
            None => {}
        }
        if let Some(precondition) = precondition {
            if !kv_version_vector(&transaction, mutation.uid)?
                .as_ref()
                .is_some_and(|current| current.equivalent(precondition))
            {
                return Err(Error::KvConflict);
            }
        }
        ensure_kv_capacity(
            &transaction,
            &self.config,
            mutation.uid,
            mutation.exact.len(),
            1,
        )?;
        transaction.execute(
            "INSERT INTO kv_directories
             (uid, directory_id, version, key_role, key_visibility, key_generation,
              status, exact_directory)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                mutation.uid,
                mutation.id,
                sql_integer(mutation.version)?,
                sql_integer(mutation.key_role)?,
                mutation.key_visibility,
                sql_integer(mutation.key_generation)?,
                sql_integer(mutation.status)?,
                mutation.exact,
            ],
        )?;
        transaction.execute(
            "INSERT INTO kv_directory_heads(uid, directory_id, version) VALUES (?1, ?2, ?3)",
            params![mutation.uid, mutation.id, sql_integer(mutation.version)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn put_kv_node(&mut self, mutation: &KvNodeMutation<'_>) -> Result<()> {
        if mutation.uid.len() != 33
            || !matches!(mutation.node_type, 3 | 4)
            || u64::from(mutation.id[0]) != mutation.node_type
            || mutation.exact.is_empty()
            || mutation.exact.len() > self.config.maximum_kv_node_bytes
        {
            return Err(Error::Invalid("KV node mutation"));
        }
        if !self.ensure_kv_namespace(mutation.uid)? {
            return Err(Error::Invalid("unknown KV namespace"));
        }
        let existing: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT exact_node FROM kv_nodes WHERE uid = ?1 AND node_id = ?2",
                params![mutation.uid, mutation.id],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            Some(exact) if exact == mutation.exact => Ok(()),
            Some(_) => Err(Error::KvConflict),
            None => {
                ensure_kv_capacity(
                    &self.connection,
                    &self.config,
                    mutation.uid,
                    mutation.exact.len(),
                    1,
                )?;
                self.connection.execute(
                    "INSERT INTO kv_nodes(uid, node_id, node_type, exact_node)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        mutation.uid,
                        mutation.id,
                        sql_integer(mutation.node_type)?,
                        mutation.exact
                    ],
                )?;
                Ok(())
            }
        }
    }

    pub fn put_kv_dirents(
        &mut self,
        uid: &[u8],
        precondition: &foks_proto::KvPathVersionVector,
        mutations: &[KvDirentMutation<'_>],
    ) -> Result<()> {
        if uid.len() != 33 || mutations.is_empty() || mutations.len() > 64 {
            return Err(Error::Invalid("KV dirent mutation batch"));
        }
        let mut keys = std::collections::BTreeSet::new();
        for mutation in mutations {
            if mutation.version == 0
                || mutation.directory_version == 0
                || mutation.exact.is_empty()
                || mutation.exact.len() > self.config.maximum_blob_bytes
                || !keys.insert((*mutation.parent, *mutation.id))
            {
                return Err(Error::Invalid("KV dirent mutation"));
            }
        }
        if !self.ensure_kv_namespace(uid)? {
            return Err(Error::Invalid("unknown KV namespace"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut existing_heads = Vec::with_capacity(mutations.len());
        for mutation in mutations {
            let stored: Option<(i64, Vec<u8>)> = transaction
                .query_row(
                    "SELECT h.version, d.exact_dirent FROM kv_dirent_heads h
                     JOIN kv_dirents d ON d.uid = h.uid AND d.parent_id = h.parent_id
                       AND d.dirent_id = h.dirent_id AND d.version = h.version
                     WHERE h.uid = ?1 AND h.parent_id = ?2 AND h.dirent_id = ?3",
                    params![uid, mutation.parent, mutation.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            existing_heads.push(stored);
        }
        if existing_heads
            .iter()
            .zip(mutations)
            .all(|(stored, mutation)| {
                stored.as_ref().is_some_and(|(version, exact)| {
                    u64::try_from(*version) == Ok(mutation.version) && exact == mutation.exact
                })
            })
        {
            return Ok(());
        }
        if !kv_version_vector(&transaction, uid)?
            .as_ref()
            .is_some_and(|current| current.equivalent(precondition))
        {
            return Err(Error::KvConflict);
        }
        let added_bytes = mutations.iter().try_fold(0usize, |total, mutation| {
            total
                .checked_add(mutation.exact.len())
                .ok_or(Error::IntegerRange)
        })?;
        ensure_kv_capacity(
            &transaction,
            &self.config,
            uid,
            added_bytes,
            u64::try_from(mutations.len()).map_err(|_| Error::IntegerRange)?,
        )?;
        for (mutation, existing) in mutations.iter().zip(existing_heads) {
            let directory_exists = transaction
                .query_row(
                    "SELECT 1 FROM kv_directories
                     WHERE uid = ?1 AND directory_id = ?2 AND version = ?3 AND status = 0",
                    params![
                        uid,
                        mutation.parent,
                        sql_integer(mutation.directory_version)?
                    ],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !directory_exists || !node_reference_exists(&transaction, uid, mutation.node_id)? {
                return Err(Error::KvConflict);
            }
            let expected = match existing {
                Some((version, _)) => crate::error::unsigned(version)?
                    .checked_add(1)
                    .ok_or(Error::IntegerRange)?,
                None => 1,
            };
            if mutation.version != expected {
                return Err(Error::KvConflict);
            }
            if mutation.node_id[0] != 0 {
                let conflicting_name = transaction
                    .query_row(
                        "SELECT 1 FROM kv_dirent_heads h
                         JOIN kv_dirents d ON d.uid = h.uid AND d.parent_id = h.parent_id
                           AND d.dirent_id = h.dirent_id AND d.version = h.version
                         WHERE h.uid = ?1 AND h.parent_id = ?2 AND h.dirent_id != ?3
                           AND d.name_mac = ?4 AND substr(d.node_id, 1, 1) != X'00'",
                        params![uid, mutation.parent, mutation.id, mutation.name_mac],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if conflicting_name {
                    return Err(Error::KvConflict);
                }
            }
            transaction.execute(
                "INSERT INTO kv_dirents
                 (uid, parent_id, dirent_id, version, directory_version, node_id,
                  name_mac, creation_time, exact_dirent)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    uid,
                    mutation.parent,
                    mutation.id,
                    sql_integer(mutation.version)?,
                    sql_integer(mutation.directory_version)?,
                    mutation.node_id,
                    mutation.name_mac,
                    sql_integer(mutation.creation_time)?,
                    mutation.exact,
                ],
            )?;
            transaction.execute(
                "INSERT INTO kv_dirent_heads(uid, parent_id, dirent_id, version)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(uid, parent_id, dirent_id)
                 DO UPDATE SET version = excluded.version",
                params![
                    uid,
                    mutation.parent,
                    mutation.id,
                    sql_integer(mutation.version)?
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn put_kv_file_chunk(&mut self, mutation: &KvFileChunkMutation<'_>) -> Result<()> {
        const MAXIMUM_CHUNK_BYTES: usize = 9 * 1024 * 1024;
        // A full 1 GiB cleartext file can expand to just over 2 GiB because
        // the v0.1.9 chunk format pads each 4 MiB clear chunk to 8 MiB.
        const MAXIMUM_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024 + 1024 * 1024;
        const MAXIMUM_CHUNKS: i64 = 512;
        if mutation.uid.len() != 33
            || mutation.ciphertext.is_empty()
            || mutation.ciphertext.len() > MAXIMUM_CHUNK_BYTES
            || mutation.exact_chunk.is_empty()
            || mutation.exact_chunk.len() > MAXIMUM_CHUNK_BYTES + 1024
            || mutation
                .final_size
                .is_some_and(|size| size == 0 || size > MAXIMUM_FILE_BYTES)
            || mutation
                .exact_metadata
                .is_some_and(|metadata| metadata.is_empty() || metadata.len() > 64 * 1024)
            || (mutation.exact_metadata.is_some() && mutation.offset != 0)
        {
            return Err(Error::Invalid("KV file chunk mutation"));
        }
        if !self.ensure_kv_namespace(mutation.uid)? {
            return Err(Error::Invalid("unknown KV namespace"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let upload: Option<(Vec<u8>, i64)> = transaction
            .query_row(
                "SELECT exact_metadata, complete FROM kv_file_uploads
                 WHERE uid = ?1 AND file_id = ?2",
                params![mutation.uid, mutation.file_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let upload_complete = match (upload, mutation.exact_metadata) {
            (None, Some(metadata)) => {
                transaction.execute(
                    "INSERT INTO kv_file_uploads
                     (uid, file_id, exact_metadata, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![
                        mutation.uid,
                        mutation.file_id,
                        metadata,
                        sql_integer(mutation.now)?
                    ],
                )?;
                0
            }
            (None, None) => return Err(Error::KvConflict),
            (Some((stored, _)), Some(metadata)) if stored != metadata => {
                return Err(Error::KvConflict)
            }
            (Some((_, complete)), _) => complete,
        };
        let existing: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT exact_upload_chunk FROM kv_file_chunks
                 WHERE uid = ?1 AND file_id = ?2 AND clear_offset = ?3",
                params![
                    mutation.uid,
                    mutation.file_id,
                    sql_integer(mutation.offset)?
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return if existing == mutation.exact_chunk {
                Ok(())
            } else {
                Err(Error::KvConflict)
            };
        }
        if upload_complete != 0 {
            return Err(Error::KvConflict);
        }
        let (count, maximum_offset, encrypted_bytes): (i64, Option<i64>, i64) = transaction
            .query_row(
                "SELECT count(*), max(clear_offset), coalesce(sum(length(ciphertext)), 0)
                 FROM kv_file_chunks WHERE uid = ?1 AND file_id = ?2",
                params![mutation.uid, mutation.file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        if count >= MAXIMUM_CHUNKS
            || maximum_offset.is_some_and(|offset| {
                u64::try_from(offset).map_or(true, |offset| mutation.offset <= offset)
            })
        {
            return Err(Error::KvConflict);
        }
        let encrypted_bytes = crate::error::unsigned(encrypted_bytes)?
            .checked_add(u64::try_from(mutation.ciphertext.len()).map_err(|_| Error::IntegerRange)?)
            .ok_or(Error::IntegerRange)?;
        if encrypted_bytes > MAXIMUM_FILE_BYTES
            || mutation
                .final_size
                .is_some_and(|final_size| final_size != encrypted_bytes)
        {
            return Err(Error::Invalid("KV file encrypted size"));
        }
        ensure_kv_capacity(
            &transaction,
            &self.config,
            mutation.uid,
            mutation
                .ciphertext
                .len()
                .checked_add(mutation.exact_chunk.len())
                .ok_or(Error::IntegerRange)?,
            1,
        )?;
        transaction.execute(
            "INSERT INTO kv_file_chunks
             (uid, file_id, clear_offset, ciphertext, final_chunk, exact_upload_chunk)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mutation.uid,
                mutation.file_id,
                sql_integer(mutation.offset)?,
                mutation.ciphertext,
                i64::from(mutation.final_size.is_some()),
                mutation.exact_chunk,
            ],
        )?;
        transaction.execute(
            "UPDATE kv_file_uploads SET updated_at = ?3
             WHERE uid = ?1 AND file_id = ?2",
            params![mutation.uid, mutation.file_id, sql_integer(mutation.now)?],
        )?;
        if let Some(final_size) = mutation.final_size {
            transaction.execute(
                "UPDATE kv_file_uploads SET complete = 1, encrypted_size = ?3
                 WHERE uid = ?1 AND file_id = ?2 AND complete = 0",
                params![mutation.uid, mutation.file_id, sql_integer(final_size)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn acquire_kv_lock(
        &mut self,
        uid: &[u8],
        parent: &[u8; 16],
        dirent: &[u8; 16],
        lock: &[u8; 16],
        now: u64,
        expires_at: u64,
    ) -> Result<()> {
        if uid.len() != 33 || expires_at <= now {
            return Err(Error::Invalid("KV lock"));
        }
        if !self.ensure_kv_namespace(uid)? {
            return Err(Error::Invalid("unknown KV namespace"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let directory_exists = transaction
            .query_row(
                "SELECT 1 FROM kv_directory_heads WHERE uid = ?1 AND directory_id = ?2",
                params![uid, parent],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !directory_exists {
            return Err(Error::KvConflict);
        }
        let existing: Option<(Vec<u8>, i64)> = transaction
            .query_row(
                "SELECT lock_id, expires_at FROM kv_locks
                 WHERE uid = ?1 AND parent_id = ?2 AND dirent_id = ?3",
                params![uid, parent, dirent],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if existing.as_ref().is_some_and(|(existing_lock, expiry)| {
            existing_lock.as_slice() != lock
                && u64::try_from(*expiry).is_ok_and(|expiry| expiry > now)
        }) {
            return Err(Error::KvLocked);
        }
        if existing.is_none() {
            ensure_kv_capacity(&transaction, &self.config, uid, 64, 1)?;
        }
        transaction.execute(
            "INSERT INTO kv_locks(uid, parent_id, dirent_id, lock_id, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(uid, parent_id, dirent_id)
             DO UPDATE SET lock_id = excluded.lock_id, expires_at = excluded.expires_at",
            params![uid, parent, dirent, lock, sql_integer(expires_at)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn release_kv_lock(
        &mut self,
        uid: &[u8],
        parent: &[u8; 16],
        dirent: &[u8; 16],
        lock: &[u8; 16],
    ) -> Result<()> {
        let changed = self.connection.execute(
            "DELETE FROM kv_locks
             WHERE uid = ?1 AND parent_id = ?2 AND dirent_id = ?3 AND lock_id = ?4",
            params![uid, parent, dirent, lock],
        )?;
        if changed == 0 {
            return Err(Error::KvLocked);
        }
        Ok(())
    }

    pub fn put_kv_root(&mut self, mutation: &KvRootMutation<'_>) -> Result<()> {
        if mutation.uid.len() != 33
            || mutation.version == 0
            || mutation.directory_version == 0
            || mutation.key_generation == 0
            || mutation.key_role > 3
            || mutation.exact.is_empty()
            || mutation.exact.len() > self.config.maximum_blob_bytes
        {
            return Err(Error::Invalid("KV root mutation"));
        }
        if !self.ensure_kv_namespace(mutation.uid)? {
            return Err(Error::Invalid("unknown KV namespace"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT exact_root FROM kv_roots WHERE uid = ?1",
                [mutation.uid],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            Some(exact) if exact == mutation.exact => return Ok(()),
            Some(_) => return Err(Error::KvConflict),
            None => {}
        }
        let directory_key: Option<(i64, i64, i64)> = transaction
            .query_row(
                "SELECT key_role, key_visibility, key_generation FROM kv_directories
                 WHERE uid = ?1 AND directory_id = ?2 AND version = ?3 AND status = 0",
                params![
                    mutation.uid,
                    mutation.directory_id,
                    sql_integer(mutation.directory_version)?
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let expected_key = (
            sql_integer(mutation.key_role)?,
            mutation.key_visibility,
            sql_integer(mutation.key_generation)?,
        );
        if directory_key != Some(expected_key) {
            return Err(Error::KvConflict);
        }
        ensure_kv_capacity(
            &transaction,
            &self.config,
            mutation.uid,
            mutation.exact.len(),
            1,
        )?;
        transaction.execute(
            "INSERT INTO kv_roots
             (uid, root_version, directory_id, directory_version, exact_root)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                mutation.uid,
                sql_integer(mutation.version)?,
                mutation.directory_id,
                sql_integer(mutation.directory_version)?,
                mutation.exact,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

impl ReadDatabase {
    pub fn kv_root(&self, uid: &[u8]) -> Result<Option<StoredKvRoot>> {
        kv_root(&self.connection, uid)
    }

    pub fn kv_directory(&self, uid: &[u8], id: &[u8; 16]) -> Result<Option<StoredKvDirectory>> {
        kv_directory(&self.connection, uid, id)
    }

    pub fn kv_node(&self, uid: &[u8], id: &[u8; 17]) -> Result<Option<StoredKvNode>> {
        let stored: Option<(i64, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT node_type, exact_node FROM kv_nodes WHERE uid = ?1 AND node_id = ?2",
                params![uid, id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        stored
            .map(|(node_type, exact)| {
                Ok(StoredKvNode {
                    id: *id,
                    node_type: crate::error::unsigned(node_type)?,
                    exact,
                })
            })
            .transpose()
    }

    pub fn kv_file(&self, uid: &[u8], id: &[u8; 16]) -> Result<Option<StoredKvFile>> {
        self.connection
            .query_row(
                "SELECT exact_metadata FROM kv_file_uploads
                 WHERE uid = ?1 AND file_id = ?2 AND complete = 1",
                params![uid, id],
                |row| {
                    Ok(StoredKvFile {
                        exact_metadata: row.get(0)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn kv_file_chunk(
        &self,
        uid: &[u8],
        id: &[u8; 16],
        offset: u64,
    ) -> Result<Option<StoredKvFileChunk>> {
        let stored: Option<(Vec<u8>, i64)> = self
            .connection
            .query_row(
                "SELECT c.ciphertext, c.final_chunk FROM kv_file_chunks c
                 JOIN kv_file_uploads f ON f.uid = c.uid AND f.file_id = c.file_id
                 WHERE c.uid = ?1 AND c.file_id = ?2 AND c.clear_offset = ?3
                   AND f.complete = 1",
                params![uid, id, sql_integer(offset)?],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(stored.map(|(ciphertext, final_chunk)| StoredKvFileChunk {
            ciphertext,
            offset,
            final_chunk: final_chunk != 0,
        }))
    }

    pub fn kv_list(
        &self,
        uid: &[u8],
        parent: &[u8; 16],
        after_name_mac: Option<&[u8; 32]>,
        limit: usize,
    ) -> Result<Vec<StoredKvDirent>> {
        if limit == 0 || limit > 1001 {
            return Err(Error::Invalid("KV list limit"));
        }
        let mut statement = self.connection.prepare(
            "SELECT d.exact_dirent, d.node_id FROM kv_dirent_heads h
             JOIN kv_dirents d ON d.uid = h.uid AND d.parent_id = h.parent_id
               AND d.dirent_id = h.dirent_id AND d.version = h.version
             WHERE h.uid = ?1 AND h.parent_id = ?2
               AND substr(d.node_id, 1, 1) != X'00'
               AND (?3 IS NULL OR d.name_mac > ?3)
             ORDER BY d.name_mac, d.dirent_id LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![
                uid,
                parent,
                after_name_mac,
                i64::try_from(limit).map_err(|_| Error::IntegerRange)?
            ],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;
        rows.map(|row| {
            let (exact, node_id) = row?;
            Ok(StoredKvDirent {
                exact,
                node_id: node_id
                    .try_into()
                    .map_err(|_| Error::Invalid("stored KV node ID"))?,
            })
        })
        .collect()
    }

    pub fn kv_version_vector(&self, uid: &[u8]) -> Result<Option<foks_proto::KvPathVersionVector>> {
        kv_version_vector(&self.connection, uid)
    }
}

fn node_reference_exists(
    connection: &rusqlite::Connection,
    uid: &[u8],
    node: &[u8; 17],
) -> Result<bool> {
    match node[0] {
        0 => Ok(node[1..].iter().all(|byte| *byte == 0)),
        1 => Ok(connection
            .query_row(
                "SELECT 1 FROM kv_directory_heads WHERE uid = ?1 AND directory_id = ?2",
                params![uid, &node[1..]],
                |_| Ok(()),
            )
            .optional()?
            .is_some()),
        2 => Ok(connection
            .query_row(
                "SELECT 1 FROM kv_file_uploads
                 WHERE uid = ?1 AND file_id = ?2 AND complete = 1",
                params![uid, &node[1..]],
                |_| Ok(()),
            )
            .optional()?
            .is_some()),
        3 | 4 => Ok(connection
            .query_row(
                "SELECT 1 FROM kv_nodes WHERE uid = ?1 AND node_id = ?2 AND node_type = ?3",
                params![uid, node, i64::from(node[0])],
                |_| Ok(()),
            )
            .optional()?
            .is_some()),
        _ => Ok(false),
    }
}

fn kv_version_vector(
    connection: &rusqlite::Connection,
    uid: &[u8],
) -> Result<Option<foks_proto::KvPathVersionVector>> {
    let Some(root) = kv_root(connection, uid)? else {
        return Ok(None);
    };
    let mut pending = std::collections::VecDeque::from([root.directory_id]);
    let mut directories = std::collections::BTreeMap::new();
    while let Some(id) = pending.pop_front() {
        if directories.contains_key(&id) {
            continue;
        }
        let version: i64 = connection.query_row(
            "SELECT version FROM kv_directory_heads WHERE uid = ?1 AND directory_id = ?2",
            params![uid, id],
            |row| row.get(0),
        )?;
        let mut entries_statement = connection.prepare(
            "SELECT h.dirent_id, h.version, d.node_id FROM kv_dirent_heads h
             JOIN kv_dirents d ON d.uid = h.uid AND d.parent_id = h.parent_id
               AND d.dirent_id = h.dirent_id AND d.version = h.version
             WHERE h.uid = ?1 AND h.parent_id = ?2
               AND substr(d.node_id, 1, 1) != X'00'
             ORDER BY h.dirent_id",
        )?;
        let rows = entries_statement.query_map(params![uid, id], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (entry_id, version, node_id) = row?;
            entries.push(foks_proto::KvDirentVersion {
                id: entry_id
                    .try_into()
                    .map_err(|_| Error::Invalid("stored KV dirent ID"))?,
                version: crate::error::unsigned(version)?,
            });
            if node_id.first() == Some(&1) {
                pending.push_back(
                    node_id[1..]
                        .try_into()
                        .map_err(|_| Error::Invalid("stored KV child directory ID"))?,
                );
            }
        }
        directories.insert(
            id,
            foks_proto::KvDirectoryVersion {
                id,
                version: crate::error::unsigned(version)?,
                entries,
            },
        );
    }
    Ok(Some(foks_proto::KvPathVersionVector {
        root_version: root.version,
        directories: directories.into_values().collect(),
    }))
}

fn kv_root(connection: &rusqlite::Connection, uid: &[u8]) -> Result<Option<StoredKvRoot>> {
    let stored: Option<(i64, Vec<u8>, i64, Vec<u8>)> = connection
        .query_row(
            "SELECT root_version, directory_id, directory_version, exact_root
             FROM kv_roots WHERE uid = ?1",
            [uid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    stored
        .map(|(version, id, directory_version, exact)| {
            Ok(StoredKvRoot {
                version: crate::error::unsigned(version)?,
                directory_id: id
                    .try_into()
                    .map_err(|_| Error::Invalid("stored KV root directory ID"))?,
                directory_version: crate::error::unsigned(directory_version)?,
                exact,
            })
        })
        .transpose()
}

fn kv_directory(
    connection: &rusqlite::Connection,
    uid: &[u8],
    id: &[u8; 16],
) -> Result<Option<StoredKvDirectory>> {
    let stored: Option<(Vec<u8>, i64, Vec<u8>)> = connection
        .query_row(
            "SELECT d.directory_id, d.version, d.exact_directory
             FROM kv_directory_heads h JOIN kv_directories d
               ON d.uid = h.uid AND d.directory_id = h.directory_id AND d.version = h.version
             WHERE h.uid = ?1 AND h.directory_id = ?2",
            params![uid, id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    stored
        .map(|(id, version, exact)| {
            Ok(StoredKvDirectory {
                id: id
                    .try_into()
                    .map_err(|_| Error::Invalid("stored KV directory ID"))?,
                version: crate::error::unsigned(version)?,
                exact,
            })
        })
        .transpose()
}

fn validate_directory(database: &Database, mutation: &KvDirectoryMutation<'_>) -> Result<()> {
    if mutation.uid.len() != 33
        || mutation.version == 0
        || mutation.key_generation == 0
        || mutation.key_role > 3
        || mutation.status > 2
        || mutation.exact.is_empty()
        || mutation.exact.len() > database.config.maximum_blob_bytes
    {
        return Err(Error::Invalid("KV directory mutation"));
    }
    Ok(())
}

fn ensure_kv_capacity(
    connection: &rusqlite::Connection,
    config: &crate::Config,
    uid: &[u8],
    added_bytes: usize,
    added_objects: u64,
) -> Result<()> {
    let (stored_bytes, stored_objects): (i64, i64) = connection.query_row(
        "SELECT
           coalesce((SELECT sum(length(exact_directory)) FROM kv_directories WHERE uid = ?1), 0) +
           coalesce((SELECT sum(length(exact_root)) FROM kv_roots WHERE uid = ?1), 0) +
           coalesce((SELECT sum(length(exact_node)) FROM kv_nodes WHERE uid = ?1), 0) +
           coalesce((SELECT sum(length(exact_dirent)) FROM kv_dirents WHERE uid = ?1), 0) +
           coalesce((SELECT sum(length(exact_metadata)) FROM kv_file_uploads WHERE uid = ?1), 0) +
           coalesce((SELECT sum(length(ciphertext) + length(exact_upload_chunk))
                     FROM kv_file_chunks WHERE uid = ?1), 0) +
           coalesce((SELECT count(*) * 64 FROM kv_locks WHERE uid = ?1), 0),
           (SELECT count(*) FROM kv_directories WHERE uid = ?1) +
           (SELECT count(*) FROM kv_roots WHERE uid = ?1) +
           (SELECT count(*) FROM kv_nodes WHERE uid = ?1) +
           (SELECT count(*) FROM kv_dirents WHERE uid = ?1) +
           (SELECT count(*) FROM kv_file_uploads WHERE uid = ?1) +
           (SELECT count(*) FROM kv_file_chunks WHERE uid = ?1) +
           (SELECT count(*) FROM kv_locks WHERE uid = ?1)",
        [uid],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let next_bytes = crate::error::unsigned(stored_bytes)?
        .checked_add(u64::try_from(added_bytes).map_err(|_| Error::IntegerRange)?)
        .ok_or(Error::IntegerRange)?;
    let next_objects = crate::error::unsigned(stored_objects)?
        .checked_add(added_objects)
        .ok_or(Error::IntegerRange)?;
    if next_bytes > config.maximum_kv_namespace_bytes
        || next_objects > config.maximum_kv_namespace_objects
    {
        return Err(Error::QuotaExceeded);
    }
    Ok(())
}
