//! Local, conservative object capacity. Inventory never deletes objects and does
//! not establish that an old remote publisher has stopped.
use crate::chunks::ChunkStore;
use crate::{Result, WriterHandle};
use std::path::Path;

pub(crate) fn initialize_online(writer: &WriterHandle, store: &dyn ChunkStore) -> Result<()> {
    let empty_candidate = writer.call(|db| {
        db.recover_chunk_publications()?;
        Ok(db.chunk_inventory_empty_candidate()?)
    })?;
    if empty_candidate && store.namespace_is_empty()? {
        writer.call(|db| {
            db.confirm_empty_chunk_inventory()?;
            Ok(())
        })?;
    }
    Ok(())
}
pub(crate) fn initialize_offline(
    db: &mut foks_server_db::Database,
    store: &dyn ChunkStore,
) -> Result<()> {
    db.recover_chunk_publications()?;
    if db.chunk_inventory_empty_candidate()? && store.namespace_is_empty()? {
        db.confirm_empty_chunk_inventory()?;
    }
    Ok(())
}
/// Requires exclusive local ownership and externally drained legacy publishers.
/// Existing tracked ambiguous publishers stay charged even if listing misses them.
/// This command only records/report inventory; it never issues DELETE.
pub fn inventory_chunk_store(
    path: impl AsRef<Path>,
    config: foks_server_db::Config,
    store: &dyn ChunkStore,
    restart: bool,
    mut progress: impl FnMut(&foks_server_db::ChunkInventoryCheckpoint),
) -> Result<foks_server_db::ChunkPhysicalUsage> {
    if !path.as_ref().is_file() {
        return Err(crate::Error::Chunk("inventory requires existing database"));
    }
    let guard = crate::DatabaseWriterGuard::acquire(path.as_ref())?;
    let mut db = guard.open_database(config)?;
    crate::chunk_binding::initialize_offline(&mut db, store)?;
    let mut checkpoint = db.begin_chunk_inventory(restart)?;
    loop {
        let page = store.inventory_page(checkpoint.cursor.as_deref(), 256)?;
        if page
            .next
            .as_ref()
            .is_some_and(|next| Some(next) == checkpoint.cursor.as_ref())
        {
            return Err(crate::Error::Chunk("inventory cursor did not advance"));
        }
        checkpoint =
            db.record_chunk_inventory_page(&checkpoint, &page.entries, page.next.as_deref())?;
        progress(&checkpoint);
        if page.next.is_none() {
            break;
        }
    }
    Ok(db.chunk_physical_usage()?)
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChunkCapacityConfig {
    pub maximum_bytes: u64,
    pub maximum_objects: u64,
    pub maximum_unresolved_attempts: u64,
    pub maximum_uncertain_bytes: u64,
}
impl Default for ChunkCapacityConfig {
    fn default() -> Self {
        let c = foks_server_db::Config::default();
        Self {
            maximum_bytes: c.maximum_chunk_physical_bytes,
            maximum_objects: c.maximum_chunk_objects,
            maximum_unresolved_attempts: c.maximum_chunk_unresolved_attempts,
            maximum_uncertain_bytes: c.maximum_chunk_uncertain_bytes,
        }
    }
}
impl ChunkCapacityConfig {
    pub fn database_config(&self) -> Result<foks_server_db::Config> {
        if [
            self.maximum_bytes,
            self.maximum_objects,
            self.maximum_unresolved_attempts,
            self.maximum_uncertain_bytes,
        ]
        .into_iter()
        .any(|v| v == 0 || v > i64::MAX as u64)
        {
            return Err(crate::Error::Config("invalid physical chunk capacity"));
        }
        Ok(foks_server_db::Config {
            maximum_chunk_physical_bytes: self.maximum_bytes,
            maximum_chunk_objects: self.maximum_objects,
            maximum_chunk_unresolved_attempts: self.maximum_unresolved_attempts,
            maximum_chunk_uncertain_bytes: self.maximum_uncertain_bytes,
            ..Default::default()
        })
    }
}
