use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension as _, TransactionBehavior};

use foks_proto::{KvDirectoryVersion, KvDirentVersion, KvPathVersionVector};

use crate::soft_schema::{APPLICATION_ID, INITIAL, VERSION};
use crate::{sqlite_integer, stored_unsigned, Acceptance, Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvProjectedEntry {
    pub dirent_id: [u8; 16],
    pub node_id: [u8; 17],
    pub version: u64,
    pub directory_version: u64,
    pub name: Vec<u8>,
    pub write_role_type: u64,
    pub write_role_visibility: i64,
    pub creation_time: u64,
    pub dirent_bytes: Vec<u8>,
    pub node_bytes: Option<Vec<u8>>,
    pub content: Option<Vec<u8>>,
    pub symlink: Option<Vec<u8>>,
    pub large_file_size: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvDirectoryProjection {
    pub host_id: Vec<u8>,
    pub party_id: Vec<u8>,
    pub root_version: u64,
    pub root_directory_id: [u8; 16],
    pub root_bytes: Vec<u8>,
    pub directory_id: [u8; 16],
    pub directory_version: u64,
    pub directory_bytes: Vec<u8>,
    pub entries: Vec<KvProjectedEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KvLargeFileStage {
    id: i64,
    pub node_id: [u8; 17],
    pub size: u64,
}

pub struct SoftStateStore {
    connection: Connection,
    owned_stages: std::collections::BTreeSet<i64>,
}

impl SoftStateStore {
    pub fn open(path: &Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(Error::SymlinkDatabase);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(path)?.permissions().mode();
            if mode & 0o077 != 0 {
                return Err(Error::InsecureSoftPermissions(mode & 0o777));
            }
        }
        let database_path = path.canonicalize()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = Connection::open_with_flags(database_path, flags)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "trusted_schema", false)?;
        connection.pragma_update(None, "secure_delete", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        initialize(&mut connection)?;
        Ok(Self {
            connection,
            owned_stages: std::collections::BTreeSet::new(),
        })
    }

    /// Starts a durable, initially invisible large-file download. Chunks are
    /// committed independently so callers never need to retain the complete
    /// plaintext in memory. The stage becomes visible only when a verified
    /// directory projection references it.
    pub fn begin_large_file(
        &mut self,
        host_id: &[u8],
        party_id: &[u8],
        node_id: [u8; 17],
    ) -> Result<KvLargeFileStage> {
        if host_id.len() != 33 || party_id.len() != 33 || node_id[0] != 2 {
            return Err(Error::InvalidKvProjection);
        }
        self.connection.execute(
            "INSERT INTO kv_large_files (host_id, party_id, node_id, size, complete) \
             VALUES (?1, ?2, ?3, 0, 0)",
            params![host_id, party_id, node_id.as_slice()],
        )?;
        let id = self.connection.last_insert_rowid();
        self.owned_stages.insert(id);
        Ok(KvLargeFileStage {
            id,
            node_id,
            size: 0,
        })
    }

    pub fn append_large_file(
        &mut self,
        stage: &mut KvLargeFileStage,
        plaintext: &[u8],
    ) -> Result<()> {
        let stored = self
            .connection
            .query_row(
                "SELECT size, complete FROM kv_large_files WHERE id = ?1",
                [stage.id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((size, complete)) = stored else {
            return Err(Error::InvalidKvProjection);
        };
        let size = stored_unsigned("KV staged file size", size)?;
        if complete != 0 || size != stage.size {
            return Err(Error::InvalidKvProjection);
        }
        if plaintext.is_empty() {
            return Ok(());
        }
        let new_size = size
            .checked_add(
                u64::try_from(plaintext.len()).map_err(|_| Error::IntegerOutOfRange {
                    field: "KV staged chunk size",
                    value: u64::MAX,
                })?,
            )
            .ok_or(Error::IntegerOutOfRange {
                field: "KV staged file size",
                value: u64::MAX,
            })?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO kv_large_file_chunks (file_id, offset, content) VALUES (?1, ?2, ?3)",
            params![
                stage.id,
                sqlite_integer("KV chunk offset", size)?,
                plaintext
            ],
        )?;
        transaction.execute(
            "UPDATE kv_large_files SET size = ?2 WHERE id = ?1 AND complete = 0",
            params![stage.id, sqlite_integer("KV staged file size", new_size)?],
        )?;
        transaction.commit()?;
        stage.size = new_size;
        Ok(())
    }

    pub fn finish_large_file(&mut self, stage: &KvLargeFileStage) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE kv_large_files SET complete = 1 WHERE id = ?1 AND size = ?2 AND complete = 0",
            params![stage.id, sqlite_integer("KV staged file size", stage.size)?],
        )?;
        if changed != 1 {
            return Err(Error::InvalidKvProjection);
        }
        Ok(())
    }

    pub fn discard_large_files(&mut self, stages: &[KvLargeFileStage]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        for stage in stages {
            transaction.execute("DELETE FROM kv_large_files WHERE id = ?1", [stage.id])?;
        }
        transaction.commit()?;
        for stage in stages {
            self.owned_stages.remove(&stage.id);
        }
        Ok(())
    }

    /// Reclaims invisible stages left by a process crash. The caller must
    /// ensure no other `SoftStateStore` is actively downloading into this
    /// database while this maintenance operation runs.
    pub fn reclaim_orphaned_large_files(&mut self) -> Result<usize> {
        if !self.owned_stages.is_empty() {
            return Err(Error::KvProjectionConflict(
                "cannot reclaim while this store owns active large-file stages",
            ));
        }
        Ok(self.connection.execute(
            "DELETE FROM kv_large_files WHERE NOT EXISTS (\
             SELECT 1 FROM kv_entries WHERE kv_entries.large_file_id = kv_large_files.id)",
            [],
        )?)
    }

    /// Atomically replaces one verified directory projection. Advancing the
    /// root invalidates every cached directory for that party before the new
    /// projection is installed.
    pub fn project_directory(&mut self, snapshot: &KvDirectoryProjection) -> Result<Acceptance> {
        self.project_tree(std::slice::from_ref(snapshot))
    }

    /// Atomically replaces a complete verified tree projection. Every
    /// directory must describe the same party and root; a failure leaves the
    /// previously cached tree untouched.
    pub fn project_tree(&mut self, snapshots: &[KvDirectoryProjection]) -> Result<Acceptance> {
        self.project_tree_with_large_files(snapshots, &[])
    }

    pub fn project_tree_with_large_files(
        &mut self,
        snapshots: &[KvDirectoryProjection],
        large_files: &[KvLargeFileStage],
    ) -> Result<Acceptance> {
        self.project_tree_impl(snapshots, large_files, false)
    }

    /// Replaces verified stale directories and removes only directory/file
    /// rows no longer reachable from the persisted root.
    pub fn project_reachable_tree_with_large_files(
        &mut self,
        snapshots: &[KvDirectoryProjection],
        large_files: &[KvLargeFileStage],
    ) -> Result<Acceptance> {
        self.project_tree_impl(snapshots, large_files, true)
    }

    fn project_tree_impl(
        &mut self,
        snapshots: &[KvDirectoryProjection],
        large_files: &[KvLargeFileStage],
        prune: bool,
    ) -> Result<Acceptance> {
        let Some(root) = snapshots.first() else {
            return Err(Error::InvalidKvProjection);
        };
        let large_file_count = large_files.len();
        let large_files = large_files
            .iter()
            .map(|stage| (stage.node_id, *stage))
            .collect::<std::collections::BTreeMap<_, _>>();
        if large_files.len() != large_file_count {
            return Err(Error::InvalidKvProjection);
        }
        let mut directory_ids = std::collections::BTreeSet::new();
        let mut used_large_files = std::collections::BTreeSet::new();
        for snapshot in snapshots {
            validate(snapshot)?;
            if snapshot.host_id != root.host_id
                || snapshot.party_id != root.party_id
                || snapshot.root_version != root.root_version
                || snapshot.root_directory_id != root.root_directory_id
                || snapshot.root_bytes != root.root_bytes
                || !directory_ids.insert(snapshot.directory_id)
            {
                return Err(Error::InvalidKvProjection);
            }
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut previous_large_files = std::collections::BTreeSet::new();
        {
            let mut statement = transaction.prepare(
                "SELECT DISTINCT e.large_file_id FROM kv_entries e WHERE e.host_id = ?1 \
                 AND e.party_id = ?2 AND e.large_file_id IS NOT NULL",
            )?;
            previous_large_files.extend(
                statement
                    .query_map(params![root.host_id, root.party_id], |row| {
                        row.get::<_, i64>(0)
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            );
        }
        let stored_root = transaction
            .query_row(
                "SELECT root_version, root_dir_id, root_bytes FROM kv_parties \
                 WHERE host_id = ?1 AND party_id = ?2",
                params![root.host_id, root.party_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        let mut acceptance = Acceptance::Unchanged;
        match stored_root {
            None => {
                transaction.execute(
                    "INSERT INTO kv_parties (host_id, party_id, root_version, root_dir_id, root_bytes) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        root.host_id,
                        root.party_id,
                        sqlite_integer("KV root version", root.root_version)?,
                        root.root_directory_id.as_slice(),
                        root.root_bytes,
                    ],
                )?;
                acceptance = Acceptance::Inserted;
            }
            Some((version, root_id, root_bytes)) => {
                let version = stored_unsigned("KV root version", version)?;
                if root.root_version < version {
                    return Err(Error::KvRootRollback {
                        stored: version,
                        received: root.root_version,
                    });
                }
                if root.root_version == version {
                    if root_id != root.root_directory_id || root_bytes != root.root_bytes {
                        return Err(Error::KvProjectionConflict(
                            "root changed at the same version",
                        ));
                    }
                } else {
                    transaction.execute(
                        "DELETE FROM kv_parties WHERE host_id = ?1 AND party_id = ?2",
                        params![root.host_id, root.party_id],
                    )?;
                    transaction.execute(
                        "INSERT INTO kv_parties (host_id, party_id, root_version, root_dir_id, root_bytes) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            root.host_id,
                            root.party_id,
                            sqlite_integer("KV root version", root.root_version)?,
                            root.root_directory_id.as_slice(),
                            root.root_bytes,
                        ],
                    )?;
                    acceptance = Acceptance::Advanced;
                }
            }
        }

        for snapshot in snapshots {
            let stored_directory = load_directory(
                &transaction,
                &snapshot.host_id,
                &snapshot.party_id,
                &snapshot.directory_id,
            )?;
            if let Some(stored) = stored_directory {
                if snapshot.directory_version < stored.directory_version {
                    return Err(Error::KvDirectoryRollback {
                        stored: stored.directory_version,
                        received: snapshot.directory_version,
                    });
                }
                if snapshot.directory_version == stored.directory_version {
                    if stored == *snapshot {
                        continue;
                    }
                    return Err(Error::KvProjectionConflict(
                        "directory changed at the same version",
                    ));
                }
                if acceptance == Acceptance::Unchanged {
                    acceptance = Acceptance::Advanced;
                }
            } else if acceptance == Acceptance::Unchanged {
                acceptance = Acceptance::Advanced;
            }

            transaction.execute(
                "INSERT INTO kv_directories (host_id, party_id, dir_id, version, dir_bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(host_id, party_id, dir_id) DO UPDATE SET \
                 version = excluded.version, dir_bytes = excluded.dir_bytes",
                params![
                    snapshot.host_id,
                    snapshot.party_id,
                    snapshot.directory_id.as_slice(),
                    sqlite_integer("KV directory version", snapshot.directory_version)?,
                    snapshot.directory_bytes,
                ],
            )?;
            transaction.execute(
                "DELETE FROM kv_entries WHERE host_id = ?1 AND party_id = ?2 AND parent_dir_id = ?3",
                params![
                    snapshot.host_id,
                    snapshot.party_id,
                    snapshot.directory_id.as_slice()
                ],
            )?;
            for entry in &snapshot.entries {
                let large_file_id = if entry.node_id[0] == 2 {
                    let stage = large_files
                        .get(&entry.node_id)
                        .ok_or(Error::InvalidKvProjection)?;
                    let valid = transaction
                        .query_row(
                            "SELECT 1 FROM kv_large_files WHERE id = ?1 AND host_id = ?2 AND \
                             party_id = ?3 AND node_id = ?4 AND size = ?5 AND complete = 1",
                            params![
                                stage.id,
                                snapshot.host_id,
                                snapshot.party_id,
                                stage.node_id.as_slice(),
                                sqlite_integer("KV staged file size", stage.size)?,
                            ],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some();
                    if !valid || entry.large_file_size != Some(stage.size) {
                        return Err(Error::InvalidKvProjection);
                    }
                    used_large_files.insert(stage.id);
                    Some(stage.id)
                } else {
                    None
                };
                transaction.execute(
                    "INSERT INTO kv_entries (host_id, party_id, parent_dir_id, dirent_id, node_id, \
                     version, dir_version, name, write_role_type, write_role_visibility, creation_time, \
                     dirent_bytes, node_bytes, content, symlink, large_file_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        snapshot.host_id,
                        snapshot.party_id,
                        snapshot.directory_id.as_slice(),
                        entry.dirent_id.as_slice(),
                        entry.node_id.as_slice(),
                        sqlite_integer("KV dirent version", entry.version)?,
                        sqlite_integer("KV dirent directory version", entry.directory_version)?,
                        entry.name,
                        sqlite_integer("KV write role", entry.write_role_type)?,
                        entry.write_role_visibility,
                        sqlite_integer("KV creation time", entry.creation_time)?,
                        entry.dirent_bytes,
                        entry.node_bytes,
                        entry.content,
                        entry.symlink,
                        large_file_id,
                    ],
                )?;
            }
        }
        if used_large_files.len() != large_files.len() {
            return Err(Error::InvalidKvProjection);
        }
        if prune {
            prune_unreachable(
                &transaction,
                &root.host_id,
                &root.party_id,
                &root.root_directory_id,
            )?;
            ensure_complete_tree(
                &transaction,
                &root.host_id,
                &root.party_id,
                &root.root_directory_id,
            )?;
        }
        transaction.execute(
            "UPDATE kv_parties SET cache_complete = ?3 WHERE host_id = ?1 AND party_id = ?2",
            params![root.host_id, root.party_id, i64::from(prune)],
        )?;
        for file_id in previous_large_files {
            transaction.execute(
                "DELETE FROM kv_large_files WHERE id = ?1 AND NOT EXISTS (\
                 SELECT 1 FROM kv_entries WHERE kv_entries.large_file_id = kv_large_files.id)",
                [file_id],
            )?;
        }
        transaction.commit()?;
        for file_id in used_large_files {
            self.owned_stages.remove(&file_id);
        }
        Ok(acceptance)
    }

    pub fn directory(
        &self,
        host_id: &[u8],
        party_id: &[u8],
        directory_id: &[u8; 16],
    ) -> Result<Option<KvDirectoryProjection>> {
        load_directory(&self.connection, host_id, party_id, directory_id)
    }

    pub fn tree(&self, host_id: &[u8], party_id: &[u8]) -> Result<Vec<KvDirectoryProjection>> {
        let mut statement = self.connection.prepare(
            "SELECT dir_id FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 ORDER BY dir_id",
        )?;
        let ids = statement
            .query_map(params![host_id, party_id], |row| row.get::<_, Vec<u8>>(0))?
            .map(|row| row?.try_into().map_err(|_| Error::InvalidKvProjection))
            .collect::<Result<Vec<[u8; 16]>>>()?;
        ids.into_iter()
            .map(|id| {
                load_directory(&self.connection, host_id, party_id, &id)?
                    .ok_or(Error::InvalidKvProjection)
            })
            .collect()
    }

    /// Reconstructs the exact cache precondition from durable projected
    /// versions. Returning `None` means this party has no complete cache.
    pub fn version_vector(
        &self,
        host_id: &[u8],
        party_id: &[u8],
    ) -> Result<Option<KvPathVersionVector>> {
        let root_version = self
            .connection
            .query_row(
                "SELECT root_version FROM kv_parties WHERE host_id = ?1 AND party_id = ?2 \
                 AND cache_complete = 1",
                params![host_id, party_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(root_version) = root_version else {
            return Ok(None);
        };
        let mut directories = Vec::new();
        let mut directory_statement = self.connection.prepare(
            "SELECT dir_id, version FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 \
             ORDER BY dir_id",
        )?;
        let rows = directory_statement.query_map(params![host_id, party_id], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (id, version) = row?;
            let id: [u8; 16] = id.try_into().map_err(|_| Error::InvalidKvProjection)?;
            let mut entry_statement = self.connection.prepare(
                "SELECT dirent_id, version FROM kv_entries WHERE host_id = ?1 AND party_id = ?2 \
                 AND parent_dir_id = ?3 ORDER BY dirent_id",
            )?;
            let entries = entry_statement
                .query_map(params![host_id, party_id, id.as_slice()], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
                })?
                .map(|row| {
                    let (id, version) = row?;
                    Ok(KvDirentVersion {
                        id: id.try_into().map_err(|_| Error::InvalidKvProjection)?,
                        version: stored_unsigned("KV dirent version", version)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            directories.push(KvDirectoryVersion {
                id,
                version: stored_unsigned("KV directory version", version)?,
                entries,
            });
        }
        Ok(Some(KvPathVersionVector {
            root_version: stored_unsigned("KV root version", root_version)?,
            directories,
        }))
    }

    /// Streams a projected large file to `writer`, reading at most one
    /// plaintext chunk from SQLite at a time.
    pub fn write_large_file<W: Write>(
        &self,
        host_id: &[u8],
        party_id: &[u8],
        node_id: &[u8; 17],
        writer: &mut W,
    ) -> Result<Option<u64>> {
        let file = self
            .connection
            .query_row(
                "SELECT f.id, f.size FROM kv_large_files f WHERE f.host_id = ?1 AND f.party_id = ?2 \
                 AND f.node_id = ?3 AND f.complete = 1 AND EXISTS (SELECT 1 FROM kv_entries e \
                 WHERE e.large_file_id = f.id) ORDER BY f.id DESC LIMIT 1",
                params![host_id, party_id, node_id.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((file_id, size)) = file else {
            return Ok(None);
        };
        let size = stored_unsigned("KV large file size", size)?;
        let mut offset = 0u64;
        let mut statement = self.connection.prepare(
            "SELECT offset, content FROM kv_large_file_chunks WHERE file_id = ?1 ORDER BY offset",
        )?;
        let mut rows = statement.query([file_id])?;
        while let Some(row) = rows.next()? {
            let stored_offset = stored_unsigned("KV chunk offset", row.get(0)?)?;
            let content: Vec<u8> = row.get(1)?;
            if stored_offset != offset {
                return Err(Error::InvalidKvProjection);
            }
            writer.write_all(&content)?;
            offset = offset
                .checked_add(content.len() as u64)
                .ok_or(Error::InvalidKvProjection)?;
        }
        if offset != size {
            return Err(Error::InvalidKvProjection);
        }
        Ok(Some(size))
    }
}

impl Drop for SoftStateStore {
    fn drop(&mut self) {
        for stage in &self.owned_stages {
            let _ = self
                .connection
                .execute("DELETE FROM kv_large_files WHERE id = ?1", [stage]);
        }
    }
}

fn prune_unreachable(
    connection: &Connection,
    host_id: &[u8],
    party_id: &[u8],
    root: &[u8; 16],
) -> Result<()> {
    connection.execute(
        "WITH RECURSIVE reachable(dir_id) AS (VALUES (?3) UNION SELECT substr(e.node_id, 2, 16) \
         FROM kv_entries e JOIN reachable r ON e.parent_dir_id = r.dir_id \
         WHERE e.host_id = ?1 AND e.party_id = ?2 AND substr(e.node_id, 1, 1) = x'01') \
         DELETE FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 \
         AND dir_id NOT IN (SELECT dir_id FROM reachable)",
        params![host_id, party_id, root.as_slice()],
    )?;
    Ok(())
}

fn ensure_complete_tree(
    connection: &Connection,
    host_id: &[u8],
    party_id: &[u8],
    root: &[u8; 16],
) -> Result<()> {
    let missing: i64 = connection.query_row(
        "WITH RECURSIVE reachable(dir_id) AS (VALUES (?3) UNION SELECT substr(e.node_id, 2, 16) \
         FROM kv_entries e JOIN reachable r ON e.parent_dir_id = r.dir_id \
         WHERE e.host_id = ?1 AND e.party_id = ?2 AND substr(e.node_id, 1, 1) = x'01') \
         SELECT count(*) FROM reachable r LEFT JOIN kv_directories d ON d.host_id = ?1 \
         AND d.party_id = ?2 AND d.dir_id = r.dir_id WHERE d.dir_id IS NULL",
        params![host_id, party_id, root.as_slice()],
        |row| row.get(0),
    )?;
    if missing != 0 {
        return Err(Error::InvalidKvProjection);
    }
    Ok(())
}

fn initialize(connection: &mut Connection) -> Result<()> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id == 0 && version == 0 {
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if count != 0 {
            return Err(Error::WrongSoftApplicationId {
                found: application_id,
                expected: APPLICATION_ID,
            });
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(INITIAL)?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", VERSION)?;
        transaction.commit()?;
        return Ok(());
    }
    if application_id != APPLICATION_ID {
        return Err(Error::WrongSoftApplicationId {
            found: application_id,
            expected: APPLICATION_ID,
        });
    }
    if version != VERSION {
        return Err(Error::UnsupportedSoftSchema {
            found: version,
            supported: VERSION,
        });
    }
    Ok(())
}

fn validate(snapshot: &KvDirectoryProjection) -> Result<()> {
    if snapshot.host_id.len() != 33
        || snapshot.party_id.len() != 33
        || snapshot.root_version == 0
        || snapshot.directory_version == 0
        || snapshot.root_bytes.is_empty()
        || snapshot.directory_bytes.is_empty()
        || snapshot.entries.iter().any(|entry| {
            entry.version == 0
                || entry.directory_version == 0
                || entry.name.is_empty()
                || entry.dirent_bytes.is_empty()
                || !matches!(
                    (entry.write_role_type, entry.write_role_visibility),
                    (1, -32768..=32767) | (2 | 3, 0)
                )
                || match entry.node_id[0] {
                    1 => {
                        entry.content.is_some()
                            || entry.symlink.is_some()
                            || entry.large_file_size.is_some()
                    }
                    2 => {
                        entry.content.is_some()
                            || entry.symlink.is_some()
                            || entry.large_file_size.is_none()
                    }
                    3 => {
                        entry.content.is_none()
                            || entry.symlink.is_some()
                            || entry.large_file_size.is_some()
                    }
                    4 => {
                        entry.content.is_some()
                            || entry.symlink.is_none()
                            || entry.large_file_size.is_some()
                    }
                    _ => true,
                }
        })
    {
        return Err(Error::InvalidKvProjection);
    }
    let mut ids = snapshot
        .entries
        .iter()
        .map(|entry| entry.dirent_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut names = snapshot
        .entries
        .iter()
        .map(|entry| entry.name.as_slice())
        .collect::<Vec<_>>();
    names.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1])
        || names.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(Error::InvalidKvProjection);
    }
    Ok(())
}

fn load_directory(
    connection: &Connection,
    host_id: &[u8],
    party_id: &[u8],
    directory_id: &[u8; 16],
) -> Result<Option<KvDirectoryProjection>> {
    let root = connection
        .query_row(
            "SELECT root_version, root_dir_id, root_bytes FROM kv_parties WHERE host_id = ?1 AND party_id = ?2",
            params![host_id, party_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)),
        )
        .optional()?;
    let Some((root_version, root_directory_id, root_bytes)) = root else {
        return Ok(None);
    };
    let directory = connection
        .query_row(
            "SELECT version, dir_bytes FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 AND dir_id = ?3",
            params![host_id, party_id, directory_id.as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    let Some((directory_version, directory_bytes)) = directory else {
        return Ok(None);
    };
    let mut statement = connection.prepare(
        "SELECT e.dirent_id, e.node_id, e.version, e.dir_version, e.name, e.write_role_type, \
         e.write_role_visibility, e.creation_time, e.dirent_bytes, e.node_bytes, e.content, \
         e.symlink, f.size FROM kv_entries e LEFT JOIN kv_large_files f ON f.id = e.large_file_id \
         WHERE e.host_id = ?1 AND e.party_id = ?2 AND e.parent_dir_id = ?3 \
         ORDER BY name, dirent_id",
    )?;
    let entries = statement
        .query_map(params![host_id, party_id, directory_id.as_slice()], |row| {
            let dirent_id: Vec<u8> = row.get(0)?;
            let node_id: Vec<u8> = row.get(1)?;
            Ok((
                dirent_id,
                node_id,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Vec<u8>>(8)?,
                row.get::<_, Option<Vec<u8>>>(9)?,
                row.get::<_, Option<Vec<u8>>>(10)?,
                row.get::<_, Option<Vec<u8>>>(11)?,
                row.get::<_, Option<i64>>(12)?,
            ))
        })?
        .map(|row| {
            let row = row?;
            Ok(KvProjectedEntry {
                dirent_id: row.0.try_into().map_err(|_| Error::InvalidKvProjection)?,
                node_id: row.1.try_into().map_err(|_| Error::InvalidKvProjection)?,
                version: stored_unsigned("KV dirent version", row.2)?,
                directory_version: stored_unsigned("KV dirent directory version", row.3)?,
                name: row.4,
                write_role_type: stored_unsigned("KV write role", row.5)?,
                write_role_visibility: row.6,
                creation_time: stored_unsigned("KV creation time", row.7)?,
                dirent_bytes: row.8,
                node_bytes: row.9,
                content: row.10,
                symlink: row.11,
                large_file_size: row
                    .12
                    .map(|size| stored_unsigned("KV large file size", size))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(KvDirectoryProjection {
        host_id: host_id.to_vec(),
        party_id: party_id.to_vec(),
        root_version: stored_unsigned("KV root version", root_version)?,
        root_directory_id: root_directory_id
            .try_into()
            .map_err(|_| Error::InvalidKvProjection)?,
        root_bytes,
        directory_id: *directory_id,
        directory_version: stored_unsigned("KV directory version", directory_version)?,
        directory_bytes,
        entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u8, name: &[u8]) -> KvProjectedEntry {
        let mut node_id = [id; 17];
        node_id[0] = 3;
        KvProjectedEntry {
            dirent_id: [id; 16],
            node_id,
            version: 1,
            directory_version: 1,
            name: name.to_vec(),
            write_role_type: 2,
            write_role_visibility: 0,
            creation_time: u64::from(id),
            dirent_bytes: vec![id],
            node_bytes: Some(vec![id + 1]),
            content: Some(vec![id + 2]),
            symlink: None,
            large_file_size: None,
        }
    }

    fn snapshot() -> KvDirectoryProjection {
        KvDirectoryProjection {
            host_id: vec![2; 33],
            party_id: vec![3; 33],
            root_version: 1,
            root_directory_id: [4; 16],
            root_bytes: vec![5],
            directory_id: [4; 16],
            directory_version: 1,
            directory_bytes: vec![6],
            entries: vec![entry(1, b"a"), entry(2, b"b")],
        }
    }

    #[test]
    fn verified_projection_round_trips_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let snapshot = snapshot();
        assert_eq!(
            store.project_directory(&snapshot).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.project_directory(&snapshot).unwrap(),
            Acceptance::Unchanged
        );
        assert_eq!(
            store
                .directory(
                    &snapshot.host_id,
                    &snapshot.party_id,
                    &snapshot.directory_id
                )
                .unwrap(),
            Some(snapshot)
        );
    }

    #[test]
    fn rollback_and_same_version_conflicts_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let mut projected = snapshot();
        store.project_directory(&projected).unwrap();
        projected.root_bytes.push(9);
        assert!(matches!(
            store.project_directory(&projected),
            Err(Error::KvProjectionConflict(_))
        ));
        projected = snapshot();
        projected.directory_version = 2;
        for entry in &mut projected.entries {
            entry.directory_version = 2;
        }
        store.project_directory(&projected).unwrap();
        projected.directory_version = 1;
        for entry in &mut projected.entries {
            entry.directory_version = 1;
        }
        assert!(matches!(
            store.project_directory(&projected),
            Err(Error::KvDirectoryRollback { .. })
        ));
    }

    #[test]
    fn root_advance_invalidates_old_directories_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let old = snapshot();
        store.project_directory(&old).unwrap();
        let mut new = snapshot();
        new.root_version = 2;
        new.root_directory_id = [8; 16];
        new.root_bytes = vec![9];
        new.directory_id = [8; 16];
        new.directory_bytes = vec![10];
        assert_eq!(store.project_directory(&new).unwrap(), Acceptance::Advanced);
        assert!(store
            .directory(&old.host_id, &old.party_id, &old.directory_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .directory(&new.host_id, &new.party_id, &new.directory_id)
                .unwrap(),
            Some(new)
        );
    }

    #[test]
    fn adding_a_directory_under_the_same_root_is_an_advance() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let root = snapshot();
        store.project_directory(&root).unwrap();
        let mut child = snapshot();
        child.directory_id = [12; 16];
        child.directory_bytes = vec![13];
        assert_eq!(
            store.project_directory(&child).unwrap(),
            Acceptance::Advanced
        );
        assert!(store
            .directory(&child.host_id, &child.party_id, &child.directory_id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn tree_projection_rolls_back_every_directory_on_a_late_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let root = snapshot();
        store.project_directory(&root).unwrap();

        let mut child = snapshot();
        child.directory_id = [12; 16];
        child.directory_bytes = vec![13];
        let mut conflicting_root = root.clone();
        conflicting_root.directory_bytes.push(99);
        assert!(matches!(
            store.project_tree(&[child.clone(), conflicting_root]),
            Err(Error::KvProjectionConflict(_))
        ));

        assert!(store
            .directory(&child.host_id, &child.party_id, &child.directory_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .directory(&root.host_id, &root.party_id, &root.directory_id)
                .unwrap(),
            Some(root)
        );
    }

    #[test]
    fn version_vector_and_large_file_chunks_are_durable_and_targeted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let mut projected = snapshot();
        let mut node_id = [9; 17];
        node_id[0] = 2;
        projected.entries = vec![KvProjectedEntry {
            dirent_id: [7; 16],
            node_id,
            version: 4,
            directory_version: 1,
            name: b"large.bin".to_vec(),
            write_role_type: 2,
            write_role_visibility: 0,
            creation_time: 7,
            dirent_bytes: vec![7],
            node_bytes: Some(vec![8]),
            content: None,
            symlink: None,
            large_file_size: Some(5),
        }];
        let mut stage = store
            .begin_large_file(&projected.host_id, &projected.party_id, node_id)
            .unwrap();
        store.append_large_file(&mut stage, b"ab").unwrap();
        store.append_large_file(&mut stage, b"cde").unwrap();
        store.finish_large_file(&stage).unwrap();
        store
            .project_reachable_tree_with_large_files(&[projected.clone()], &[stage])
            .unwrap();

        let versions = store
            .version_vector(&projected.host_id, &projected.party_id)
            .unwrap()
            .unwrap();
        assert_eq!(versions.root_version, 1);
        assert_eq!(versions.directories[0].id, projected.directory_id);
        assert_eq!(versions.directories[0].entries[0].version, 4);
        drop(store);

        let mut store = SoftStateStore::open(&path).unwrap();
        let mut output = Vec::new();
        assert_eq!(
            store
                .write_large_file(
                    &projected.host_id,
                    &projected.party_id,
                    &node_id,
                    &mut output,
                )
                .unwrap(),
            Some(5)
        );
        assert_eq!(output, b"abcde");

        projected.directory_version = 2;
        projected.directory_bytes = vec![10];
        projected.entries.clear();
        store
            .project_reachable_tree_with_large_files(&[projected.clone()], &[])
            .unwrap();
        assert_eq!(
            store
                .write_large_file(
                    &projected.host_id,
                    &projected.party_id,
                    &node_id,
                    &mut Vec::new(),
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn orphan_reclamation_refuses_to_delete_a_live_owned_stage() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let projected = snapshot();
        let mut node_id = [9; 17];
        node_id[0] = 2;
        let mut stage = store
            .begin_large_file(&projected.host_id, &projected.party_id, node_id)
            .unwrap();

        assert!(matches!(
            store.reclaim_orphaned_large_files(),
            Err(Error::KvProjectionConflict(_))
        ));
        store.append_large_file(&mut stage, b"still live").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn plaintext_soft_database_must_be_private() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        std::fs::write(&path, []).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            SoftStateStore::open(&path),
            Err(Error::InsecureSoftPermissions(0o644))
        ));
    }
}
