//! Internal chunk placement. These types never appear in protocol messages.

use crate::allocation::RowAllocationExt as _;
use rusqlite::{params, Connection, Row};
use sha2::{Digest as _, Sha256};

use crate::{error::sql_integer, Database, Error, ReadDatabase, Result, StoredKvFileChunk};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalKvChunk {
    pub digest: [u8; 32],
    pub ciphertext_bytes: u64,
    pub encoded_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KvChunkPayload {
    Inline(Vec<u8>),
    External(ExternalKvChunk),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvChunkKey {
    pub uid: [u8; 33],
    pub file_id: [u8; 16],
    pub offset: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredKvChunkObject {
    pub key: KvChunkKey,
    pub chunk: StoredKvFileChunk,
    /// Present only for inline rows; the object store contains these exact bytes.
    pub exact_chunk: Option<Vec<u8>>,
}

pub(crate) fn chunk_digest(exact: &[u8]) -> [u8; 32] {
    Sha256::digest(exact).into()
}

pub(crate) fn validate_encoded_chunk(
    exact: &[u8],
    ciphertext: &[u8],
    offset: u64,
    final_chunk: bool,
    digest: Option<[u8; 32]>,
) -> Result<()> {
    if exact.is_empty()
        || exact.len() > 9 * 1024 * 1024 + 1024
        || ciphertext.is_empty()
        || ciphertext.len() > 9 * 1024 * 1024
        || digest.is_some_and(|digest| digest != chunk_digest(exact))
    {
        return Err(Error::Invalid("external KV chunk digest or size"));
    }
    let decoded = foks_proto::KvUploadChunk::decode(exact)
        .map_err(|_| Error::Invalid("external KV chunk encoding"))?;
    if decoded.offset != offset
        || decoded.ciphertext != ciphertext
        || decoded.final_upload.is_some() != final_chunk
    {
        return Err(Error::Invalid("external KV chunk fields"));
    }
    Ok(())
}

pub(crate) fn payload_from_row(row: &Row<'_>, start: usize) -> Result<KvChunkPayload> {
    let ciphertext: Vec<u8> = row.budgeted_get(start)?;
    let digest: Option<Vec<u8>> = row.budgeted_get(start + 1)?;
    let bytes: Option<i64> = row.budgeted_get(start + 2)?;
    let encoded: Option<i64> = row.budgeted_get(start + 3)?;
    match (digest, bytes, encoded) {
        (None, None, None) => Ok(KvChunkPayload::Inline(ciphertext)),
        (Some(digest), Some(bytes), Some(encoded)) if ciphertext.is_empty() => {
            let bytes = crate::error::unsigned(bytes)?;
            let encoded = crate::error::unsigned(encoded)?;
            if bytes == 0
                || bytes > 9 * 1024 * 1024
                || encoded == 0
                || encoded > 9 * 1024 * 1024 + 1024
            {
                return Err(Error::Invalid("stored external KV chunk size"));
            }
            Ok(KvChunkPayload::External(ExternalKvChunk {
                digest: digest
                    .try_into()
                    .map_err(|_| Error::Invalid("stored external KV chunk digest"))?,
                ciphertext_bytes: bytes,
                encoded_bytes: encoded,
            }))
        }
        _ => Err(Error::Invalid("stored KV chunk representation")),
    }
}

fn has_external_chunks(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM kv_file_chunks WHERE external_digest IS NOT NULL)",
        [],
        |row| row.budgeted_get(0),
    )?)
}

impl Database {
    /// Includes incomplete and reclaiming uploads: they may still need the store
    /// for backup or an exact retry even if downloads cannot currently see them.
    pub fn has_external_chunks(&self) -> Result<bool> {
        has_external_chunks(&self.connection)
    }

    /// Returns one bounded row, including incomplete uploads. A private backup
    /// or an exclusively owned offline database is required for a stable walk.
    /// Callers must release the result's database borrow before object-store I/O.
    pub fn next_kv_chunk(
        &self,
        after: Option<&KvChunkKey>,
        external: bool,
    ) -> Result<Option<StoredKvChunkObject>> {
        let (index, predicate) = if external {
            ("kv_chunks_external", "external_digest IS NOT NULL")
        } else {
            ("kv_chunks_inline", "external_digest IS NULL")
        };
        let sql = format!(
            "SELECT uid,file_id,clear_offset,ciphertext,external_digest,
            external_ciphertext_bytes,external_encoded_bytes,final_chunk,exact_upload_chunk
            FROM kv_file_chunks INDEXED BY {index}
            WHERE {predicate} AND (uid,file_id,clear_offset) > (?1,?2,?3)
            ORDER BY uid,file_id,clear_offset LIMIT 1"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let uid = after.map_or(&[][..], |key| key.uid.as_slice());
        let file_id = after.map_or(&[][..], |key| key.file_id.as_slice());
        let offset = after.map_or(Ok(-1), |key| sql_integer(key.offset))?;
        let mut rows = statement.query(params![uid, file_id, offset])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let uid: Vec<u8> = row.budgeted_get(0)?;
        let file_id: Vec<u8> = row.budgeted_get(1)?;
        let offset = crate::error::unsigned(row.budgeted_get(2)?)?;
        Ok(Some(StoredKvChunkObject {
            key: KvChunkKey {
                uid: uid
                    .try_into()
                    .map_err(|_| Error::Invalid("stored KV chunk namespace"))?,
                file_id: file_id
                    .try_into()
                    .map_err(|_| Error::Invalid("stored KV chunk file ID"))?,
                offset,
            },
            chunk: StoredKvFileChunk {
                payload: payload_from_row(row, 3)?,
                offset,
                final_chunk: row.budgeted_get(7)?,
            },
            exact_chunk: if external {
                None
            } else {
                Some(row.budgeted_get(8)?)
            },
        }))
    }

    /// Publish the object first. CAS makes a concurrent deletion or already
    /// converted row a harmless false result. Charged lengths do not change.
    pub fn externalize_kv_chunk(
        &mut self,
        key: &KvChunkKey,
        expected_exact: &[u8],
        digest: &[u8; 32],
    ) -> Result<bool> {
        if expected_exact.is_empty()
            || expected_exact.len() > 9 * 1024 * 1024 + 1024
            || chunk_digest(expected_exact) != *digest
        {
            return Err(Error::Invalid("external KV chunk digest or size"));
        }
        let decoded = foks_proto::KvUploadChunk::decode(expected_exact)
            .map_err(|_| Error::Invalid("external KV chunk encoding"))?;
        validate_encoded_chunk(
            expected_exact,
            &decoded.ciphertext,
            key.offset,
            decoded.final_upload.is_some(),
            Some(*digest),
        )?;
        Ok(self.connection.execute(
            "UPDATE kv_file_chunks SET external_digest=?4,
                external_ciphertext_bytes=length(ciphertext), external_encoded_bytes=length(exact_upload_chunk),
                ciphertext=X'',exact_upload_chunk=X''
             WHERE uid=?1 AND file_id=?2 AND clear_offset=?3 AND external_digest IS NULL
                AND exact_upload_chunk=?5 AND ciphertext=?6 AND final_chunk=?7",
            params![key.uid,key.file_id,sql_integer(key.offset)?,digest.as_slice(),expected_exact,
                decoded.ciphertext,decoded.final_upload.is_some()],
        )? == 1)
    }

    /// Hydrates a private backup after verifying its immutable object. A changed
    /// digest or representation fails the CAS; invalid object bytes fail closed.
    pub fn hydrate_kv_chunk(
        &mut self,
        key: &KvChunkKey,
        expected: &ExternalKvChunk,
        exact: &[u8],
        ciphertext: &[u8],
    ) -> Result<bool> {
        if exact.is_empty()
            || exact.len() > 9 * 1024 * 1024 + 1024
            || ciphertext.is_empty()
            || ciphertext.len() > 9 * 1024 * 1024
            || exact.len() as u64 != expected.encoded_bytes
            || ciphertext.len() as u64 != expected.ciphertext_bytes
        {
            return Err(Error::Invalid("external KV chunk length"));
        }
        // The exact digest binds the final-size metadata too. The row's final
        // flag is checked by the CAS below rather than accepted from the caller.
        let decoded = foks_proto::KvUploadChunk::decode(exact)
            .map_err(|_| Error::Invalid("external KV chunk encoding"))?;
        validate_encoded_chunk(
            exact,
            ciphertext,
            key.offset,
            decoded.final_upload.is_some(),
            Some(expected.digest),
        )?;
        Ok(self.connection.execute(
            "UPDATE kv_file_chunks SET ciphertext=?4,exact_upload_chunk=?5,
                external_encoded_bytes=NULL,external_ciphertext_bytes=NULL,external_digest=NULL
             WHERE uid=?1 AND file_id=?2 AND clear_offset=?3 AND external_digest=?6
                AND external_ciphertext_bytes=?7 AND external_encoded_bytes=?8 AND final_chunk=?9",
            params![
                key.uid,
                key.file_id,
                sql_integer(key.offset)?,
                ciphertext,
                exact,
                expected.digest.as_slice(),
                sql_integer(expected.ciphertext_bytes)?,
                sql_integer(expected.encoded_bytes)?,
                decoded.final_upload.is_some()
            ],
        )? == 1)
    }
}

impl ReadDatabase {
    pub fn has_external_chunks(&self) -> Result<bool> {
        has_external_chunks(&self.connection)
    }
}
