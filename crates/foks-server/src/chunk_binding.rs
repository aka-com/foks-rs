//! Namespace placement and optional local database authority; not a distributed fence.
use std::path::Path;

use foks_server_db::{
    ChunkStorageBinding, ChunkStorageBindingState as State, Database, KvChunkPayload, ReadDatabase,
};

use crate::chunks::ChunkStore;
use crate::{Entropy, Error, Result, WriterHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkAdoptionProgress {
    pub verified_objects: u64,
    pub verified_bytes: u64,
}

fn marker(binding: &ChunkStorageBinding) -> Result<Vec<u8>> {
    let mut bytes = b"FOKS-CHUNK-NAMESPACE\0\x01".to_vec();
    bytes.extend_from_slice(
        &binding
            .namespace_id
            .ok_or(Error::Chunk("missing chunk namespace ID"))?,
    );
    bytes.extend_from_slice(
        &binding
            .host_id
            .ok_or(Error::Chunk("missing chunk binding host"))?,
    );
    if let Some(identity) = binding.authority_identity {
        bytes[b"FOKS-CHUNK-NAMESPACE\0".len()] = 2;
        bytes.extend_from_slice(&identity);
    }
    Ok(bytes)
}

fn verify(
    binding: &ChunkStorageBinding,
    store: &dyn ChunkStore,
    host: &[u8],
    source: &crate::storage_lease::StorageLease,
) -> Result<()> {
    source.verify_authority(binding.authority_identity)?;
    if binding.state != State::Active {
        return Err(Error::Chunk(
            "chunk store adoption or initialization required",
        ));
    }
    if binding.locator.as_deref() != Some(store.locator()?.as_str())
        || binding.host_id.as_ref().map(|h| h.as_slice()) != Some(host)
    {
        return Err(Error::Chunk(
            "configured chunk store differs from database binding",
        ));
    }
    if store.read_namespace_marker()?.as_deref() != Some(marker(binding)?.as_slice()) {
        return Err(Error::Chunk(
            "chunk storage namespace marker is missing or mismatched",
        ));
    }
    Ok(())
}

/// A database with no binding, or with a never-activated binding to another
/// locator, starts a new namespace at the configured locator. Nothing can have
/// been stored under a binding that never became active, so replacing it is
/// safe; the database layer applies the same rule.
fn starts_fresh(binding: Option<&ChunkStorageBinding>, locator: &str) -> bool {
    binding.is_none_or(|binding| {
        binding.state == State::PendingNew
            && binding.verified_objects == 0
            && binding.locator.as_deref() != Some(locator)
    })
}

/// A new binding never takes over a namespace that already has a marker: it
/// belongs to another binding, typically the installation a backup was taken
/// from. Refusing before the binding row is written leaves the database free
/// to start against another store.
fn refuse_marked_namespace(store: &dyn ChunkStore) -> Result<()> {
    if store.read_namespace_marker()?.is_some() {
        return Err(Error::Chunk(FOREIGN_NAMESPACE));
    }
    Ok(())
}

const FOREIGN_NAMESPACE: &str =
    "chunk store namespace belongs to another binding; configure a new empty directory or prefix";

/// Publishes the marker of a binding that is not yet active. A marker that is
/// already present and differs belongs to another binding, for example a
/// database left PendingNew by an earlier release on the store of the
/// installation it was restored from; report the remedy rather than the bare
/// marker conflict.
fn publish_marker(store: &dyn ChunkStore, binding: &ChunkStorageBinding) -> Result<()> {
    let expected = marker(binding)?;
    if store
        .read_namespace_marker()?
        .is_some_and(|present| present != expected)
    {
        return Err(Error::Chunk(FOREIGN_NAMESPACE));
    }
    store.create_namespace_marker_if_absent(&expected)
}

/// Validate a snapshot before it can be served or hydrated. No snapshot is held
/// by this function; metadata is copied before the remote marker read.
pub(crate) fn verify_reader(
    reader: &ReadDatabase,
    store: Option<&dyn ChunkStore>,
    source: &crate::storage_lease::StorageLease,
) -> Result<()> {
    let binding = reader.chunk_storage_binding()?;
    let external = reader.has_external_chunks()?;
    match (binding, store) {
        (Some(binding), Some(store)) => {
            let host = reader
                .host_bootstrap()?
                .ok_or(Error::Chunk("chunk binding requires host bootstrap"))?;
            verify(&binding, store, &host.host_id, source)
        }
        (None, None) if !external => Ok(()),
        (None, Some(_)) => Err(Error::Chunk(
            "chunk store has not been bound to this database",
        )),
        _ => Err(Error::Chunk(
            "bound external storage requires its configured backend",
        )),
    }
}

/// Called before any serving starts. The writer retains local exclusive ownership,
/// but no job is retained while marker I/O runs.
pub(crate) fn initialize_online(
    writer: &WriterHandle,
    store: Option<&dyn ChunkStore>,
    source: &crate::storage_lease::StorageLease,
) -> Result<()> {
    source.require_writer(writer)?;
    let Some(store) = store else {
        return writer.call(|db| {
            if db.chunk_storage_binding()?.is_some() || db.has_external_chunks()? {
                return Err(Error::Chunk(
                    "bound external storage requires its configured backend",
                ));
            }
            Ok(())
        });
    };
    let locator = store.locator()?;
    let mut namespace = [0; 16];
    crate::OsEntropy.fill(&mut namespace)?;
    let authority = source.authority_identity()?;
    let fresh_locator = locator.clone();
    let fresh = writer.call(move |db| {
        Ok(starts_fresh(
            db.chunk_storage_binding()?.as_ref(),
            &fresh_locator,
        ))
    })?;
    if fresh {
        refuse_marked_namespace(store)?;
    }
    let empty = !fresh || store.namespace_is_empty()?;
    let binding = writer.call(move |db| {
        let requested = if starts_fresh(db.chunk_storage_binding()?.as_ref(), &locator) && !empty {
            None
        } else {
            authority
        };
        Ok(db
            .prepare_chunk_storage_binding_with_authority(&namespace, &locator, false, requested)?)
    })?;
    source.verify_authority(binding.authority_identity)?;
    if binding.state == State::Active {
        verify(
            &binding,
            store,
            &binding
                .host_id
                .ok_or(Error::Chunk("missing chunk binding host"))?,
            source,
        )?;
    } else {
        publish_marker(store, &binding)?;
        writer.call(move |db| {
            db.activate_chunk_storage_binding(&binding)?;
            Ok(())
        })?;
    }
    crate::chunk_ledger::initialize_online(writer, store)
}

pub(crate) fn initialize_offline(
    database: &mut Database,
    store: &dyn ChunkStore,
    source: &crate::storage_lease::StorageLease,
) -> Result<()> {
    let mut namespace = [0; 16];
    crate::OsEntropy.fill(&mut namespace)?;
    let locator = store.locator()?;
    let fresh = starts_fresh(database.chunk_storage_binding()?.as_ref(), &locator);
    if fresh {
        refuse_marked_namespace(store)?;
    }
    let authority = if fresh && !store.namespace_is_empty()? {
        None
    } else {
        source.authority_identity()?
    };
    let binding = database
        .prepare_chunk_storage_binding_with_authority(&namespace, &locator, false, authority)?;
    source.verify_authority(binding.authority_identity)?;
    if binding.state == State::Active {
        verify(
            &binding,
            store,
            &binding
                .host_id
                .ok_or(Error::Chunk("missing chunk binding host"))?,
            source,
        )?;
    } else {
        publish_marker(store, &binding)?;
        database.activate_chunk_storage_binding(&binding)?;
    }
    crate::chunk_ledger::initialize_offline(database, store)
}

/// Explicit offline adoption of legacy external references. Each verified row is
/// checkpointed. A changed reference revision resets the next run to a full scan.
/// The store must remain immutable; this command does not prove exclusive remote
/// ownership or repair missing data. Progress callbacks receive no secret bytes.
pub fn adopt_chunk_store(
    path: impl AsRef<Path>,
    config: foks_server_db::Config,
    store: &dyn ChunkStore,
    mut progress: impl FnMut(ChunkAdoptionProgress),
) -> Result<ChunkAdoptionProgress> {
    if !path.as_ref().is_file() {
        return Err(Error::Chunk("adoption requires an existing database"));
    }
    let guard = crate::DatabaseWriterGuard::acquire(path.as_ref())?;
    let source = crate::storage_lease::StorageLease::with_store(path.as_ref(), Some(store), false)?;
    let mut database = guard.open_database(config)?;
    let mut namespace = [0; 16];
    crate::OsEntropy.fill(&mut namespace)?;
    let authority = if database.chunk_storage_binding()?.is_some() {
        source.authority_identity()?
    } else {
        None
    };
    let mut binding = database.prepare_chunk_storage_binding_with_authority(
        &namespace,
        &store.locator()?,
        true,
        authority,
    )?;
    source.verify_authority(binding.authority_identity)?;
    if binding.state == State::Active {
        verify(
            &binding,
            store,
            &binding
                .host_id
                .ok_or(Error::Chunk("missing chunk binding host"))?,
            &source,
        )?;
    } else {
        let expected_marker = marker(&binding)?;
        if store
            .read_namespace_marker()?
            .is_some_and(|bytes| bytes != expected_marker)
        {
            return Err(Error::Chunk(
                "storage namespace marker belongs to another binding",
            ));
        }
        if binding.state == State::PendingAdoption {
            while let Some(stored) = database.next_kv_chunk(binding.after.as_ref(), true)? {
                let KvChunkPayload::External(reference) = &stored.chunk.payload else {
                    return Err(Error::Chunk("adoption returned inline data"));
                };
                let bytes = store.get_object(&reference.object)?;
                crate::chunk_storage::decode_external(
                    reference,
                    stored.key.offset,
                    stored.chunk.final_chunk,
                    &bytes,
                )?;
                binding = database.checkpoint_chunk_adoption(
                    &binding,
                    &stored.key,
                    reference.object.encoded_bytes,
                )?;
                progress(ChunkAdoptionProgress {
                    verified_objects: binding.verified_objects,
                    verified_bytes: binding.verified_bytes,
                });
            }
        }
        store.create_namespace_marker_if_absent(&expected_marker)?;
        database.activate_chunk_storage_binding(&binding)?;
    }
    Ok(ChunkAdoptionProgress {
        verified_objects: binding.verified_objects,
        verified_bytes: binding.verified_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    struct LostMarkerReply {
        inner: Arc<dyn ChunkStore>,
        lose: AtomicBool,
    }
    impl ChunkStore for LostMarkerReply {
        fn remove_stale_temporaries(&self) -> Result<u64> {
            self.inner.remove_stale_temporaries()
        }
        fn namespace_lease(
            &self,
            exclusive: bool,
        ) -> crate::Result<Option<crate::chunks::ChunkNamespaceLease>> {
            self.inner.namespace_lease(exclusive)
        }
        fn namespace_is_empty(&self) -> Result<bool> {
            self.inner.namespace_is_empty()
        }
        fn inventory_page(
            &self,
            cursor: Option<&str>,
            maximum: usize,
        ) -> Result<crate::chunks::ChunkInventoryPage> {
            self.inner.inventory_page(cursor, maximum)
        }
        fn locator(&self) -> Result<String> {
            self.inner.locator()
        }
        fn read_namespace_marker(&self) -> Result<Option<Vec<u8>>> {
            self.inner.read_namespace_marker()
        }
        fn create_namespace_marker_if_absent(&self, marker: &[u8]) -> Result<()> {
            self.inner.create_namespace_marker_if_absent(marker)?;
            if self.lose.swap(false, Ordering::SeqCst) {
                Err(Error::Chunk("injected lost marker reply"))
            } else {
                Ok(())
            }
        }
        fn put_object(&self, object: &foks_server_db::ChunkObject, bytes: &[u8]) -> Result<()> {
            self.inner.put_object(object, bytes)
        }
        fn get_object(&self, object: &foks_server_db::ChunkObject) -> Result<Vec<u8>> {
            self.inner.get_object(object)
        }
    }

    #[test]
    fn pending_binding_survives_lost_marker_reply_and_writer_restart() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("db");
        drop(Database::open(&path, foks_server_db::Config::default()).unwrap());
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO host_metadata VALUES(1,?1,'test',X'',X'')",
                [&[1_u8; 33][..]],
            )
            .unwrap();
        drop(connection);
        let store = LostMarkerReply {
            inner: crate::chunks::ChunkStoreConfig::Filesystem {
                directory: temporary.path().join("objects"),
            }
            .build()
            .unwrap(),
            lose: AtomicBool::new(true),
        };
        let writer =
            crate::Writer::start(path.clone(), foks_server_db::Config::default(), 4).unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(&store), false).unwrap();
        assert!(initialize_online(&writer.handle(), Some(&store), &source).is_err());
        let pending = writer
            .call(|db| Ok(db.chunk_storage_binding()?.unwrap()))
            .unwrap();
        assert_eq!(pending.state, State::PendingNew);
        let original_marker = store.read_namespace_marker().unwrap().unwrap();
        writer.shutdown().unwrap();
        let writer =
            crate::Writer::start(path.clone(), foks_server_db::Config::default(), 4).unwrap();
        initialize_online(&writer.handle(), Some(&store), &source).unwrap();
        let active = writer
            .call(|db| Ok(db.chunk_storage_binding()?.unwrap()))
            .unwrap();
        assert_eq!(active.state, State::Active);
        assert_eq!(active.namespace_id, pending.namespace_id);
        assert_eq!(
            store.read_namespace_marker().unwrap().unwrap(),
            original_marker
        );
        assert!(initialize_online(&writer.handle(), None, &source).is_err());
        writer.shutdown().unwrap();
    }

    fn authority_fixture() -> (tempfile::TempDir, std::path::PathBuf, Arc<dyn ChunkStore>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("database");
        drop(Database::open(&path, foks_server_db::Config::default()).unwrap());
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute(
            "INSERT INTO host_metadata VALUES(1,?1,'test',X'',X'')",
            [[3u8; 33].as_slice()],
        )
        .unwrap();
        drop(c);
        let store = crate::chunks::ChunkStoreConfig::Filesystem {
            directory: dir.path().join("objects"),
        }
        .build()
        .unwrap();
        (dir, path, store)
    }

    #[test]
    fn fresh_authority_rejects_stopped_clones_and_inode_replacement() {
        let (dir, path, store) = authority_fixture();
        let writer =
            crate::Writer::start(path.clone(), foks_server_db::Config::default(), 4).unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(store.as_ref()), false)
                .unwrap();
        initialize_online(&writer.handle(), Some(store.as_ref()), &source).unwrap();
        let binding = writer
            .call(|db| Ok(db.chunk_storage_binding()?.unwrap()))
            .unwrap();
        assert!(binding.authority_identity.is_some());
        let new_marker = store.read_namespace_marker().unwrap().unwrap();
        let mut legacy = binding.clone();
        legacy.authority_identity = None;
        assert_ne!(
            new_marker,
            marker(&legacy).unwrap(),
            "old binaries must reject exact marker mismatch"
        );
        writer.shutdown().unwrap();
        drop(source);
        let clone = dir.path().join("clone");
        std::fs::copy(&path, &clone).unwrap();
        let cloned_source =
            crate::storage_lease::StorageLease::with_store(&clone, Some(store.as_ref()), false)
                .unwrap();
        let reader = ReadDatabase::open(&clone, foks_server_db::Config::default()).unwrap();
        assert!(verify_reader(&reader, Some(store.as_ref()), &cloned_source).is_err());
        drop(reader);
        drop(cloned_source);
        assert!(crate::chunk_collection::collect_chunk_store(
            &clone,
            foks_server_db::Config::default(),
            store.as_ref(),
            1,
            1024,
            false
        )
        .is_err());
        let clone_writer =
            crate::Writer::start(clone.clone(), foks_server_db::Config::default(), 4).unwrap();
        let cloned_source =
            crate::storage_lease::StorageLease::with_store(&clone, Some(store.as_ref()), false)
                .unwrap();
        assert!(
            initialize_online(&clone_writer.handle(), Some(store.as_ref()), &cloned_source)
                .is_err()
        );
        clone_writer.shutdown().unwrap();
        drop(cloned_source);
        std::fs::rename(&path, dir.path().join("old-inode")).unwrap();
        std::fs::copy(&clone, &path).unwrap();
        assert!(crate::chunk_collection::collect_chunk_store(
            &path,
            foks_server_db::Config::default(),
            store.as_ref(),
            1,
            1024,
            false
        )
        .is_err());
    }

    #[test]
    fn unbound_database_refuses_a_marked_namespace_without_pinning_it() {
        let (dir, path, old_store) = authority_fixture();
        old_store
            .create_namespace_marker_if_absent(b"marker of the installation that was backed up")
            .unwrap();
        let writer =
            crate::Writer::start(path.clone(), foks_server_db::Config::default(), 4).unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(old_store.as_ref()), false)
                .unwrap();
        assert!(matches!(
            initialize_online(&writer.handle(), Some(old_store.as_ref()), &source),
            Err(Error::Chunk(FOREIGN_NAMESPACE))
        ));
        assert!(writer
            .call(|db| Ok(db.chunk_storage_binding()?))
            .unwrap()
            .is_none());
        drop(source);
        let new_store = crate::chunks::ChunkStoreConfig::Filesystem {
            directory: dir.path().join("new-objects"),
        }
        .build()
        .unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(new_store.as_ref()), false)
                .unwrap();
        initialize_online(&writer.handle(), Some(new_store.as_ref()), &source).unwrap();
        let binding = writer
            .call(|db| Ok(db.chunk_storage_binding()?.unwrap()))
            .unwrap();
        assert_eq!(binding.state, State::Active);
        assert_eq!(binding.locator, Some(new_store.locator().unwrap()));
        assert!(binding.authority_identity.is_some());
        writer.shutdown().unwrap();
        drop(source);
        // The offline path applies the same refusal.
        let unbound = dir.path().join("unbound");
        drop(Database::open(&unbound, foks_server_db::Config::default()).unwrap());
        let c = rusqlite::Connection::open(&unbound).unwrap();
        c.execute(
            "INSERT INTO host_metadata VALUES(1,?1,'test',X'',X'')",
            [[3u8; 33].as_slice()],
        )
        .unwrap();
        drop(c);
        let source = crate::storage_lease::StorageLease::with_store(
            &unbound,
            Some(old_store.as_ref()),
            false,
        )
        .unwrap();
        let mut db = Database::open(&unbound, foks_server_db::Config::default()).unwrap();
        assert!(initialize_offline(&mut db, old_store.as_ref(), &source).is_err());
        assert!(db.chunk_storage_binding().unwrap().is_none());
    }

    #[test]
    fn never_activated_binding_to_a_marked_store_moves_to_a_new_store() {
        let (dir, path, old_store) = authority_fixture();
        old_store
            .create_namespace_marker_if_absent(b"marker of the installation that was backed up")
            .unwrap();
        // A database pinned by an earlier release: PendingNew, no authority,
        // locator of a store whose marker belongs to another binding.
        let mut db = Database::open(&path, foks_server_db::Config::default()).unwrap();
        db.prepare_chunk_storage_binding_with_authority(
            &[7; 16],
            &old_store.locator().unwrap(),
            false,
            None,
        )
        .unwrap();
        drop(db);
        let writer =
            crate::Writer::start(path.clone(), foks_server_db::Config::default(), 4).unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(old_store.as_ref()), false)
                .unwrap();
        // Restarted on the same old store, the pinned database gets the remedy,
        // not the bare marker conflict.
        assert!(matches!(
            initialize_online(&writer.handle(), Some(old_store.as_ref()), &source),
            Err(Error::Chunk(FOREIGN_NAMESPACE))
        ));
        drop(source);
        let new_store = crate::chunks::ChunkStoreConfig::Filesystem {
            directory: dir.path().join("new-objects"),
        }
        .build()
        .unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(new_store.as_ref()), false)
                .unwrap();
        initialize_online(&writer.handle(), Some(new_store.as_ref()), &source).unwrap();
        let binding = writer
            .call(|db| Ok(db.chunk_storage_binding()?.unwrap()))
            .unwrap();
        assert_eq!(binding.state, State::Active);
        assert_eq!(binding.locator, Some(new_store.locator().unwrap()));
        assert_ne!(binding.namespace_id, Some([7; 16]));
        assert!(binding.authority_identity.is_some());
        writer.shutdown().unwrap();
    }

    #[test]
    fn pending_authority_rejects_clone_before_marker_creation() {
        let (dir, path, store) = authority_fixture();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(store.as_ref()), false)
                .unwrap();
        let mut db = Database::open(&path, foks_server_db::Config::default()).unwrap();
        db.prepare_chunk_storage_binding_with_authority(
            &[7; 16],
            &store.locator().unwrap(),
            false,
            source.authority_identity().unwrap(),
        )
        .unwrap();
        drop(db);
        drop(source);
        assert!(store.read_namespace_marker().unwrap().is_none());
        let clone = dir.path().join("clone");
        std::fs::copy(&path, &clone).unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&clone, Some(store.as_ref()), false)
                .unwrap();
        let mut db = Database::open(&clone, foks_server_db::Config::default()).unwrap();
        assert!(initialize_offline(&mut db, store.as_ref(), &source).is_err());
        assert!(store.read_namespace_marker().unwrap().is_none());
        drop(db);
        drop(source);
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(store.as_ref()), false)
                .unwrap();
        let mut db = Database::open(&path, foks_server_db::Config::default()).unwrap();
        initialize_offline(&mut db, store.as_ref(), &source).unwrap();
        assert!(db
            .chunk_storage_binding()
            .unwrap()
            .unwrap()
            .authority_identity
            .is_some());
    }

    #[test]
    fn legacy_namespace_never_autoqualifies_or_deletes_even_with_pending_intent() {
        let (_dir, path, store) = authority_fixture();
        let mut db = Database::open(&path, foks_server_db::Config::default()).unwrap();
        let binding = db
            .prepare_chunk_storage_binding(&[7; 16], &store.locator().unwrap(), false)
            .unwrap();
        store
            .create_namespace_marker_if_absent(&marker(&binding).unwrap())
            .unwrap();
        db.activate_chunk_storage_binding(&binding).unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(store.as_ref()), false)
                .unwrap();
        initialize_offline(&mut db, store.as_ref(), &source).unwrap();
        assert_eq!(
            db.chunk_storage_binding()
                .unwrap()
                .unwrap()
                .authority_identity,
            None
        );
        let bytes = b"legacy orphan still owned by a possible clone";
        let token = db
            .reserve_chunk_object(
                &crate::chunks::chunk_digest(bytes),
                bytes.len() as u64,
                &[9; 16],
                1,
            )
            .unwrap();
        store.put_object(&token.object(), bytes).unwrap();
        db.settle_chunk_publication(&token, true).unwrap();
        db.begin_chunk_collection(&[2; 16], 1, 1024).unwrap();
        let before = db.chunk_physical_usage().unwrap();
        drop(db);
        drop(source);
        let report = crate::chunk_collection::collect_chunk_store(
            &path,
            foks_server_db::Config::default(),
            store.as_ref(),
            1,
            1024,
            false,
        )
        .unwrap();
        assert!(report.report_only);
        assert!(!report.backend_qualified);
        assert_eq!(report.deleted_objects, 0);
        assert_eq!(report.usage, before);
        assert_eq!(store.get_object(&token.object()).unwrap(), bytes);
    }

    #[test]
    fn physical_namespace_lock_excludes_different_database_paths_and_store_instances() {
        let (dir, path, store) = authority_fixture();
        let clone = dir.path().join("clone");
        std::fs::copy(&path, &clone).unwrap();
        let second = crate::chunks::ChunkStoreConfig::Filesystem {
            directory: dir.path().join("objects"),
        }
        .build()
        .unwrap();
        let shared = Arc::new(
            crate::storage_lease::StorageLease::with_store(&path, Some(store.as_ref()), false)
                .unwrap(),
        );
        let detached = shared.clone();
        drop(shared);
        assert!(crate::storage_lease::StorageLease::with_store(
            &clone,
            Some(second.as_ref()),
            true
        )
        .is_err());
        drop(detached);
        let exclusive =
            crate::storage_lease::StorageLease::with_store(&clone, Some(second.as_ref()), true)
                .unwrap();
        assert!(
            crate::storage_lease::StorageLease::with_store(&path, Some(store.as_ref()), false)
                .is_err()
        );
        drop(exclusive);
    }
    #[test]
    fn mismatched_writer_source_is_rejected_before_namespace_binding() {
        let (dir, path, store) = authority_fixture();
        let other = dir.path().join("other");
        std::fs::copy(&path, &other).unwrap();
        let writer = crate::Writer::start(other, foks_server_db::Config::default(), 4).unwrap();
        let source =
            crate::storage_lease::StorageLease::with_store(&path, Some(store.as_ref()), false)
                .unwrap();
        assert!(initialize_online(&writer.handle(), Some(store.as_ref()), &source).is_err());
        assert!(store.read_namespace_marker().unwrap().is_none());
        assert!(writer
            .call(|db| Ok(db.chunk_storage_binding()?))
            .unwrap()
            .is_none());
        writer.shutdown().unwrap();
    }
}
