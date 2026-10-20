//! Internal installation/storage binding; never part of the FOKS protocol.
use crate::allocation::RowAllocationExt as _;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::{Database, Error, KvChunkKey, ReadDatabase, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkStorageBindingState {
    Legacy,
    PendingNew,
    PendingAdoption,
    Active,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkStorageBinding {
    pub state: ChunkStorageBindingState,
    pub namespace_id: Option<[u8; 16]>,
    pub host_id: Option<[u8; 33]>,
    pub locator: Option<String>,
    pub authority_identity: Option<[u8; 32]>,
    pub scan_revision: u64,
    pub after: Option<KvChunkKey>,
    pub verified_objects: u64,
    pub verified_bytes: u64,
}

fn read(connection: &Connection) -> Result<Option<ChunkStorageBinding>> {
    let mut statement = connection.prepare(
        "SELECT state,namespace_id,host_id,locator,scan_revision,
        after_uid,after_file_id,after_offset,verified_objects,verified_bytes,authority_identity
        FROM chunk_storage_binding WHERE singleton=1",
    )?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let state = match row.budgeted_get::<_, i64>(0)? {
        1 => ChunkStorageBindingState::Legacy,
        2 => ChunkStorageBindingState::PendingNew,
        3 => ChunkStorageBindingState::PendingAdoption,
        4 => ChunkStorageBindingState::Active,
        _ => return Err(Error::Invalid("chunk binding state")),
    };
    let fixed = |bytes: Option<Vec<u8>>| -> Result<Option<[u8; 16]>> {
        bytes
            .map(|bytes| {
                bytes
                    .try_into()
                    .map_err(|_| Error::Invalid("chunk namespace ID"))
            })
            .transpose()
    };
    let host: Option<Vec<u8>> = row.budgeted_get(2)?;
    let uid: Vec<u8> = row.budgeted_get(5)?;
    let file: Vec<u8> = row.budgeted_get(6)?;
    let offset: i64 = row.budgeted_get(7)?;
    Ok(Some(ChunkStorageBinding {
        state,
        namespace_id: fixed(row.budgeted_get(1)?)?,
        host_id: host
            .map(|v| {
                v.try_into()
                    .map_err(|_| Error::Invalid("chunk binding host ID"))
            })
            .transpose()?,
        locator: row.budgeted_get(3)?,
        authority_identity: row
            .budgeted_get::<_, Option<Vec<u8>>>(10)?
            .map(|v| {
                v.try_into()
                    .map_err(|_| Error::Invalid("chunk authority identity"))
            })
            .transpose()?,
        scan_revision: crate::error::unsigned(row.budgeted_get(4)?)?,
        after: if uid.is_empty() && file.is_empty() && offset == -1 {
            None
        } else {
            Some(KvChunkKey {
                uid: uid
                    .try_into()
                    .map_err(|_| Error::Invalid("adoption cursor namespace"))?,
                file_id: file
                    .try_into()
                    .map_err(|_| Error::Invalid("adoption cursor file"))?,
                offset: crate::error::unsigned(offset)?,
            })
        },
        verified_objects: crate::error::unsigned(row.budgeted_get(8)?)?,
        verified_bytes: crate::error::unsigned(row.budgeted_get(9)?)?,
    }))
}

impl ReadDatabase {
    pub fn chunk_storage_binding(&self) -> Result<Option<ChunkStorageBinding>> {
        read(&self.connection)
    }
}

impl Database {
    pub fn chunk_storage_binding(&self) -> Result<Option<ChunkStorageBinding>> {
        read(&self.connection)
    }

    /// Starts or resumes a binding. Explicit adoption is required for legacy data.
    /// The caller must own the installation writer for the entire operation.
    pub fn prepare_chunk_storage_binding(
        &mut self,
        namespace_id: &[u8; 16],
        locator: &str,
        adopt: bool,
    ) -> Result<ChunkStorageBinding> {
        self.prepare_chunk_storage_binding_with_authority(namespace_id, locator, adopt, None)
    }

    /// Authority is installed only by the original fresh binding transaction.
    pub fn prepare_chunk_storage_binding_with_authority(
        &mut self,
        namespace_id: &[u8; 16],
        locator: &str,
        adopt: bool,
        authority_identity: Option<[u8; 32]>,
    ) -> Result<ChunkStorageBinding> {
        if locator.is_empty() || locator.len() > 4096 {
            return Err(Error::Invalid("chunk locator"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let host: Vec<u8> = transaction
            .query_row(
                "SELECT host_id FROM host_metadata WHERE singleton=1",
                [],
                |r| r.budgeted_get(0),
            )
            .optional()?
            .ok_or(Error::Invalid("chunk binding requires host bootstrap"))?;
        let current = read(&transaction)?;
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM chunk_storage_revision WHERE singleton=1",
            [],
            |r| r.budgeted_get(0),
        )?;
        // A binding that never became active has stored nothing, so a start
        // against another locator replaces it instead of pinning the database
        // to a store it can never use (for example the store of the
        // installation a backup was restored from).
        let replaceable = current.as_ref().is_some_and(|binding| {
            binding.state == ChunkStorageBindingState::PendingNew
                && binding.verified_objects == 0
                && binding.locator.as_deref() != Some(locator)
                && binding.host_id.as_ref().map(|v| v.as_slice()) == Some(host.as_slice())
        });
        if let Some(binding) = current.as_ref().filter(|_| !replaceable) {
            if binding.state != ChunkStorageBindingState::Legacy {
                if binding.authority_identity.is_some()
                    && binding.authority_identity != authority_identity
                {
                    return Err(Error::Invalid("chunk authority identity mismatch"));
                }
                if binding.locator.as_deref() != Some(locator)
                    || binding.host_id.as_ref().map(|v| v.as_slice()) != Some(host.as_slice())
                {
                    return Err(Error::Invalid("chunk storage binding mismatch"));
                }
                if binding.state == ChunkStorageBindingState::PendingAdoption && !adopt {
                    return Err(Error::Invalid("chunk storage adoption required"));
                }
                if binding.state == ChunkStorageBindingState::PendingAdoption
                    && binding.scan_revision != revision as u64
                {
                    transaction.execute("UPDATE chunk_storage_binding SET scan_revision=?1,after_uid=X'',after_file_id=X'',after_offset=-1,verified_objects=0,verified_bytes=0 WHERE singleton=1", [revision])?;
                }
                let result = read(&transaction)?.ok_or(Error::Invalid("missing chunk binding"))?;
                transaction.commit()?;
                return Ok(result);
            }
        }
        let external: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv_file_chunks WHERE external_digest IS NOT NULL)",
            [],
            |r| r.budgeted_get(0),
        )?;
        let legacy = external
            || current
                .as_ref()
                .is_some_and(|binding| binding.state == ChunkStorageBindingState::Legacy);
        if legacy && !adopt {
            return Err(Error::Invalid("chunk storage adoption required"));
        }
        transaction.execute("INSERT INTO chunk_storage_binding(singleton,state,namespace_id,host_id,locator,scan_revision,authority_identity)
            VALUES(1,?1,?2,?3,?4,?5,?6) ON CONFLICT(singleton) DO UPDATE SET state=excluded.state,
            namespace_id=excluded.namespace_id,host_id=excluded.host_id,locator=excluded.locator,
            scan_revision=excluded.scan_revision,authority_identity=excluded.authority_identity", params![if legacy {3} else {2}, namespace_id.as_slice(),host,locator,revision, if legacy {None} else {authority_identity}.as_ref().map(|v| v.as_slice())])?;
        let result = read(&transaction)?.ok_or(Error::Invalid("missing chunk binding"))?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn checkpoint_chunk_adoption(
        &mut self,
        expected: &ChunkStorageBinding,
        key: &KvChunkKey,
        encoded_bytes: u64,
    ) -> Result<ChunkStorageBinding> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read(&transaction)?.as_ref() != Some(expected)
            || expected.state != ChunkStorageBindingState::PendingAdoption
        {
            return Err(Error::Invalid("chunk adoption state changed"));
        }
        let (after_uid, after_file, after_offset) =
            expected.after.as_ref().map_or((&[][..], &[][..], -1), |k| {
                (k.uid.as_slice(), k.file_id.as_slice(), k.offset as i64)
            });
        let next: Option<(Vec<u8>, Vec<u8>, i64, i64)> = transaction.query_row(
            "SELECT uid,file_id,clear_offset,external_encoded_bytes FROM kv_file_chunks INDEXED BY kv_chunks_external
             WHERE external_digest IS NOT NULL AND (uid,file_id,clear_offset) > (?1,?2,?3)
             ORDER BY uid,file_id,clear_offset LIMIT 1", params![after_uid,after_file,after_offset],
            |row| Ok((row.budgeted_get(0)?,row.budgeted_get(1)?,row.budgeted_get(2)?,row.budgeted_get(3)?))).optional()?;
        if !next.is_some_and(|(uid, file, offset, bytes)| {
            uid == key.uid
                && file == key.file_id
                && offset >= 0
                && offset as u64 == key.offset
                && bytes >= 0
                && bytes as u64 == encoded_bytes
        }) {
            return Err(Error::Invalid(
                "chunk adoption cursor must advance exactly one reference",
            ));
        }
        let changed = transaction.execute(
            "UPDATE chunk_storage_binding SET after_uid=?1,after_file_id=?2,after_offset=?3,
            verified_objects=verified_objects+1,verified_bytes=verified_bytes+?4 WHERE singleton=1
            AND scan_revision=(SELECT revision FROM chunk_storage_revision WHERE singleton=1)",
            params![
                key.uid.as_slice(),
                key.file_id.as_slice(),
                crate::error::sql_integer(key.offset)?,
                crate::error::sql_integer(encoded_bytes)?
            ],
        )?;
        if changed != 1 {
            return Err(Error::Invalid(
                "chunk adoption references changed; resume to rescan",
            ));
        }
        let result = read(&transaction)?.ok_or(Error::Invalid("missing chunk binding"))?;
        transaction.commit()?;
        Ok(result)
    }

    /// Activation is CAS-protected and requires a complete revision-consistent scan.
    pub fn activate_chunk_storage_binding(&mut self, expected: &ChunkStorageBinding) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read(&transaction)?.as_ref() != Some(expected)
            || !matches!(
                expected.state,
                ChunkStorageBindingState::PendingNew | ChunkStorageBindingState::PendingAdoption
            )
        {
            return Err(Error::Invalid("chunk binding changed before activation"));
        }
        let (uid, file, offset) = expected.after.as_ref().map_or((&[][..], &[][..], -1), |k| {
            (k.uid.as_slice(), k.file_id.as_slice(), k.offset as i64)
        });
        let remaining: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv_file_chunks WHERE external_digest IS NOT NULL
            AND (uid,file_id,clear_offset) > (?1,?2,?3))",
            params![uid, file, offset],
            |r| r.budgeted_get(0),
        )?;
        if remaining {
            return Err(Error::Invalid("chunk adoption scan incomplete"));
        }
        let changed = transaction.execute(
            "UPDATE chunk_storage_binding SET state=4 WHERE singleton=1
            AND scan_revision=(SELECT revision FROM chunk_storage_revision WHERE singleton=1)",
            [],
        )?;
        if changed != 1 {
            return Err(Error::Invalid("chunk binding references changed"));
        }
        transaction.commit()?;
        Ok(())
    }

    /// Only call on a private self-contained backup/restore copy, never live state.
    pub fn clear_private_chunk_storage_binding(&mut self) -> Result<()> {
        if self.has_external_chunks()? {
            return Err(Error::Invalid("cannot clear binding with external chunks"));
        }
        self.clear_private_chunk_ledger()?;
        self.connection
            .execute("DELETE FROM chunk_storage_binding", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Database) {
        let temporary = tempfile::tempdir().unwrap();
        let db = Database::open(temporary.path().join("db"), crate::Config::default()).unwrap();
        db.connection
            .execute(
                "INSERT INTO host_metadata VALUES(1,?1,'test',X'',X'')",
                [&[1_u8; 33][..]],
            )
            .unwrap();
        db.connection
            .execute(
                "INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(?1,1,?1)",
                [&[1_u8; 33][..]],
            )
            .unwrap();
        db.connection.execute("INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(?1,?2,X'',0,0)", params![[1_u8;33].as_slice(),[2_u8;16].as_slice()]).unwrap();
        (temporary, db)
    }
    fn insert(db: &Database, offset: i64) {
        db.connection.execute("INSERT OR IGNORE INTO chunk_objects(object_id,object_key,digest,encoded_bytes,state) VALUES(?1,lower(hex(?1)),?1,2,3)",[[3_u8;32].as_slice()]).unwrap();
        db.connection.execute("INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk,external_digest,external_ciphertext_bytes,external_encoded_bytes,external_object_id)
            VALUES(?1,?2,?3,X'',0,X'',?4,1,2,?4)", params![[1_u8;33].as_slice(),[2_u8;16].as_slice(),offset,[3_u8;32].as_slice()]).unwrap();
    }
    fn key(offset: u64) -> KvChunkKey {
        KvChunkKey {
            uid: [1; 33],
            file_id: [2; 16],
            offset,
        }
    }

    #[test]
    fn binding_initialization_resumes_and_rejects_relocation() {
        let (_temp, mut db) = fixture();
        let first = db
            .prepare_chunk_storage_binding(&[4; 16], "store-a", false)
            .unwrap();
        assert_eq!(first.state, ChunkStorageBindingState::PendingNew);
        assert_eq!(
            db.prepare_chunk_storage_binding(&[5; 16], "store-a", false)
                .unwrap(),
            first
        );
        db.activate_chunk_storage_binding(&first).unwrap();
        assert!(db
            .prepare_chunk_storage_binding(&[5; 16], "store-b", false)
            .is_err());
        assert_eq!(
            db.chunk_storage_binding().unwrap().unwrap().state,
            ChunkStorageBindingState::Active
        );
        assert!(db.activate_chunk_storage_binding(&first).is_err());
        db.clear_private_chunk_storage_binding().unwrap();
        assert!(db.chunk_storage_binding().unwrap().is_none());
    }

    #[test]
    fn never_activated_binding_is_replaced_by_another_locator() {
        let (_temp, mut db) = fixture();
        let first = db
            .prepare_chunk_storage_binding_with_authority(&[4; 16], "old-store", false, None)
            .unwrap();
        assert_eq!(first.state, ChunkStorageBindingState::PendingNew);
        let replaced = db
            .prepare_chunk_storage_binding_with_authority(
                &[5; 16],
                "new-store",
                false,
                Some([6; 32]),
            )
            .unwrap();
        assert_eq!(replaced.state, ChunkStorageBindingState::PendingNew);
        assert_eq!(replaced.locator.as_deref(), Some("new-store"));
        assert_eq!(replaced.namespace_id, Some([5; 16]));
        assert_eq!(replaced.authority_identity, Some([6; 32]));
        // On its own locator the replacement keeps its recorded authority.
        assert!(db
            .prepare_chunk_storage_binding_with_authority(&[7; 16], "new-store", false, None)
            .is_err());
        // External references are never silently re-homed by a replacement.
        insert(&db, 0);
        assert!(db
            .prepare_chunk_storage_binding_with_authority(
                &[8; 16],
                "third-store",
                false,
                Some([6; 32])
            )
            .is_err());
        assert_eq!(db.chunk_storage_binding().unwrap().unwrap(), replaced);
    }

    #[test]
    fn adoption_checkpoint_revision_covers_insert_update_and_cascade() {
        let (_temp, mut db) = fixture();
        insert(&db, 0);
        assert!(db
            .prepare_chunk_storage_binding(&[4; 16], "store", false)
            .is_err());
        let first = db
            .prepare_chunk_storage_binding(&[4; 16], "store", true)
            .unwrap();
        assert!(db.activate_chunk_storage_binding(&first).is_err());
        let checked = db.checkpoint_chunk_adoption(&first, &key(0), 2).unwrap();
        insert(&db, 1);
        assert!(db.activate_chunk_storage_binding(&checked).is_err());
        let restarted = db
            .prepare_chunk_storage_binding(&[5; 16], "store", true)
            .unwrap();
        assert_eq!(restarted.namespace_id, first.namespace_id);
        assert!(restarted.after.is_none());
        assert_eq!(restarted.verified_objects, 0);
        let checked = db
            .checkpoint_chunk_adoption(&restarted, &key(0), 2)
            .unwrap();
        db.connection
            .execute(
                "UPDATE kv_file_chunks SET final_chunk=1 WHERE clear_offset=0",
                [],
            )
            .unwrap();
        assert!(db.checkpoint_chunk_adoption(&checked, &key(1), 2).is_err());
        let restarted = db
            .prepare_chunk_storage_binding(&[5; 16], "store", true)
            .unwrap();
        db.connection
            .execute("DELETE FROM kv_namespaces", [])
            .unwrap();
        assert!(db.activate_chunk_storage_binding(&restarted).is_err());
        let restarted = db
            .prepare_chunk_storage_binding(&[5; 16], "store", true)
            .unwrap();
        db.activate_chunk_storage_binding(&restarted).unwrap();
    }

    #[test]
    fn schema_50_external_migration_requires_explicit_adoption() {
        let (temp, db) = fixture();
        insert(&db, 0);
        db.connection
            .execute_batch(concat!(
                include_str!("../tests/fixtures/remove_waitlist_count.sql"),
                include_str!("../tests/fixtures/remove_read_dependency.sql"),
                include_str!("../tests/fixtures/remove_kv_usage.sql"),
                include_str!("../tests/fixtures/remove_kv_current.sql")
            ))
            .unwrap();
        db.connection
            .execute_batch(include_str!("../tests/fixtures/remove_chunk_binding.sql"))
            .unwrap();
        db.connection
            .pragma_update(None, "user_version", 50)
            .unwrap();
        drop(db);
        let mut db = Database::open(temp.path().join("db"), crate::Config::default()).unwrap();
        assert_eq!(
            db.chunk_storage_binding().unwrap().unwrap().state,
            ChunkStorageBindingState::Legacy
        );
        assert!(db
            .prepare_chunk_storage_binding(&[1; 16], "store", false)
            .is_err());
        assert!(db.clear_private_chunk_storage_binding().is_err());
        let binding = db
            .prepare_chunk_storage_binding(&[1; 16], "store", true)
            .unwrap();
        assert!(db.activate_chunk_storage_binding(&binding).is_err());
        let verified = db.checkpoint_chunk_adoption(&binding, &key(0), 2).unwrap();
        db.activate_chunk_storage_binding(&verified).unwrap();
    }
    #[test]
    fn schema_53_binding_migrates_unqualified_and_stays_unqualified() {
        let (dir, mut db) = fixture();
        let original = db
            .prepare_chunk_storage_binding(&[4; 16], "local", false)
            .unwrap();
        db.activate_chunk_storage_binding(&original).unwrap();
        db.connection
            .execute_batch(concat!(
                include_str!("../tests/fixtures/remove_waitlist_count.sql"),
                include_str!("../tests/fixtures/remove_read_dependency.sql"),
                include_str!("../tests/fixtures/remove_kv_usage.sql"),
                include_str!("../tests/fixtures/remove_kv_current.sql")
            ))
            .unwrap();
        db.connection
            .execute_batch(
                "DROP TRIGGER chunk_attempt_policy_immutable;
            DROP INDEX chunk_attempts_publication_policy;
            ALTER TABLE chunk_object_attempts DROP COLUMN publication_policy;
            ALTER TABLE chunk_storage_binding DROP COLUMN authority_identity;
            PRAGMA user_version=53",
            )
            .unwrap();
        drop(db);
        let mut db = Database::open(dir.path().join("db"), crate::Config::default()).unwrap();
        let migrated = db
            .prepare_chunk_storage_binding_with_authority(&[9; 16], "local", false, Some([2; 32]))
            .unwrap();
        assert_eq!(migrated.authority_identity, None);
        assert_eq!(migrated.namespace_id, original.namespace_id);
        assert_eq!(migrated.state, ChunkStorageBindingState::Active);
    }
}
