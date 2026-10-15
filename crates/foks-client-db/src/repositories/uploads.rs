//! Durable immutable upload preparation and namespace-publication ownership.
use crate::*;

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE kv_upload_limits (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 max_reserved_bytes INTEGER NOT NULL CHECK(max_reserved_bytes BETWEEN 16777216 AND 1099511627776),
 max_pending INTEGER NOT NULL CHECK(max_pending BETWEEN 1 AND 128)
) STRICT;
INSERT INTO kv_upload_limits VALUES(1,268435456,16);
CREATE TABLE kv_uploads (
 operation_id BLOB PRIMARY KEY CHECK(length(operation_id)=16),
 host_id BLOB NOT NULL CHECK(length(host_id)=33),
 uid BLOB NOT NULL CHECK(length(uid)=33),
 party_id BLOB NOT NULL CHECK(length(party_id)=33),
 file_id BLOB NOT NULL CHECK(length(file_id)=17),
 manifest_hash BLOB NOT NULL CHECK(length(manifest_hash)=32),
 state INTEGER NOT NULL DEFAULT 0 CHECK(state BETWEEN 0 AND 11),
 chunk_count INTEGER NOT NULL DEFAULT 0 CHECK(chunk_count BETWEEN 0 AND 256),
 acknowledged_chunks INTEGER NOT NULL DEFAULT 0 CHECK(acknowledged_chunks BETWEEN 0 AND 256),
 submitting_sequence INTEGER CHECK(submitting_sequence BETWEEN 0 AND 255),
 encoded_bytes INTEGER NOT NULL DEFAULT 0 CHECK(encoded_bytes>=0),
 reserved_bytes INTEGER NOT NULL CHECK(reserved_bytes>=0),
 plaintext_bytes INTEGER CHECK(plaintext_bytes BETWEEN 0 AND 1073741824),
 namespace_child BLOB UNIQUE REFERENCES mutation_operations(operation_id) ON DELETE SET NULL,
 adapter_parent BLOB UNIQUE REFERENCES mutation_operations(operation_id) ON DELETE SET NULL,
 receipt_version INTEGER CHECK(receipt_version>0),
 receipt_directories INTEGER CHECK(receipt_directories>=0),
 receipt_entries INTEGER CHECK(receipt_entries>=0),
 created_at INTEGER NOT NULL CHECK(created_at>=0),
 CHECK(acknowledged_chunks<=chunk_count),
 CHECK(state NOT IN (1,2,3,4,5,6) OR (chunk_count>0 AND plaintext_bytes IS NOT NULL)),
 CHECK(state NOT IN (4,5) OR namespace_child IS NOT NULL),
 CHECK(state NOT IN (5,6) OR (receipt_version IS NOT NULL AND receipt_directories IS NOT NULL AND receipt_entries IS NOT NULL))
) STRICT, WITHOUT ROWID;
CREATE TRIGGER kv_upload_parent_live BEFORE DELETE ON mutation_operations
WHEN EXISTS(SELECT 1 FROM kv_uploads WHERE adapter_parent=OLD.operation_id AND state NOT IN (6,9,11))
BEGIN SELECT RAISE(ABORT,'live upload owns adapter parent'); END;
CREATE INDEX kv_uploads_by_owner ON kv_uploads(host_id,uid,party_id,operation_id);
CREATE TABLE kv_upload_chunks (
 operation_id BLOB NOT NULL REFERENCES kv_uploads(operation_id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL CHECK(sequence BETWEEN 0 AND 255),
 chunk_offset INTEGER NOT NULL CHECK(chunk_offset>=0),
 final_chunk INTEGER NOT NULL CHECK(final_chunk IN (0,1)),
 encoded_bytes INTEGER NOT NULL CHECK(encoded_bytes BETWEEN 1 AND 8388736),
 payload_hash BLOB NOT NULL CHECK(length(payload_hash)=32),
 PRIMARY KEY(operation_id,sequence),
 UNIQUE(operation_id,chunk_offset)
) STRICT, WITHOUT ROWID;
"#;

#[cfg(test)]
pub(crate) const REMOVE_SCHEMA:&str="DROP TRIGGER kv_upload_parent_live; DROP TABLE kv_upload_chunks; DROP TABLE kv_uploads; DROP TABLE kv_upload_limits;";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UploadState {
    AwaitingSource = 10,
    InlineUntracked = 11,
    Preparing = 0,
    Ready = 1,
    Uploading = 2,
    ContentComplete = 3,
    Publishing = 4,
    RemoteVerified = 5,
    Finalized = 6,
    NeedsSource = 7,
    Blocked = 8,
    Cancelled = 9,
}
impl UploadState {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Finalized | Self::Cancelled | Self::InlineUntracked
        )
    }
    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            10 => Self::AwaitingSource,
            11 => Self::InlineUntracked,
            0 => Self::Preparing,
            1 => Self::Ready,
            2 => Self::Uploading,
            3 => Self::ContentComplete,
            4 => Self::Publishing,
            5 => Self::RemoteVerified,
            6 => Self::Finalized,
            7 => Self::NeedsSource,
            8 => Self::Blocked,
            9 => Self::Cancelled,
            _ => return Err(invalid("unknown upload state")),
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadChunkRecord {
    pub sequence: u32,
    pub offset: u64,
    pub final_chunk: bool,
    pub encoded_bytes: u64,
    pub payload_hash: [u8; 32],
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadOperation {
    pub id: [u8; 16],
    pub host_id: Vec<u8>,
    pub uid: Vec<u8>,
    pub party_id: Vec<u8>,
    pub file_id: [u8; 17],
    pub manifest_hash: [u8; 32],
    pub state: UploadState,
    pub chunk_count: u32,
    pub acknowledged_chunks: u32,
    pub encoded_bytes: u64,
    pub reserved_bytes: u64,
    pub plaintext_bytes: Option<u64>,
    pub namespace_child: Option<[u8; 16]>,
    pub adapter_parent: Option<[u8; 16]>,
    pub receipt_version: Option<u64>,
    pub receipt_directories: Option<u64>,
    pub receipt_entries: Option<u64>,
    pub created_at: u64,
    pub chunks: Vec<UploadChunkRecord>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadLimits {
    pub max_reserved_bytes: u64,
    pub max_pending: u32,
}
fn invalid(s: &'static str) -> Error {
    Error::InvalidMutationOperation(s)
}
const SELECT: &str = "SELECT operation_id,host_id,uid,party_id,file_id,manifest_hash,state,chunk_count,encoded_bytes,reserved_bytes,plaintext_bytes,namespace_child,adapter_parent,receipt_version,receipt_directories,receipt_entries,created_at FROM kv_uploads WHERE operation_id=?1";
fn u64_column(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<u64> {
    let v: i64 = r.get(i)?;
    u64::try_from(v).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(i, v))
}
fn optional_u64_column(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Option<u64>> {
    let v: Option<i64> = r.get(i)?;
    v.map(|v| u64::try_from(v).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(i, v)))
        .transpose()
}
fn read_on(c: &Connection, id: &[u8; 16]) -> Result<Option<UploadOperation>> {
    let row = c
        .query_row(SELECT, [id.as_slice()], |r| {
            Ok((
                r.get::<_, [u8; 16]>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, Vec<u8>>(3)?,
                r.get::<_, [u8; 17]>(4)?,
                r.get::<_, [u8; 32]>(5)?,
                r.get::<_, u8>(6)?,
                r.get::<_, u32>(7)?,
                u64_column(r, 8)?,
                u64_column(r, 9)?,
                optional_u64_column(r, 10)?,
                r.get::<_, Option<[u8; 16]>>(11)?,
                r.get::<_, Option<[u8; 16]>>(12)?,
                optional_u64_column(r, 13)?,
                optional_u64_column(r, 14)?,
                optional_u64_column(r, 15)?,
                u64_column(r, 16)?,
            ))
        })
        .optional()?;
    let Some((
        id,
        host_id,
        uid,
        party_id,
        file_id,
        manifest_hash,
        state,
        chunk_count,
        encoded_bytes,
        reserved_bytes,
        plaintext_bytes,
        namespace_child,
        adapter_parent,
        receipt_version,
        receipt_directories,
        receipt_entries,
        created_at,
    )) = row
    else {
        return Ok(None);
    };
    let mut q=c.prepare("SELECT sequence,chunk_offset,final_chunk,encoded_bytes,payload_hash FROM kv_upload_chunks WHERE operation_id=?1 ORDER BY sequence")?;
    let chunks = q
        .query_map([id.as_slice()], |r| {
            Ok(UploadChunkRecord {
                sequence: r.get(0)?,
                offset: u64_column(r, 1)?,
                final_chunk: r.get(2)?,
                encoded_bytes: u64_column(r, 3)?,
                payload_hash: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let acknowledged_chunks: u32 = c.query_row(
        "SELECT acknowledged_chunks FROM kv_uploads WHERE operation_id=?1",
        [id.as_slice()],
        |r| r.get(0),
    )?;
    Ok(Some(UploadOperation {
        id,
        host_id,
        uid,
        party_id,
        file_id,
        manifest_hash,
        state: UploadState::from_u8(state)?,
        chunk_count,
        acknowledged_chunks,
        encoded_bytes,
        reserved_bytes,
        plaintext_bytes,
        namespace_child,
        adapter_parent,
        receipt_version,
        receipt_directories,
        receipt_entries,
        created_at,
        chunks,
    }))
}
fn limits_on(c: &Connection) -> Result<UploadLimits> {
    Ok(c.query_row(
        "SELECT max_reserved_bytes,max_pending FROM kv_upload_limits WHERE singleton=1",
        [],
        |r| {
            Ok(UploadLimits {
                max_reserved_bytes: u64_column(r, 0)?,
                max_pending: r.get(1)?,
            })
        },
    )?)
}
fn admit(c: &Connection, bytes: u64, new: bool) -> Result<()> {
    let limits = limits_on(c)?;
    let (reserved,pending):(u64,u32)=c.query_row("SELECT coalesce(sum(reserved_bytes),0),coalesce(sum(CASE WHEN state NOT IN (6,9,11) THEN 1 ELSE 0 END),0) FROM kv_uploads",[],|r|Ok((u64_column(r,0)?,r.get(1)?)))?;
    if reserved
        .checked_add(bytes)
        .is_none_or(|v| v > limits.max_reserved_bytes)
        || (new && pending >= limits.max_pending)
    {
        return Err(invalid("local upload spool capacity exhausted"));
    }
    Ok(())
}
impl HardStateStore {
    /// Reserve the identity and pending slot before the source or IPC stream is read.
    pub fn record_upload_intent(
        &mut self,
        id: [u8; 16],
        host: &[u8],
        uid: &[u8],
        party: &[u8],
        created_at: u64,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        admit(&tx, 0, true)?;
        let count: u32 = tx.query_row("SELECT count(*) FROM kv_uploads", [], |r| r.get(0))?;
        if count >= 1024 {
            return Err(invalid("upload receipt capacity exhausted"));
        }
        tx.execute("INSERT INTO kv_uploads(operation_id,host_id,uid,party_id,file_id,manifest_hash,state,reserved_bytes,created_at) VALUES(?1,?2,?3,?4,zeroblob(17),zeroblob(32),10,0,?5)",params![id.as_slice(),host,uid,party,sqlite_integer("upload creation",created_at)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish_inline_upload_identity(&mut self, id: &[u8; 16]) -> Result<()> {
        if self.connection.execute("UPDATE kv_uploads SET state=11 WHERE operation_id=?1 AND state=0 AND file_id=zeroblob(17) AND reserved_bytes=0",[id.as_slice()])?!=1{return Err(invalid("inline upload identity has content"))}
        Ok(())
    }
    pub fn begin_upload_source(&mut self, id: &[u8; 16]) -> Result<()> {
        if self.connection.execute("UPDATE kv_uploads SET state=0 WHERE operation_id=?1 AND state=10 AND file_id=zeroblob(17)",[id.as_slice()])?!=1{return Err(invalid("upload source already consumed; resume or cancel the original upload"))}
        Ok(())
    }
    pub fn upload(&self, id: &[u8; 16]) -> Result<Option<UploadOperation>> {
        read_on(&self.connection, id)
    }
    pub fn upload_for_namespace(&self, child: &[u8; 16]) -> Result<Option<UploadOperation>> {
        let id: Option<[u8; 16]> = self
            .connection
            .query_row(
                "SELECT operation_id FROM kv_uploads WHERE namespace_child=?1",
                [child.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| self.upload(&id))
            .transpose()
            .map(Option::flatten)
    }
    pub fn upload_for_adapter(&self, parent: &[u8; 16]) -> Result<Option<UploadOperation>> {
        let id: Option<[u8; 16]> = self
            .connection
            .query_row(
                "SELECT operation_id FROM kv_uploads WHERE adapter_parent=?1",
                [parent.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| self.upload(&id))
            .transpose()
            .map(Option::flatten)
    }
    pub fn begin_upload_pass(&mut self, id: &[u8; 16]) -> Result<()> {
        if self.connection.execute("UPDATE kv_uploads SET state=2,acknowledged_chunks=0,submitting_sequence=NULL WHERE operation_id=?1 AND state IN (1,2,3) AND namespace_child IS NULL",[id.as_slice()])?!=1{return Err(invalid("upload cannot start another content pass"))}
        Ok(())
    }
    pub fn begin_upload_chunk(&mut self, id: &[u8; 16], sequence: u32) -> Result<()> {
        if self.connection.execute("UPDATE kv_uploads SET submitting_sequence=?2 WHERE operation_id=?1 AND state=2 AND acknowledged_chunks=?2 AND submitting_sequence IS NULL AND ?2<chunk_count",params![id.as_slice(),sequence])?!=1{return Err(invalid("upload chunk attempt is out of order"))}
        Ok(())
    }
    pub fn acknowledge_upload_chunk(&mut self, id: &[u8; 16], sequence: u32) -> Result<()> {
        if self.connection.execute("UPDATE kv_uploads SET acknowledged_chunks=?2+1,submitting_sequence=NULL WHERE operation_id=?1 AND state=2 AND submitting_sequence=?2 AND acknowledged_chunks=?2",params![id.as_slice(),sequence])?!=1{return Err(invalid("upload chunk acknowledgment is out of order"))}
        Ok(())
    }
    pub fn upload_limits(&self) -> Result<UploadLimits> {
        limits_on(&self.connection)
    }
    pub fn set_upload_limits(&mut self, limits: UploadLimits) -> Result<()> {
        let tx = self.write_transaction()?;
        tx.execute(
            "UPDATE kv_upload_limits SET max_reserved_bytes=?1,max_pending=?2 WHERE singleton=1",
            params![
                sqlite_integer("upload capacity", limits.max_reserved_bytes)?,
                limits.max_pending
            ],
        )?;
        admit(&tx, 0, false)?;
        tx.commit()?;
        Ok(())
    }
    pub fn uploads(&self, host: &[u8], uid: &[u8]) -> Result<Vec<UploadOperation>> {
        let mut q=self.connection.prepare("SELECT operation_id FROM kv_uploads WHERE host_id=?1 AND uid=?2 ORDER BY operation_id LIMIT 1025")?;
        let ids = q
            .query_map(params![host, uid], |r| r.get::<_, [u8; 16]>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if ids.len() > 1024 {
            return Err(invalid("upload receipt inventory needs cleanup"));
        }
        ids.iter()
            .map(|id| self.upload(id)?.ok_or(invalid("upload disappeared")))
            .collect()
    }
    pub fn preflight_upload(&self, bytes: u64) -> Result<()> {
        admit(&self.connection, bytes, true)?;
        let total: u32 = self
            .connection
            .query_row("SELECT count(*) FROM kv_uploads", [], |r| r.get(0))?;
        if total >= 1024 {
            return Err(invalid("upload receipt capacity exhausted"));
        }
        Ok(())
    }
    /// Charge and bind the manifest before filesystem installation. No remote content exists yet.
    pub fn record_upload(&mut self, op: &UploadOperation) -> Result<()> {
        if op.state != UploadState::Preparing || op.chunk_count != 0 || op.namespace_child.is_some()
        {
            return Err(invalid("invalid upload preparation"));
        }
        let tx = self.write_transaction()?;
        if let Some(existing) = read_on(&tx, &op.id)? {
            if existing.state != UploadState::Preparing
                || existing.file_id != [0; 17]
                || existing.manifest_hash != [0; 32]
                || existing.host_id != op.host_id
                || existing.uid != op.uid
                || existing.party_id != op.party_id
                || existing.reserved_bytes != 0
            {
                return Err(invalid("upload identity already has prepared content"));
            }
            admit(&tx, op.reserved_bytes, false)?;
            tx.execute("UPDATE kv_uploads SET file_id=?2,manifest_hash=?3,reserved_bytes=?4,adapter_parent=?5 WHERE operation_id=?1",params![op.id.as_slice(),op.file_id.as_slice(),op.manifest_hash.as_slice(),sqlite_integer("upload reservation",op.reserved_bytes)?,op.adapter_parent.as_ref().map(|v|v.as_slice())])?;
        } else {
            admit(&tx, op.reserved_bytes, true)?;
            let total: u32 = tx.query_row("SELECT count(*) FROM kv_uploads", [], |r| r.get(0))?;
            if total >= 1024 {
                return Err(invalid("upload receipt capacity exhausted"));
            }
            tx.execute("INSERT INTO kv_uploads(operation_id,host_id,uid,party_id,file_id,manifest_hash,reserved_bytes,adapter_parent,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![op.id.as_slice(),op.host_id,op.uid,op.party_id,op.file_id.as_slice(),op.manifest_hash.as_slice(),sqlite_integer("upload reservation",op.reserved_bytes)?,op.adapter_parent.as_ref().map(|v|v.as_slice()),sqlite_integer("upload creation",op.created_at)?])?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Charge before allocating/installing another protected record. Charges survive failed writes.
    pub fn reserve_upload_bytes(&mut self, id: &[u8; 16], bytes: u64) -> Result<()> {
        let tx = self.write_transaction()?;
        admit(&tx, bytes, false)?;
        if tx.execute("UPDATE kv_uploads SET reserved_bytes=reserved_bytes+?2 WHERE operation_id=?1 AND state=0",params![id.as_slice(),sqlite_integer("upload reservation",bytes)?])?!=1{return Err(invalid("upload is not preparing"))}
        tx.commit()?;
        Ok(())
    }
    pub fn record_upload_chunk(&mut self, id: &[u8; 16], chunk: &UploadChunkRecord) -> Result<()> {
        let tx = self.write_transaction()?;
        let op = read_on(&tx, id)?.ok_or(invalid("upload missing"))?;
        if op.state != UploadState::Preparing
            || op.file_id == [0; 17]
            || op.manifest_hash == [0; 32]
            || chunk.sequence != op.chunk_count
            || op
                .chunks
                .last()
                .is_some_and(|v| v.final_chunk || v.offset >= chunk.offset)
            || (chunk.sequence == 0 && chunk.offset != 0)
            || op
                .encoded_bytes
                .checked_add(chunk.encoded_bytes)
                .is_none_or(|v| v > op.reserved_bytes)
        {
            return Err(invalid("invalid upload chunk sequence or reservation"));
        }
        tx.execute(
            "INSERT INTO kv_upload_chunks VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                id.as_slice(),
                chunk.sequence,
                sqlite_integer("upload offset", chunk.offset)?,
                chunk.final_chunk,
                sqlite_integer("upload encoded bytes", chunk.encoded_bytes)?,
                chunk.payload_hash.as_slice()
            ],
        )?;
        tx.execute("UPDATE kv_uploads SET chunk_count=chunk_count+1,encoded_bytes=encoded_bytes+?2 WHERE operation_id=?1",params![id.as_slice(),sqlite_integer("upload encoded bytes",chunk.encoded_bytes)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn ready_upload(&mut self, id: &[u8; 16], plaintext_bytes: u64) -> Result<()> {
        let tx = self.write_transaction()?;
        let op = read_on(&tx, id)?.ok_or(invalid("upload missing"))?;
        if op.state != UploadState::Preparing || !op.chunks.last().is_some_and(|c| c.final_chunk) {
            return Err(invalid("upload is incomplete"));
        }
        tx.execute(
            "UPDATE kv_uploads SET state=1,plaintext_bytes=?2 WHERE operation_id=?1",
            params![
                id.as_slice(),
                sqlite_integer("upload plaintext bytes", plaintext_bytes)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn advance_upload(&mut self, id: &[u8; 16], state: UploadState) -> Result<()> {
        let tx = self.write_transaction()?;
        let op = read_on(&tx, id)?.ok_or(invalid("upload missing"))?;
        let legal = op.state == state
            || matches!(
                (op.state, state),
                (UploadState::Preparing, UploadState::NeedsSource)
                    | (
                        UploadState::Ready | UploadState::Uploading | UploadState::ContentComplete,
                        UploadState::Uploading
                    )
                    | (UploadState::Uploading, UploadState::ContentComplete)
                    | (UploadState::RemoteVerified, UploadState::Finalized)
            );
        if !legal
            || (state == UploadState::ContentComplete && op.acknowledged_chunks != op.chunk_count)
        {
            return Err(invalid("invalid upload transition"));
        }
        tx.execute(
            "UPDATE kv_uploads SET state=?2 WHERE operation_id=?1",
            params![id.as_slice(), state as u8],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn cancel_upload(&mut self, id: &[u8; 16]) -> Result<()> {
        let tx = self.write_transaction()?;
        let op = read_on(&tx, id)?.ok_or(invalid("upload missing"))?;
        let rejected_child = op
            .namespace_child
            .map(|child| {
                tx.query_row(
                    "SELECT state=5 FROM mutation_operations WHERE operation_id=?1",
                    [child.as_slice()],
                    |r| r.get::<_, bool>(0),
                )
            })
            .transpose()?
            .unwrap_or(false);
        if (op.namespace_child.is_some() && !rejected_child)
            || matches!(
                op.state,
                UploadState::RemoteVerified | UploadState::Finalized
            )
        {
            return Err(invalid(
                "upload publication must be reconciled before cancellation",
            ));
        }
        tx.execute(
            "UPDATE kv_uploads SET state=9,namespace_child=NULL WHERE operation_id=?1",
            [id.as_slice()],
        )?;
        if let Some(parent) = op.adapter_parent {
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM kv_adapter_submissions WHERE internal_id=?1 AND state IN (0,2))",[parent.as_slice()],|r|r.get(0))?;
            if !valid {
                return Err(invalid("upload adapter cannot be cancelled"));
            }
            if tx.execute("UPDATE mutation_operations SET state=5 WHERE operation_id=?1 AND operation_kind=8 AND state IN (1,2,3,5)",[parent.as_slice()])? != 1 {
                return Err(invalid("upload adapter parent cannot be cancelled"));
            }
            tx.execute("UPDATE kv_adapter_submissions SET state=2,ancillary_committed=max(ancillary_committed,EXISTS(SELECT 1 FROM mutation_children c JOIN mutation_operations m ON m.operation_id=c.child_id WHERE c.parent_id=?1 AND c.completion=0 AND m.state IN (4,6))) WHERE internal_id=?1",[parent.as_slice()])?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Call only after protected material and temporary files have been durably erased.
    /// Retain identity and receipt, removing the no-longer-needed per-chunk bindings.
    pub fn release_upload_spool(&mut self, id: &[u8; 16], manifest_bytes: u64) -> Result<()> {
        let tx = self.write_transaction()?;
        if tx.execute(
            "UPDATE kv_uploads SET reserved_bytes=?2 WHERE operation_id=?1 AND state IN (6,9,11)",
            params![
                id.as_slice(),
                sqlite_integer("upload manifest bytes", manifest_bytes)?
            ],
        )? != 1
        {
            return Err(invalid("upload cleanup is not terminal"));
        }
        tx.execute(
            "DELETE FROM kv_upload_chunks WHERE operation_id=?1",
            [id.as_slice()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn verify_upload(
        &mut self,
        id: &[u8; 16],
        version: u64,
        directories: u64,
        entries: u64,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        if tx.execute("UPDATE kv_uploads SET state=5,receipt_version=?2,receipt_directories=?3,receipt_entries=?4 WHERE operation_id=?1 AND state=4 AND EXISTS(SELECT 1 FROM mutation_operations m WHERE m.operation_id=kv_uploads.namespace_child AND m.state IN (4,6))",params![id.as_slice(),sqlite_integer("upload version",version)?,sqlite_integer("upload directories",directories)?,sqlite_integer("upload entries",entries)?])?!=1{return Err(invalid("upload namespace is not verified"))}
        tx.commit()?;
        Ok(())
    }
    pub fn record_upload_namespace(
        &mut self,
        op: &MutationOperation,
        upload: &[u8; 16],
        adapter: Option<([u8; 16], bool)>,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        let parent = read_on(&tx, upload)?.ok_or(invalid("upload missing"))?;
        if parent.state != UploadState::ContentComplete
            || parent.namespace_child.is_some()
            || op.kind != MutationKind::KvNamespace
            || op.state != MutationState::Prepared
            || op.host_id != parent.host_id
            || op.scope_id != parent.party_id
            || adapter.map(|v| v.0) != parent.adapter_parent
        {
            return Err(invalid("namespace does not match prepared upload"));
        }
        super::journals::record_mutation_on(&tx, op)?;
        if let Some((id, completion)) = adapter {
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM mutation_operations WHERE operation_id=?1 AND operation_kind=?2 AND state IN (2,3) AND host_id=?3 AND scope_id=?4 AND CASE WHEN length(subject_id)=0 THEN scope_id ELSE subject_id END=?5)",params![id.as_slice(),MutationKind::KvAdapter as u8,parent.host_id,parent.uid,parent.party_id],|r|r.get(0))?;
            if !valid {
                return Err(invalid("upload adapter binding changed"));
            }
            let unresolved: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM mutation_children c JOIN mutation_operations m ON m.operation_id=c.child_id WHERE c.parent_id=?1 AND m.state NOT IN (4,5,6))",
                [id.as_slice()], |row| row.get(0),
            )?;
            if unresolved {
                return Err(invalid("upload adapter has an unresolved prior child"));
            }
            // This parent is an aggregate receipt, not a replayable remote request.
            // The immutable upload completed its content pass and has no publication
            // child; every earlier child has a definite outcome. Reenter submission
            // only to attach this new namespace child, atomically with its ownership.
            tx.execute(
                "UPDATE mutation_operations SET state=2 WHERE operation_id=?1 AND state=3",
                [id.as_slice()],
            )?;
            let children: u32 = tx.query_row(
                "SELECT count(*) FROM mutation_children WHERE parent_id=?1",
                [id.as_slice()],
                |r| r.get(0),
            )?;
            if children >= 256 {
                return Err(invalid("adapter child budget exhausted"));
            }
            tx.execute(
                "INSERT INTO mutation_children(parent_id,child_id,completion) VALUES(?1,?2,?3)",
                params![id.as_slice(), op.operation_id.as_slice(), completion],
            )?;
        }
        tx.execute(
            "UPDATE kv_uploads SET state=4,namespace_child=?2 WHERE operation_id=?1",
            params![upload.as_slice(), op.operation_id.as_slice()],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// A definitely rejected namespace child permits a fresh precondition, never an ambiguous one.
    pub fn retry_upload_namespace(&mut self, id: &[u8; 16]) -> Result<()> {
        if self.connection.execute("UPDATE kv_uploads SET state=3,namespace_child=NULL WHERE operation_id=?1 AND state=4 AND EXISTS(SELECT 1 FROM mutation_operations m WHERE m.operation_id=kv_uploads.namespace_child AND m.state=5)",[id.as_slice()])?!=1{return Err(invalid("upload namespace is not definitely rejected"))}
        Ok(())
    }
}
