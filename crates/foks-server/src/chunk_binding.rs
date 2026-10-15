//! Placement identity is not an ownership lease or a distributed fence.
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
    Ok(bytes)
}

fn verify(binding: &ChunkStorageBinding, store: &dyn ChunkStore, host: &[u8]) -> Result<()> {
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

/// Validate a snapshot before it can be served or hydrated. No snapshot is held
/// by this function; metadata is copied before the remote marker read.
pub(crate) fn verify_reader(reader: &ReadDatabase, store: Option<&dyn ChunkStore>) -> Result<()> {
    let binding = reader.chunk_storage_binding()?;
    let external = reader.has_external_chunks()?;
    match (binding, store) {
        (Some(binding), Some(store)) => {
            let host = reader
                .host_bootstrap()?
                .ok_or(Error::Chunk("chunk binding requires host bootstrap"))?;
            verify(&binding, store, &host.host_id)
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
) -> Result<()> {
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
    let binding = writer
        .call(move |db| Ok(db.prepare_chunk_storage_binding(&namespace, &locator, false)?))?;
    if binding.state == State::Active {
        verify(
            &binding,
            store,
            &binding
                .host_id
                .ok_or(Error::Chunk("missing chunk binding host"))?,
        )?;
    } else {
        store.create_namespace_marker_if_absent(&marker(&binding)?)?;
        writer.call(move |db| {
            db.activate_chunk_storage_binding(&binding)?;
            Ok(())
        })?;
    }
    crate::chunk_ledger::initialize_online(writer, store)
}

pub(crate) fn initialize_offline(database: &mut Database, store: &dyn ChunkStore) -> Result<()> {
    let mut namespace = [0; 16];
    crate::OsEntropy.fill(&mut namespace)?;
    let binding = database.prepare_chunk_storage_binding(&namespace, &store.locator()?, false)?;
    if binding.state == State::Active {
        verify(
            &binding,
            store,
            &binding
                .host_id
                .ok_or(Error::Chunk("missing chunk binding host"))?,
        )?;
    } else {
        store.create_namespace_marker_if_absent(&marker(&binding)?)?;
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
    let mut database = guard.open_database(config)?;
    let mut namespace = [0; 16];
    crate::OsEntropy.fill(&mut namespace)?;
    let mut binding =
        database.prepare_chunk_storage_binding(&namespace, &store.locator()?, true)?;
    if binding.state == State::Active {
        verify(
            &binding,
            store,
            &binding
                .host_id
                .ok_or(Error::Chunk("missing chunk binding host"))?,
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
        assert!(initialize_online(&writer.handle(), Some(&store)).is_err());
        let pending = writer
            .call(|db| Ok(db.chunk_storage_binding()?.unwrap()))
            .unwrap();
        assert_eq!(pending.state, State::PendingNew);
        let original_marker = store.read_namespace_marker().unwrap().unwrap();
        writer.shutdown().unwrap();
        let writer =
            crate::Writer::start(path.clone(), foks_server_db::Config::default(), 4).unwrap();
        initialize_online(&writer.handle(), Some(&store)).unwrap();
        let active = writer
            .call(|db| Ok(db.chunk_storage_binding()?.unwrap()))
            .unwrap();
        assert_eq!(active.state, State::Active);
        assert_eq!(active.namespace_id, pending.namespace_id);
        assert_eq!(
            store.read_namespace_marker().unwrap().unwrap(),
            original_marker
        );
        assert!(initialize_online(&writer.handle(), None).is_err());
        writer.shutdown().unwrap();
    }
}
