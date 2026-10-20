/// Current-name presence query shared by contract with the typed metadata catalog.
/// Names are deliberately nonunique; exclude only the requested dirent itself.
pub const KV_NAME_CONFLICT_SQL: &str = "SELECT '1' FROM kv_dirent_current
WHERE uid = ?1 AND parent_id = ?2 AND dirent_id != ?3 AND name_mac = ?4 LIMIT 1";

pub const KV_LIST_INITIAL_SQL: &str = "SELECT d.exact_dirent, c.node_id FROM kv_dirent_current c
JOIN kv_dirents d ON d.uid=c.uid AND d.parent_id=c.parent_id AND d.dirent_id=c.dirent_id AND d.version=c.version
WHERE c.uid=?1 AND c.parent_id=?2 ORDER BY c.name_mac,c.dirent_id LIMIT ?3";
pub const KV_LIST_MAC_SQL: &str = "SELECT d.exact_dirent, c.node_id FROM kv_dirent_current c
JOIN kv_dirents d ON d.uid=c.uid AND d.parent_id=c.parent_id AND d.dirent_id=c.dirent_id AND d.version=c.version
WHERE c.uid=?1 AND c.parent_id=?2 AND c.name_mac>?3 ORDER BY c.name_mac,c.dirent_id LIMIT ?4";
pub const KV_LIST_TIME_SQL: &str = "SELECT d.exact_dirent, c.node_id FROM kv_dirent_current c
JOIN kv_dirents d ON d.uid=c.uid AND d.parent_id=c.parent_id AND d.dirent_id=c.dirent_id AND d.version=c.version
WHERE c.uid=?1 AND c.parent_id=?2 AND c.creation_time>=?3 ORDER BY c.creation_time,c.dirent_id LIMIT ?4";

use crate::allocation::RowAllocationExt as _;
use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result};

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
    pub exact_directory: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvListCursor {
    Mac([u8; 32]),
    Time(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KvVersionCheck {
    Current,
    Stale(foks_proto::KvPathVersionVector),
    Future,
    Missing,
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
    pub payload: crate::KvChunkPayload,
    pub offset: u64,
    pub final_chunk: bool,
}

impl Database {
    pub fn kv_directory(&self, uid: &[u8], id: &[u8; 16]) -> Result<Option<StoredKvDirectory>> {
        kv_directory(&self.connection, uid, id)
    }

    pub fn kv_dirent(
        &self,
        uid: &[u8],
        parent: &[u8; 16],
        id: &[u8; 16],
    ) -> Result<Option<StoredKvDirent>> {
        kv_dirent(&self.connection, uid, parent, id)
    }

    /// Materializes a party-scoped KV namespace for a local user or team.
    /// The party binding is verified against existing user and team tables.
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
                |row| Ok((row.budgeted_get(0)?, row.budgeted_get(1)?)),
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
                |row| Ok((row.budgeted_get(0)?, row.budgeted_get(1)?)),
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
            if kv_version_check(&transaction, mutation.uid, precondition)?
                != KvVersionCheck::Current
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
        ensure_kv_tree_capacity(&transaction, &self.config, mutation.uid, 1, 0)?;
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(Vec<u8>, Option<i64>)> = transaction
            .query_row(
                "SELECT exact_node,ciphertext_bytes FROM kv_nodes WHERE uid = ?1 AND node_id = ?2",
                params![mutation.uid, mutation.id],
                |row| Ok((row.budgeted_get(0)?, row.budgeted_get(1)?)),
            )
            .optional()?;
        match existing {
            Some((exact, length)) if exact == mutation.exact => {
                if length.is_none() {
                    stamp_node_usage(&transaction, mutation)?;
                    transaction.commit()?;
                }
                Ok(())
            }
            Some(_) => Err(Error::KvConflict),
            None => {
                ensure_kv_capacity(
                    &transaction,
                    &self.config,
                    mutation.uid,
                    mutation.exact.len(),
                    1,
                )?;
                transaction.execute(
                    "INSERT INTO kv_nodes(uid, node_id, node_type, exact_node)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        mutation.uid,
                        mutation.id,
                        sql_integer(mutation.node_type)?,
                        mutation.exact
                    ],
                )?;
                stamp_node_usage(&transaction, mutation)?;
                transaction.commit()?;
                Ok(())
            }
        }
    }

    pub fn put_kv_dirents(
        &mut self,
        uid: &[u8],
        precondition: Option<&foks_proto::KvPathVersionVector>,
        caller_role: foks_proto::Role,
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
                || mutation.exact.len() > self.config.maximum_kv_dirent_bytes
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
                    |row| Ok((row.budgeted_get(0)?, row.budgeted_get(1)?)),
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
        if let Some(precondition) = precondition {
            if kv_version_check(&transaction, uid, precondition)? != KvVersionCheck::Current {
                return Err(Error::KvConflict);
            }
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
        let added_dirents = existing_heads
            .iter()
            .filter(|stored| stored.is_none())
            .count();
        ensure_kv_tree_capacity(
            &transaction,
            &self.config,
            uid,
            0,
            u64::try_from(added_dirents).map_err(|_| Error::IntegerRange)?,
        )?;
        for (mutation, existing) in mutations.iter().zip(existing_heads) {
            if caller_role < foks_proto::Role::OWNER {
                if let Some((_, exact)) = existing.as_ref() {
                    let stored = foks_proto::KvDirent::decode(exact)
                        .map_err(|_| Error::Invalid("stored KV dirent"))?;
                    if caller_role < stored.write_role {
                        return Err(Error::KvPermission);
                    }
                }
            }
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
                        KV_NAME_CONFLICT_SQL,
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
        self.put_kv_file_chunk_impl(mutation, None, true)
    }

    /// Validates admission without retaining upload/chunk changes or reserving
    /// quota. External I/O may follow; the eventual commit must recheck admission.
    pub fn preflight_kv_file_chunk(&mut self, mutation: &KvFileChunkMutation<'_>) -> Result<()> {
        self.put_kv_file_chunk_impl(mutation, None, false)
    }

    /// The caller must durably publish the immutable object before this commit.
    pub fn put_kv_file_chunk_external(
        &mut self,
        mutation: &KvFileChunkMutation<'_>,
        digest: &[u8; 32],
    ) -> Result<()> {
        crate::kv_objects::validate_encoded_chunk(
            mutation.exact_chunk,
            mutation.ciphertext,
            mutation.offset,
            mutation.final_size.is_some(),
            Some(*digest),
        )?;
        let decoded = foks_proto::KvUploadChunk::decode(mutation.exact_chunk)
            .map_err(|_| Error::Invalid("external KV chunk encoding"))?;
        if decoded.final_upload.as_ref().map(|value| value.size) != mutation.final_size {
            return Err(Error::Invalid("external KV chunk final size"));
        }
        self.put_kv_file_chunk_impl(mutation, Some(digest), true)
    }

    fn put_kv_file_chunk_impl(
        &mut self,
        mutation: &KvFileChunkMutation<'_>,
        digest: Option<&[u8; 32]>,
        commit: bool,
    ) -> Result<()> {
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
        let upload: Option<(Vec<u8>, i64, bool)> = transaction
            .query_row(
                "SELECT exact_metadata, complete, reclaiming FROM kv_file_uploads
                 WHERE uid = ?1 AND file_id = ?2",
                params![mutation.uid, mutation.file_id],
                |row| {
                    Ok((
                        row.budgeted_get(0)?,
                        row.budgeted_get(1)?,
                        row.budgeted_get(2)?,
                    ))
                },
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
            (Some((_, _, true)), _) => return Err(Error::KvConflict),
            (Some((stored, _, false)), Some(metadata)) if stored != metadata => {
                return Err(Error::KvConflict)
            }
            (Some((_, complete, false)), _) => complete,
        };
        let existing: Option<(Vec<u8>, Option<Vec<u8>>)> = transaction
            .query_row(
                "SELECT exact_upload_chunk, external_digest FROM kv_file_chunks
                 WHERE uid = ?1 AND file_id = ?2 AND clear_offset = ?3",
                params![
                    mutation.uid,
                    mutation.file_id,
                    sql_integer(mutation.offset)?
                ],
                |row| Ok((row.budgeted_get(0)?, row.budgeted_get(1)?)),
            )
            .optional()?;
        if let Some((existing, external_digest)) = existing {
            let matches = match external_digest {
                Some(stored) => stored == crate::kv_objects::chunk_digest(mutation.exact_chunk),
                None => existing == mutation.exact_chunk,
            };
            return if matches {
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
                KV_UPLOAD_USAGE_SQL,
                params![mutation.uid, mutation.file_id],
                |row| {
                    Ok((
                        row.budgeted_get(0)?,
                        row.budgeted_get(1)?,
                        row.budgeted_get(2)?,
                    ))
                },
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
        if !commit {
            return Ok(());
        }
        let external = digest.is_some();
        let object_id = digest
            .map(|digest| {
                crate::chunk_ledger::require_present(
                    &transaction,
                    digest,
                    mutation.exact_chunk.len() as u64,
                )
            })
            .transpose()?;
        transaction.execute(
            "INSERT INTO kv_file_chunks
             (uid, file_id, clear_offset, ciphertext, final_chunk, exact_upload_chunk,
              external_digest, external_ciphertext_bytes, external_encoded_bytes, external_object_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                mutation.uid,
                mutation.file_id,
                sql_integer(mutation.offset)?,
                if external {
                    &[][..]
                } else {
                    mutation.ciphertext
                },
                i64::from(mutation.final_size.is_some()),
                if external {
                    &[][..]
                } else {
                    mutation.exact_chunk
                },
                digest.map(|value| value.as_slice()),
                external.then_some(mutation.ciphertext.len() as i64),
                external.then_some(mutation.exact_chunk.len() as i64),
                object_id.as_ref().map(|id|id.as_slice()),
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
        timeout: u64,
    ) -> Result<()> {
        if uid.len() != 33 {
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
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT created_at FROM kv_locks
                 WHERE uid = ?1 AND parent_id = ?2 AND dirent_id = ?3",
                params![uid, parent, dirent],
                |row| row.budgeted_get(0),
            )
            .optional()?;
        if let Some(created_at) = existing {
            let expires_at = crate::error::unsigned(created_at)?
                .checked_add(timeout)
                .ok_or(Error::IntegerRange)?;
            if now < expires_at {
                return Err(Error::KvLocked);
            }
        }
        if existing.is_none() {
            ensure_kv_capacity(&transaction, &self.config, uid, 64, 1)?;
        }
        transaction.execute(
            "INSERT INTO kv_locks(uid, parent_id, dirent_id, lock_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(uid, parent_id, dirent_id)
             DO UPDATE SET lock_id = excluded.lock_id, created_at = excluded.created_at",
            params![uid, parent, dirent, lock, sql_integer(now)?],
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
            return Err(Error::KvLockTimeout);
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
        let existing: Option<(i64, Vec<u8>)> = transaction
            .query_row(
                "SELECT root_version, exact_root FROM kv_roots WHERE uid = ?1",
                [mutation.uid],
                |row| Ok((row.budgeted_get(0)?, row.budgeted_get(1)?)),
            )
            .optional()?;
        match &existing {
            Some((version, exact))
                if crate::error::unsigned(*version)? == mutation.version
                    && exact == mutation.exact =>
            {
                return Ok(())
            }
            Some((version, _))
                if crate::error::unsigned(*version)?
                    .checked_add(1)
                    .is_none_or(|next| next != mutation.version) =>
            {
                return Err(Error::KvConflict)
            }
            None if mutation.version != 1 => return Err(Error::KvConflict),
            _ => {}
        }
        let directory_exists = transaction
            .query_row(
                "SELECT 1 FROM kv_directories
                 WHERE uid = ?1 AND directory_id = ?2 AND version = ?3 AND status = 0",
                params![
                    mutation.uid,
                    mutation.directory_id,
                    sql_integer(mutation.directory_version)?
                ],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !directory_exists {
            return Err(Error::KvConflict);
        }
        let prior_bytes = existing.as_ref().map_or(0, |(_, exact)| exact.len());
        ensure_kv_capacity(
            &transaction,
            &self.config,
            mutation.uid,
            mutation.exact.len().saturating_sub(prior_bytes),
            u64::from(existing.is_none()),
        )?;
        transaction.execute(
            "INSERT INTO kv_roots
             (uid, root_version, directory_id, directory_version, exact_root)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(uid) DO UPDATE SET
               root_version = excluded.root_version,
               directory_id = excluded.directory_id,
               directory_version = excluded.directory_version,
               exact_root = excluded.exact_root",
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
        kv_node(&self.connection, uid, id)
    }

    pub fn kv_file(&self, uid: &[u8], id: &[u8; 16]) -> Result<Option<StoredKvFile>> {
        kv_file(&self.connection, uid, id)
    }

    pub fn kv_file_chunk(
        &self,
        uid: &[u8],
        id: &[u8; 16],
        offset: u64,
    ) -> Result<Option<StoredKvFileChunk>> {
        kv_file_chunk(&self.connection, uid, id, offset)
    }

    pub fn kv_list(
        &self,
        uid: &[u8],
        parent: &[u8; 16],
        cursor: Option<KvListCursor>,
        limit: usize,
    ) -> Result<Vec<StoredKvDirent>> {
        kv_list(&self.connection, uid, parent, cursor, limit)
    }

    pub fn kv_dirent_at_name(
        &self,
        uid: &[u8],
        parent: &[u8; 16],
        directory_version: u64,
        name_mac: &[u8; 32],
    ) -> Result<Option<StoredKvDirent>> {
        kv_dirent_at_name(&self.connection, uid, parent, directory_version, name_mac)
    }

    pub fn kv_usage(&self, uid: &[u8]) -> Result<foks_proto::KvUsage> {
        kv_usage(&self.connection, uid)
    }

    pub fn kv_version_check(
        &self,
        uid: &[u8],
        supplied: &foks_proto::KvPathVersionVector,
    ) -> Result<KvVersionCheck> {
        kv_version_check(&self.connection, uid, supplied)
    }

    pub fn kv_version_vector(&self, uid: &[u8]) -> Result<Option<foks_proto::KvPathVersionVector>> {
        kv_version_vector(&self.connection, uid)
    }
}

impl ReadSnapshot<'_> {
    pub fn kv_root(&self, uid: &[u8]) -> Result<Option<StoredKvRoot>> {
        kv_root(self.connection(), uid)
    }

    pub fn kv_directory(&self, uid: &[u8], id: &[u8; 16]) -> Result<Option<StoredKvDirectory>> {
        kv_directory(self.connection(), uid, id)
    }

    pub fn kv_node(&self, uid: &[u8], id: &[u8; 17]) -> Result<Option<StoredKvNode>> {
        kv_node(self.connection(), uid, id)
    }

    pub fn kv_file(&self, uid: &[u8], id: &[u8; 16]) -> Result<Option<StoredKvFile>> {
        kv_file(self.connection(), uid, id)
    }

    pub fn kv_file_chunk(
        &self,
        uid: &[u8],
        id: &[u8; 16],
        offset: u64,
    ) -> Result<Option<StoredKvFileChunk>> {
        kv_file_chunk(self.connection(), uid, id, offset)
    }

    pub fn kv_list(
        &self,
        uid: &[u8],
        parent: &[u8; 16],
        cursor: Option<KvListCursor>,
        limit: usize,
    ) -> Result<Vec<StoredKvDirent>> {
        kv_list(self.connection(), uid, parent, cursor, limit)
    }

    pub fn kv_dirent_at_name(
        &self,
        uid: &[u8],
        parent: &[u8; 16],
        directory_version: u64,
        name_mac: &[u8; 32],
    ) -> Result<Option<StoredKvDirent>> {
        kv_dirent_at_name(self.connection(), uid, parent, directory_version, name_mac)
    }

    pub fn kv_usage(&self, uid: &[u8]) -> Result<foks_proto::KvUsage> {
        kv_usage(self.connection(), uid)
    }

    pub fn kv_version_check(
        &self,
        uid: &[u8],
        supplied: &foks_proto::KvPathVersionVector,
    ) -> Result<KvVersionCheck> {
        kv_version_check(self.connection(), uid, supplied)
    }

    pub fn kv_version_vector(&self, uid: &[u8]) -> Result<Option<foks_proto::KvPathVersionVector>> {
        kv_version_vector(self.connection(), uid)
    }
}

fn kv_node(
    connection: &rusqlite::Connection,
    uid: &[u8],
    id: &[u8; 17],
) -> Result<Option<StoredKvNode>> {
    let stored: Option<(i64, Vec<u8>)> = connection
        .query_row(
            "SELECT node_type, exact_node FROM kv_nodes WHERE uid = ?1 AND node_id = ?2",
            params![uid, id],
            |row| Ok((row.budgeted_get(0)?, row.budgeted_get(1)?)),
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

fn kv_file(
    connection: &rusqlite::Connection,
    uid: &[u8],
    id: &[u8; 16],
) -> Result<Option<StoredKvFile>> {
    connection
        .query_row(
            "SELECT exact_metadata FROM kv_file_uploads
             WHERE uid = ?1 AND file_id = ?2 AND complete = 1 AND reclaiming = 0",
            params![uid, id],
            |row| {
                Ok(StoredKvFile {
                    exact_metadata: row.budgeted_get(0)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn kv_file_chunk(
    connection: &rusqlite::Connection,
    uid: &[u8],
    id: &[u8; 16],
    offset: u64,
) -> Result<Option<StoredKvFileChunk>> {
    let mut statement = connection.prepare(
        "SELECT c.ciphertext, c.external_digest, c.external_ciphertext_bytes,
                c.external_encoded_bytes, c.final_chunk, o.object_id,o.namespace_id,o.object_key,o.key_format
         FROM kv_file_chunks c LEFT JOIN chunk_objects o ON o.object_id=c.external_object_id
         JOIN kv_file_uploads f ON f.uid = c.uid AND f.file_id = c.file_id
         WHERE c.uid = ?1 AND c.file_id = ?2 AND c.clear_offset = ?3
           AND f.complete = 1 AND f.reclaiming = 0",
    )?;
    let mut rows = statement.query(params![uid, id, sql_integer(offset)?])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(StoredKvFileChunk {
        payload: crate::kv_objects::payload_from_row(row, 0, 5)?,
        offset,
        final_chunk: row.budgeted_get(4)?,
    }))
}

fn kv_list(
    connection: &rusqlite::Connection,
    uid: &[u8],
    parent: &[u8; 16],
    cursor: Option<KvListCursor>,
    limit: usize,
) -> Result<Vec<StoredKvDirent>> {
    if limit == 0 || limit > 4096 {
        return Err(Error::Invalid("KV list limit"));
    }
    let (query, mac, time) = match cursor {
        None => (KV_LIST_INITIAL_SQL, None, None),
        Some(KvListCursor::Mac(mac)) => (KV_LIST_MAC_SQL, Some(mac), None),
        Some(KvListCursor::Time(time)) => (KV_LIST_TIME_SQL, None, Some(time)),
    };
    let mut statement = connection.prepare(query)?;
    let limit = i64::try_from(limit).map_err(|_| Error::IntegerRange)?;
    let mut rows = match (mac, time) {
        (None, None) => statement.query(params![uid, parent, limit])?,
        (Some(mac), None) => statement.query(params![uid, parent, mac, limit])?,
        (None, Some(time)) => statement.query(params![uid, parent, sql_integer(time)?, limit])?,
        (Some(_), Some(_)) => unreachable!("KV list cursor has one arm"),
    };
    let mut stored = Vec::new();
    while let Some(row) = rows.next()? {
        let exact = row.budgeted_get::<_, Vec<u8>>(0)?;
        let node_id = row.budgeted_get::<_, Vec<u8>>(1)?;
        stored.push(StoredKvDirent {
            exact,
            node_id: node_id
                .try_into()
                .map_err(|_| Error::Invalid("stored KV node ID"))?,
            exact_directory: None,
        });
    }
    Ok(stored)
}

fn kv_dirent(
    connection: &rusqlite::Connection,
    uid: &[u8],
    parent: &[u8; 16],
    id: &[u8; 16],
) -> Result<Option<StoredKvDirent>> {
    connection
        .query_row(
            "SELECT d.exact_dirent, d.node_id FROM kv_dirent_heads h
             JOIN kv_dirents d ON d.uid = h.uid AND d.parent_id = h.parent_id
               AND d.dirent_id = h.dirent_id AND d.version = h.version
             WHERE h.uid = ?1 AND h.parent_id = ?2 AND h.dirent_id = ?3",
            params![uid, parent, id],
            |row| {
                Ok((
                    row.budgeted_get::<_, Vec<u8>>(0)?,
                    row.budgeted_get::<_, Vec<u8>>(1)?,
                ))
            },
        )
        .optional()?
        .map(|(exact, node_id)| {
            Ok(StoredKvDirent {
                exact,
                node_id: node_id
                    .try_into()
                    .map_err(|_| Error::Invalid("stored KV node ID"))?,
                exact_directory: None,
            })
        })
        .transpose()
}

/// Shared key selection; exact native payloads are fetched in this same statement.
/// The private hosted catalog wraps the identical selection with separate bounds.
pub const KV_HISTORICAL_NAME_SELECTION_SQL: &str = "SELECT e.dirent_id,e.version FROM kv_dirents e INDEXED BY kv_dirent_historical_name JOIN kv_directories d ON d.uid=e.uid AND d.directory_id=e.parent_id AND d.version=e.directory_version WHERE e.uid=?1 AND e.parent_id=?2 AND e.name_mac=?3 AND e.directory_version=CAST(?4 AS INTEGER) ORDER BY e.version DESC,e.dirent_id ASC LIMIT 1";
static KV_HISTORICAL_NAME_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("WITH selected AS ({}) SELECT e.exact_dirent,e.node_id,d.exact_directory FROM selected s JOIN kv_dirents e ON e.uid=?1 AND e.parent_id=?2 AND e.dirent_id=s.dirent_id AND e.version=s.version JOIN kv_directories d ON d.uid=e.uid AND d.directory_id=e.parent_id AND d.version=e.directory_version",KV_HISTORICAL_NAME_SELECTION_SQL)
});

fn kv_dirent_at_name(
    connection: &rusqlite::Connection,
    uid: &[u8],
    parent: &[u8; 16],
    directory_version: u64,
    name_mac: &[u8; 32],
) -> Result<Option<StoredKvDirent>> {
    let stored: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = connection
        .query_row(
            KV_HISTORICAL_NAME_SQL.as_str(),
            params![uid, parent, name_mac, sql_integer(directory_version)?],
            |row| {
                Ok((
                    row.budgeted_get(0)?,
                    row.budgeted_get(1)?,
                    row.budgeted_get(2)?,
                ))
            },
        )
        .optional()?;
    stored
        .map(|row| {
            let (exact, node_id, exact_directory) = row;
            Ok(StoredKvDirent {
                exact,
                node_id: node_id
                    .try_into()
                    .map_err(|_| Error::Invalid("stored KV node ID"))?,
                exact_directory: Some(exact_directory),
            })
        })
        .transpose()
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
                 WHERE uid = ?1 AND file_id = ?2 AND reclaiming = 0",
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
            |row| row.budgeted_get(0),
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
                row.budgeted_get::<_, Vec<u8>>(0)?,
                row.budgeted_get::<_, i64>(1)?,
                row.budgeted_get::<_, Vec<u8>>(2)?,
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

fn kv_version_check(
    connection: &rusqlite::Connection,
    uid: &[u8],
    supplied: &foks_proto::KvPathVersionVector,
) -> Result<KvVersionCheck> {
    let Some(root) = kv_root(connection, uid)? else {
        return Ok(KvVersionCheck::Missing);
    };
    if root.version < supplied.root_version {
        return Ok(KvVersionCheck::Future);
    }
    let mut stale_directories = Vec::new();
    for directory in &supplied.directories {
        let current: Option<i64> = connection
            .query_row(
                "SELECT version FROM kv_directory_heads
                 WHERE uid = ?1 AND directory_id = ?2",
                params![uid, directory.id],
                |row| row.budgeted_get(0),
            )
            .optional()?;
        let Some(current) = current else {
            return Ok(KvVersionCheck::Missing);
        };
        let current = crate::error::unsigned(current)?;
        if current < directory.version {
            return Ok(KvVersionCheck::Future);
        }
        let mut stale_entries = Vec::new();
        for entry in &directory.entries {
            let current: Option<i64> = connection
                .query_row(
                    "SELECT version FROM kv_dirent_heads
                     WHERE uid = ?1 AND parent_id = ?2 AND dirent_id = ?3",
                    params![uid, directory.id, entry.id],
                    |row| row.budgeted_get(0),
                )
                .optional()?;
            let Some(current) = current else {
                return Ok(KvVersionCheck::Missing);
            };
            let current = crate::error::unsigned(current)?;
            if current < entry.version {
                return Ok(KvVersionCheck::Future);
            }
            if current > entry.version {
                stale_entries.push(foks_proto::KvDirentVersion {
                    id: entry.id,
                    version: current,
                });
            }
        }
        if current > directory.version || !stale_entries.is_empty() {
            stale_directories.push(foks_proto::KvDirectoryVersion {
                id: directory.id,
                version: current,
                entries: stale_entries,
            });
        }
    }
    if (supplied.root_version == 0 || root.version == supplied.root_version)
        && stale_directories.is_empty()
    {
        return Ok(KvVersionCheck::Current);
    }
    Ok(KvVersionCheck::Stale(foks_proto::KvPathVersionVector {
        root_version: root.version,
        directories: stale_directories,
    }))
}

/// Public ciphertext Usage, distinct from quota accounting. Canonical text
/// integers share the exact SQL with the lossless native/Wasm catalog ABI.
pub const KV_PUBLIC_USAGE_SQL: &str = "SELECT CAST(usage_small_count AS TEXT),CAST(usage_small_bytes AS TEXT),CAST(usage_large_count AS TEXT),CAST(usage_large_bytes AS TEXT),CAST(usage_large_chunks AS TEXT),CAST(usage_unvalidated_small_count AS TEXT) FROM kv_namespaces WHERE namespace_id=?1";

fn kv_usage(connection: &rusqlite::Connection, uid: &[u8]) -> Result<foks_proto::KvUsage> {
    let values: Option<[String; 6]> = connection
        .query_row(KV_PUBLIC_USAGE_SQL, [uid], |row| {
            Ok([
                row.budgeted_get(0)?,
                row.budgeted_get(1)?,
                row.budgeted_get(2)?,
                row.budgeted_get(3)?,
                row.budgeted_get(4)?,
                row.budgeted_get(5)?,
            ])
        })
        .optional()?;
    let mut values_unsigned = [0u64; 6];
    if let Some(values) = values {
        for (out, value) in values_unsigned.iter_mut().zip(values) {
            let signed = value
                .parse::<i64>()
                .map_err(|_| Error::Invalid("stored KV usage integer"))?;
            if signed.to_string() != value {
                return Err(Error::Invalid("stored KV usage integer"));
            }
            *out = crate::error::unsigned(signed)?;
        }
    }
    if values_unsigned[5] != 0 {
        return Err(Error::Invalid("stored KV small node"));
    }
    Ok(foks_proto::KvUsage {
        small: foks_proto::KvUsageStats {
            number: values_unsigned[0],
            bytes: values_unsigned[1],
        },
        large: foks_proto::KvChunkedUsageStats {
            base: foks_proto::KvUsageStats {
                number: values_unsigned[2],
                bytes: values_unsigned[3],
            },
            chunks: values_unsigned[4],
        },
    })
}

fn stamp_node_usage(
    connection: &rusqlite::Connection,
    mutation: &KvNodeMutation<'_>,
) -> Result<()> {
    let length = foks_proto::KvSmallFileBox::ciphertext_length(mutation.exact)
        .map_err(|_| Error::Invalid("stored KV small node"))?;
    if connection.execute("UPDATE kv_nodes SET ciphertext_bytes=?4 WHERE uid=?1 AND node_id=?2 AND exact_node=?3 AND ciphertext_bytes IS NULL",
        params![mutation.uid,mutation.id,mutation.exact,sql_integer(length)?])? != 1 {
        return Err(Error::Invalid("KV node usage stamp"));
    }
    Ok(())
}

fn kv_root(connection: &rusqlite::Connection, uid: &[u8]) -> Result<Option<StoredKvRoot>> {
    let stored: Option<(i64, Vec<u8>, i64, Vec<u8>)> = connection
        .query_row(
            "SELECT root_version, directory_id, directory_version, exact_root
             FROM kv_roots WHERE uid = ?1",
            [uid],
            |row| {
                Ok((
                    row.budgeted_get(0)?,
                    row.budgeted_get(1)?,
                    row.budgeted_get(2)?,
                    row.budgeted_get(3)?,
                ))
            },
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
            |row| {
                Ok((
                    row.budgeted_get(0)?,
                    row.budgeted_get(1)?,
                    row.budgeted_get(2)?,
                ))
            },
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
    let (stored_bytes, stored_objects): (i64, i64) =
        connection.query_row(KV_NAMESPACE_USAGE_SQL, [uid], |row| {
            Ok((row.budgeted_get(0)?, row.budgeted_get(1)?))
        })?;
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

fn ensure_kv_tree_capacity(
    connection: &rusqlite::Connection,
    config: &crate::Config,
    uid: &[u8],
    added_directories: u64,
    added_dirents: u64,
) -> Result<()> {
    let (directories, dirents): (i64, i64) =
        connection.query_row(KV_TREE_USAGE_SQL, [uid], |row| {
            Ok((row.budgeted_get(0)?, row.budgeted_get(1)?))
        })?;
    let next_directories = crate::error::unsigned(directories)?
        .checked_add(added_directories)
        .ok_or(Error::IntegerRange)?;
    let next_dirents = crate::error::unsigned(dirents)?
        .checked_add(added_dirents)
        .ok_or(Error::IntegerRange)?;
    if next_directories > config.maximum_kv_directories || next_dirents > config.maximum_kv_dirents
    {
        return Err(Error::QuotaExceeded);
    }
    Ok(())
}

pub(crate) fn validate_kv_tree_capacity(
    connection: &rusqlite::Connection,
    config: &crate::Config,
) -> Result<()> {
    let invalid: i64 = connection.query_row(
        "SELECT EXISTS(
             SELECT uid FROM kv_directory_heads
             GROUP BY uid HAVING count(*) > ?1
         ) OR EXISTS(
             SELECT uid FROM kv_dirent_heads
             GROUP BY uid HAVING count(*) > ?2
         ) OR EXISTS(
             SELECT 1 FROM kv_nodes WHERE length(exact_node) > ?3
         ) OR EXISTS(
             SELECT 1 FROM kv_dirents WHERE length(exact_dirent) > ?4
         )",
        params![
            sql_integer(config.maximum_kv_directories)?,
            sql_integer(config.maximum_kv_dirents)?,
            i64::try_from(config.maximum_kv_node_bytes).map_err(|_| Error::IntegerRange)?,
            i64::try_from(config.maximum_kv_dirent_bytes).map_err(|_| Error::IntegerRange)?,
        ],
        |row| row.budgeted_get(0),
    )?;
    if invalid != 0 {
        return Err(Error::QuotaExceeded);
    }
    Ok(())
}

const KV_UPLOAD_USAGE_SQL: &str = "SELECT chunk_count,
                    (SELECT clear_offset FROM kv_file_chunks
                     WHERE uid = ?1 AND file_id = ?2 ORDER BY clear_offset DESC LIMIT 1),
                    chunk_bytes
                 FROM kv_file_uploads WHERE uid = ?1 AND file_id = ?2";
const KV_NAMESPACE_USAGE_SQL: &str =
    "SELECT accounted_bytes, accounted_objects FROM kv_namespaces WHERE namespace_id = ?1";
const KV_TREE_USAGE_SQL: &str =
    "SELECT directory_count, dirent_count FROM kv_namespaces WHERE namespace_id = ?1";

#[cfg(test)]
mod accounting_scaling_tests {
    use super::*;
    use rusqlite::{Connection, StatementStatus};

    #[test]
    fn production_usage_queries_have_bounded_vm_work_with_retained_history() {
        let mut c = Connection::open_in_memory().unwrap();
        crate::schema::initialize(&mut c).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON;
            INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33));
            INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'01');
            INSERT INTO kv_directory_heads VALUES(zeroblob(33),zeroblob(16),1);
            INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(zeroblob(33),zeroblob(16),X'01',0,0);").unwrap();
        let mut costs = Vec::new();
        for size in [1_i64, 100, 10000] {
            let tx = c.transaction().unwrap();
            for version in 1..=size {
                // One current dirent, many retained historical versions.
                tx.execute("INSERT OR IGNORE INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),zeroblob(16),?1,1,zeroblob(17),zeroblob(32),0,X'01')", [version]).unwrap();
            }
            tx.execute("INSERT INTO kv_dirent_heads VALUES(zeroblob(33),zeroblob(16),zeroblob(16),?1) ON CONFLICT(uid,parent_id,dirent_id) DO UPDATE SET version=excluded.version", [size]).unwrap();
            // Grow live head cardinality too: the prior count(heads) queries
            // must fail this guard even though history has just one main head.
            for number in 1..=size {
                let mut id = [0_u8; 16];
                id[..8].copy_from_slice(&number.to_be_bytes());
                tx.execute(
                    "INSERT OR IGNORE INTO kv_directories VALUES(zeroblob(33),?1,1,3,0,1,0,X'01')",
                    [id],
                )
                .unwrap();
                tx.execute(
                    "INSERT OR IGNORE INTO kv_directory_heads VALUES(zeroblob(33),?1,1)",
                    [id],
                )
                .unwrap();
                tx.execute("INSERT OR IGNORE INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),?1,1,1,zeroblob(17),zeroblob(32),0,X'01')", [id]).unwrap();
                tx.execute(
                    "INSERT OR IGNORE INTO kv_dirent_heads VALUES(zeroblob(33),zeroblob(16),?1,1)",
                    [id],
                )
                .unwrap();
            }

            for offset in 0..size.min(512) {
                tx.execute("INSERT OR IGNORE INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk) VALUES(zeroblob(33),zeroblob(16),?1,X'01',0,X'02')", [offset]).unwrap();
            }
            tx.commit().unwrap();
            let mut current = Vec::new();
            // These are the exact private constants used by admission above;
            // reverting the production queries to aggregate scans fails here.
            for sql in [
                KV_NAMESPACE_USAGE_SQL,
                KV_TREE_USAGE_SQL,
                KV_UPLOAD_USAGE_SQL,
            ] {
                let mut statement = c.prepare(sql).unwrap();
                if sql == KV_UPLOAD_USAGE_SQL {
                    let _: i64 = statement
                        .query_row(params![[0_u8; 33], [0_u8; 16]], |r| r.budgeted_get(0))
                        .unwrap();
                } else {
                    let _: i64 = statement
                        .query_row([[0_u8; 33]], |r| r.budgeted_get(0))
                        .unwrap();
                }
                current.push(statement.get_status(StatementStatus::VmStep));
            }
            costs.push(current);
        }
        assert_eq!(costs[0], costs[1]);
        assert_eq!(costs[1], costs[2]);
    }
}

#[cfg(test)]
#[path = "kv_current_tests.rs"]
mod current_tests;

#[cfg(test)]
#[path = "kv_historical_tests.rs"]
mod historical_tests;

#[cfg(test)]
#[path = "kv_usage_tests.rs"]
mod usage_tests;
