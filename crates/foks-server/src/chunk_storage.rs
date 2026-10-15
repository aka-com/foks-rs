//! Representation changes for encrypted chunks; no protocol or key changes.

use std::path::Path;

use foks_server_db::{ExternalKvChunk, KvChunkPayload, StoredKvFileChunk};

use crate::chunks::{chunk_digest, ChunkStore};
use crate::{Error, Result};

/// Convert existing inline chunks while holding the installation's exclusive
/// local writer lock. Stop the server first. Interrupted conversion is resumable:
/// each object is durable before its row changes, and objects are never deleted.
pub fn externalize_chunks(
    database_path: impl AsRef<Path>,
    config: foks_server_db::Config,
    store: &dyn ChunkStore,
) -> Result<u64> {
    if !database_path.as_ref().is_file() {
        return Err(Error::Chunk("conversion requires an existing database"));
    }
    let guard = crate::DatabaseWriterGuard::acquire(database_path.as_ref())?;
    let mut database = guard.open_database(config)?;
    crate::chunk_binding::initialize_offline(&mut database, store)?;
    let mut after = None;
    let mut converted = 0;
    while let Some(stored) = database.next_kv_chunk(after.as_ref(), false)? {
        let exact = stored
            .exact_chunk
            .as_deref()
            .ok_or(Error::Chunk("missing inline encoding"))?;
        let chunk = foks_proto::KvUploadChunk::decode(exact)?;
        let KvChunkPayload::Inline(ciphertext) = &stored.chunk.payload else {
            return Err(Error::Chunk("conversion cursor returned external data"));
        };
        if chunk.ciphertext != *ciphertext
            || chunk.offset != stored.key.offset
            || chunk.final_upload.is_some() != stored.chunk.final_chunk
        {
            return Err(Error::Chunk("inline encoding does not match stored chunk"));
        }
        let digest = chunk_digest(exact);
        let mut attempt = [0; 16];
        crate::Entropy::fill(&crate::OsEntropy, &mut attempt)?;
        let now = foks_server_db::Clock::now_micros(&foks_server_db::SystemClock)?;
        let token = database.reserve_chunk_object(&digest, exact.len() as u64, &attempt, now)?;
        match token.action {
            foks_server_db::ChunkReservationAction::Publish => {
                let published = store.put(&digest, exact);
                database.settle_chunk_publication(&token, published.is_ok())?;
                published?;
            }
            foks_server_db::ChunkReservationAction::Verify => {
                let observed = store.get(&digest, exact.len() as u64)?;
                if observed != exact {
                    return Err(Error::Chunk("conversion retry object mismatch"));
                }
                database.confirm_chunk_presence(&token)?;
            }
            foks_server_db::ChunkReservationAction::Present => {}
        }
        database.validate_chunk_reservation(&token)?;
        if !database.externalize_kv_chunk(&stored.key, exact, &digest)? {
            return Err(Error::Chunk("chunk changed during offline conversion"));
        }
        converted += 1;
        after = Some(stored.key);
    }
    Ok(converted)
}

/// Decode and check a canonical object independently of the store implementation.
pub(crate) fn decode_external(
    reference: &ExternalKvChunk,
    offset: u64,
    final_chunk: bool,
    exact: &[u8],
) -> Result<foks_proto::KvUploadChunk> {
    if exact.len() as u64 != reference.encoded_bytes || chunk_digest(exact) != reference.digest {
        return Err(Error::Chunk("object length or digest mismatch"));
    }
    let decoded = foks_proto::KvUploadChunk::decode(exact)?;
    if decoded.offset != offset
        || decoded.final_upload.is_some() != final_chunk
        || decoded.ciphertext.len() as u64 != reference.ciphertext_bytes
    {
        return Err(Error::Chunk("object metadata mismatch"));
    }
    Ok(decoded)
}

pub(crate) fn ciphertext(
    chunk: StoredKvFileChunk,
    store: Option<&dyn ChunkStore>,
) -> Result<Vec<u8>> {
    match chunk.payload {
        KvChunkPayload::Inline(bytes) => Ok(bytes),
        KvChunkPayload::External(reference) => {
            let store = store.ok_or(Error::Chunk("external reference has no configured store"))?;
            let exact = store.get(&reference.digest, reference.encoded_bytes)?;
            Ok(decode_external(&reference, chunk.offset, chunk.final_chunk, &exact)?.ciphertext)
        }
    }
}

pub(crate) fn hydrate_backup(
    path: &Path,
    config: foks_server_db::Config,
    store: Option<&dyn ChunkStore>,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Result<()> {
    {
        let reader = foks_server_db::ReadDatabase::open(path, config.clone())?;
        crate::chunk_binding::verify_reader(&reader, store)?;
    }
    let mut database = foks_server_db::Database::open(path, config)?;
    let mut after = None;
    while let Some(stored) = database.next_kv_chunk(after.as_ref(), true)? {
        if cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(Error::Chunk("backup cancelled"));
        }
        let store = store.ok_or(Error::Chunk("backup requires the source chunk store"))?;
        let KvChunkPayload::External(reference) = &stored.chunk.payload else {
            return Err(Error::Chunk("hydration cursor returned inline data"));
        };
        let exact = store.get(&reference.digest, reference.encoded_bytes)?;
        let chunk = decode_external(
            reference,
            stored.key.offset,
            stored.chunk.final_chunk,
            &exact,
        )?;
        if !database.hydrate_kv_chunk(&stored.key, reference, &exact, &chunk.ciphertext)? {
            return Err(Error::Chunk(
                "private backup chunk changed during hydration",
            ));
        }
        after = Some(stored.key);
    }
    database.clear_private_chunk_storage_binding()?;
    if database.checkpoint()?.outcome() != foks_server_db::CheckpointOutcome::Complete {
        return Err(Error::Chunk("backup hydration checkpoint incomplete"));
    }
    Ok(())
}
