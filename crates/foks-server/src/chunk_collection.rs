//! Offline, bounded physical collection. Remote backends remain report-only.
use crate::{chunks::ChunkStore, Error, Result};
use std::path::Path;
#[derive(Clone, Debug)]
pub struct ChunkCollectionReport {
    /// Ledger candidates only; remote providers are not qualified for deletion.
    pub eligible_objects: u64,
    pub report_only: bool,
    pub deleted_objects: u64,
    pub deleted_bytes: u64,
    pub backend_qualified: bool,
    pub usage: foks_server_db::ChunkPhysicalUsage,
}
/// Reclaims at most one bounded batch under exclusive local ownership. Repeat
/// to drain additional batches. No remote provider has been qualified for DELETE.
pub fn collect_chunk_store(
    path: impl AsRef<Path>,
    config: foks_server_db::Config,
    store: &dyn ChunkStore,
    maximum_objects: usize,
    maximum_bytes: u64,
    dry_run: bool,
) -> Result<ChunkCollectionReport> {
    if !(1..=128).contains(&maximum_objects) || maximum_bytes == 0 {
        return Err(Error::Config("invalid collection batch bounds"));
    }
    if !path.as_ref().is_file() {
        return Err(Error::Chunk("collection requires an existing database"));
    }
    let writer = crate::DatabaseWriterGuard::acquire(path.as_ref())?;
    let _storage =
        crate::storage_lease::StorageLease::with_store(path.as_ref(), Some(store), true)?;
    store.remove_stale_temporaries()?;
    let mut db = writer.open_database(config)?;
    crate::chunk_binding::initialize_offline(&mut db, store, &_storage)?;
    let qualified = store.supports_collection()
        && _storage.verify_authority(
            db.chunk_storage_binding()?
                .and_then(|b| b.authority_identity),
        )?;
    let eligible_objects = db.chunk_collection_candidates()?;
    let mut report = ChunkCollectionReport {
        eligible_objects,
        report_only: dry_run || !qualified,
        deleted_objects: 0,
        deleted_bytes: 0,
        backend_qualified: qualified,
        usage: db.chunk_physical_usage()?,
    };
    if dry_run || !report.backend_qualified {
        return Ok(report);
    }
    let mut token = [0; 16];
    crate::Entropy::fill(&crate::OsEntropy, &mut token)?;
    for deletion in db.begin_chunk_collection(&token, maximum_objects, maximum_bytes)? {
        store.delete_object(&deletion.object)?;
        db.finish_chunk_collection(&deletion)?;
        report.deleted_objects += 1;
        report.deleted_bytes += deletion.object.encoded_bytes;
    }
    report.usage = db.chunk_physical_usage()?;
    Ok(report)
}
