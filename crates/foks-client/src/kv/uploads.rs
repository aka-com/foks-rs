//! Immutable protected upload spool. No plaintext is written to disk.
use super::rpc::KvRequest;
use super::support::{random_kv_node_id, read_kv_upload_chunk_with_carry};
use super::{KvWriteOptions, KvWriteResult, KvWriteSession};
use crate::{
    Error, MutationCoordinator, ProtectedMutationStore, ProtectedRecordFamily, ProtectedRecordKey,
    ProtectedStoreError, Result,
};
use foks_client_db::{
    HardStateStore, KvDirectoryProjection, MutationState, UploadChunkRecord, UploadOperation,
    UploadState,
};
use foks_crypto::{derive_kv_keys, prefixed_hash, seal_kv_chunk};
use foks_proto::{KvLargeFileMetadata, KvNodeId, KvNodeType, KvUploadChunk, Role, SecretSeed};
use foks_snowpack::{decode, encode, Value};
use std::io::Read;
use std::path::Path;
use zeroize::{Zeroize, Zeroizing};
const MANIFEST_HASH: u64 = 0x72f2_8f6b_34e1_a931;
const CHUNK_HASH: u64 = 0xe8c3_61f4_75b8_91a2;
// 4 MiB plaintext plus Snowpack binary prefix pads to 8 MiB; secretbox adds16.
// The canonical chunk tuple (including u64 offsets/final totals) fits the128-byte margin.
const MAX_CHUNK_RECORD: u64 = 8 * 1024 * 1024 + 128;
const PROTECTED_OVERHEAD: u64 = 48;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvUploadReceipt {
    pub operation_id: [u8; 16],
    pub node_id: KvNodeId,
    pub version: u64,
    pub plaintext_bytes: u64,
}
#[derive(Debug)]
pub struct KvUploadResult {
    pub receipt: KvUploadReceipt,
    pub path: Vec<KvDirectoryProjection>,
}
struct Manifest {
    id: [u8; 16],
    host: Vec<u8>,
    uid: Vec<u8>,
    party: Vec<u8>,
    file: KvNodeId,
    parent: [u8; 16],
    name: String,
    metadata: KvLargeFileMetadata,
    write_role: Role,
    overwrite: bool,
    expected_version: Option<u64>,
    selected: Option<[u8; 16]>,
    expected_size: Option<u64>,
    seed: SecretSeed,
}
fn scrub_value(value: &mut Value) {
    match value {
        Value::Binary(bytes) => bytes.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(scrub_value),
        _ => (),
    }
}
fn protected_error(error: ProtectedStoreError) -> Error {
    Error::ProtectedStore(error.to_string())
}
fn invalid(s: &'static str) -> Error {
    Error::OperationBinding(s)
}
fn binary(v: &Value) -> Result<&[u8]> {
    if let Value::Binary(b) = v {
        Ok(b)
    } else {
        Err(invalid("invalid upload material binary"))
    }
}
fn unsigned(v: &Value) -> Result<u64> {
    if let Value::Unsigned(b) = v {
        Ok(*b)
    } else {
        Err(invalid("invalid upload material number"))
    }
}
fn boolean(v: &Value) -> Result<bool> {
    if let Value::Bool(b) = v {
        Ok(*b)
    } else {
        Err(invalid("invalid upload material boolean"))
    }
}
fn fixed<const N: usize>(v: &Value) -> Result<[u8; N]> {
    binary(v)?
        .try_into()
        .map_err(|_| invalid("invalid upload material length"))
}
fn optional_u64(v: &Value) -> Result<Option<u64>> {
    if matches!(v, Value::Null) {
        Ok(None)
    } else {
        unsigned(v).map(Some)
    }
}
impl Manifest {
    fn encoded(&self) -> Result<Zeroizing<Vec<u8>>> {
        let mut value = Value::Array(vec![
            Value::Unsigned(1),
            Value::Binary(self.id.to_vec()),
            Value::Binary(self.host.clone()),
            Value::Binary(self.uid.clone()),
            Value::Binary(self.party.clone()),
            Value::Binary(self.file.0.to_vec()),
            Value::Binary(self.parent.to_vec()),
            Value::Binary(self.name.as_bytes().to_vec()),
            Value::Binary(self.metadata.encode()?),
            Value::Binary(self.write_role.encoded()?),
            Value::Bool(self.overwrite),
            self.expected_version.map_or(Value::Null, Value::Unsigned),
            self.selected
                .map_or(Value::Null, |v| Value::Binary(v.to_vec())),
            self.expected_size.map_or(Value::Null, Value::Unsigned),
            Value::Binary(self.seed.as_bytes().to_vec()),
        ]);
        let encoded = encode(&value);
        scrub_value(&mut value);
        Ok(Zeroizing::new(encoded?))
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut value = decode(bytes)?;
        let result = (|| {
            let Value::Array(v) = &value else {
                return Err(invalid("invalid upload manifest"));
            };
            if v.len() != 15 || unsigned(&v[0])? != 1 {
                return Err(invalid("unsupported upload manifest"));
            }
            let name = String::from_utf8(binary(&v[7])?.to_vec())
                .map_err(|_| invalid("invalid upload name"))?;
            super::support::validate_kv_component(name.as_bytes())?;
            Ok(Self {
                id: fixed(&v[1])?,
                host: binary(&v[2])?.to_vec(),
                uid: binary(&v[3])?.to_vec(),
                party: binary(&v[4])?.to_vec(),
                file: KvNodeId(fixed(&v[5])?),
                parent: fixed(&v[6])?,
                name,
                metadata: KvLargeFileMetadata::decode(binary(&v[8])?)?,
                write_role: Role::decode(binary(&v[9])?)?,
                overwrite: boolean(&v[10])?,
                expected_version: optional_u64(&v[11])?,
                selected: if matches!(v[12], Value::Null) {
                    None
                } else {
                    Some(fixed(&v[12])?)
                },
                expected_size: optional_u64(&v[13])?,
                seed: SecretSeed::new(fixed(&v[14])?),
            })
        })();
        scrub_value(&mut value);
        result
    }
    fn options(&self) -> KvWriteOptions {
        KvWriteOptions {
            read_role: self.metadata.key.role,
            write_role: self.write_role,
            overwrite: self.overwrite,
            expected_version: self.expected_version,
        }
    }
    fn validate(&self, op: &UploadOperation) -> Result<()> {
        if self.id != op.id
            || self.host != op.host_id
            || self.uid != op.uid
            || self.party != op.party_id
            || self.file.0 != op.file_id
            || self.file.node_type()? != KvNodeType::File
        {
            return Err(invalid("upload manifest scope mismatch"));
        }
        Ok(())
    }
}
pub(crate) fn validate_inventory(
    op: &UploadOperation,
    family: ProtectedRecordFamily,
    bytes: &[u8],
) -> Result<()> {
    match family {
        ProtectedRecordFamily::UploadManifest => {
            if prefixed_hash(MANIFEST_HASH, bytes) != op.manifest_hash {
                return Err(invalid("upload manifest fingerprint changed"));
            }
            Manifest::decode(bytes)?.validate(op)
        }
        ProtectedRecordFamily::UploadChunk(sequence) => {
            let chunk = KvUploadChunk::decode(bytes)?;
            if bytes.len() as u64 > MAX_CHUNK_RECORD {
                return Err(invalid("upload chunk record exceeds limit"));
            }
            if let Some(row) = op.chunks.iter().find(|c| c.sequence == sequence) {
                if prefixed_hash(CHUNK_HASH, bytes) != row.payload_hash
                    || bytes.len() as u64 != row.encoded_bytes
                    || chunk.offset != row.offset
                    || chunk.final_upload.is_some() != row.final_chunk
                {
                    return Err(invalid("upload chunk binding changed"));
                }
            } else if !matches!(op.state, UploadState::Preparing | UploadState::NeedsSource)
                || sequence != op.chunk_count
            {
                return Err(invalid("unowned upload chunk"));
            }
            Ok(())
        }
        _ => Err(invalid("invalid upload record family")),
    }
}
fn load_manifest<S: ProtectedMutationStore + ?Sized>(
    store: &mut S,
    op: &UploadOperation,
) -> Result<Manifest> {
    let bytes = store
        .get(&ProtectedRecordKey::UploadManifest(&op.id).encoded())
        .map_err(protected_error)?;
    validate_inventory(op, ProtectedRecordFamily::UploadManifest, &bytes)?;
    Manifest::decode(&bytes)
}
fn receipt(op: &UploadOperation) -> Result<KvUploadReceipt> {
    Ok(KvUploadReceipt {
        operation_id: op.id,
        node_id: KvNodeId(op.file_id),
        version: op
            .receipt_version
            .ok_or(invalid("upload receipt missing"))?,
        plaintext_bytes: op.plaintext_bytes.ok_or(invalid("upload size missing"))?,
    })
}
fn remove(store: &mut dyn ProtectedMutationStore, key: &[u8]) -> Result<()> {
    match store.remove(key) {
        Ok(()) | Err(ProtectedStoreError::Missing) => Ok(()),
        Err(e) => Err(protected_error(e)),
    }
}

/// Cleanup is local, bounded and idempotent; a failed erasure retains its reservation.
pub fn cleanup_file_upload(
    database: &Path,
    store: &mut dyn ProtectedMutationStore,
    id: [u8; 16],
) -> Result<()> {
    let mut hard = HardStateStore::open(database)?;
    let op = hard.upload(&id)?.ok_or(invalid("upload not recorded"))?;
    if !op.state.terminal() {
        return Err(invalid("upload cleanup is not terminal"));
    }
    // The unjournaled next record can exist after fsync-before-insert, so include it.
    for sequence in 0..=op.chunk_count.min(255) {
        remove(
            store,
            &ProtectedRecordKey::UploadChunk(&id, sequence).encoded(),
        )?;
    }
    remove(store, &ProtectedRecordKey::UploadManifest(&id).encoded())?;
    if store
        .cleanup_upload_temporaries()
        .map_err(protected_error)?
    {
        hard.release_upload_spool(&id, 0)?;
    }
    Ok(())
}
/// Resolve already verified local success before acquiring fresh network credentials.
pub fn verified_file_upload(
    database: &Path,
    store: &mut dyn ProtectedMutationStore,
    id: [u8; 16],
    host: &[u8],
    uid: &[u8],
    party: &[u8],
) -> Result<Option<KvUploadReceipt>> {
    let mut hard = HardStateStore::open(database)?;
    let Some(mut op) = hard.upload(&id)? else {
        return Ok(None);
    };
    if op.host_id != host || op.uid != uid || op.party_id != party {
        return Err(invalid("upload belongs to another account or store"));
    }
    if op.state == UploadState::Publishing {
        if let Some(child) = op
            .namespace_child
            .map(|child| hard.mutation(&child))
            .transpose()?
            .flatten()
        {
            if child.state == MutationState::RemoteVerified {
                let material =
                    MutationCoordinator::new(database, store).load_bound_request(&child)?;
                let (_, dirents) = super::write::decode_namespace_material(&material)?;
                let manifest = load_manifest(store, &op)?;
                let [entry] = dirents.as_slice() else {
                    return Err(invalid("upload publication shape changed"));
                };
                if entry.value != manifest.file || entry.parent != manifest.parent {
                    return Err(invalid("upload publication identity changed"));
                }
                hard.verify_upload(&id, entry.version, 0, 0)?;
                op = hard.upload(&id)?.ok_or(invalid("upload disappeared"))?;
            }
        }
    }
    if !matches!(
        op.state,
        UploadState::RemoteVerified | UploadState::Finalized
    ) {
        return Ok(None);
    }
    let result = receipt(&op)?;
    if let Some(child) = op
        .namespace_child
        .map(|id| hard.mutation(&id))
        .transpose()?
        .flatten()
    {
        if child.state == MutationState::RemoteVerified {
            super::write::finalize_verified_material(database, store, &child)?;
        }
    }
    if let Some(parent) = op.adapter_parent {
        let parent = hard
            .mutation(&parent)?
            .ok_or(invalid("upload adapter parent missing"))?;
        // Preserve content and manifest until the application receipt is durable.
        if parent.state != MutationState::Finalized {
            return Ok(Some(result));
        }
    }
    hard.advance_upload(&id, UploadState::Finalized)?;
    // Successful receipt survives cleanup errors; pending bytes remain charged.
    let _ = cleanup_file_upload(database, store, id);
    Ok(Some(result))
}

impl KvWriteSession<'_> {
    pub fn resume_adapter_file_upload(&mut self) -> Result<Option<KvUploadReceipt>> {
        let Some(parent) = self.adapter_parent else {
            return Ok(None);
        };
        let Some(upload) =
            HardStateStore::open(&self.host.database_path)?.upload_for_adapter(&parent)?
        else {
            return Ok(None);
        };
        self.resume_file_upload(upload.id)
            .map(|result| Some(result.receipt))
    }

    pub fn bind_file_upload(&mut self, id: [u8; 16]) -> Result<()> {
        if self.upload_identity.is_some() {
            return Err(invalid("session already has an upload identity"));
        }
        let mut hard = HardStateStore::open(&self.host.database_path)?;
        if let Some(op) = hard.upload(&id)? {
            if op.host_id != self.host.host_id.as_bytes()
                || op.uid != self.actor_uid
                || op.party_id != self.party.party.as_bytes()
            {
                return Err(invalid("upload belongs to another account or store"));
            }
        } else {
            hard.record_upload_intent(
                id,
                self.host.host_id.as_bytes(),
                &self.actor_uid,
                self.party.party.as_bytes(),
                crate::now_microseconds()?,
            )?;
        }
        hard.begin_upload_source(&id)?;
        self.upload_identity = Some(id);
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_and_submit_large_file<R: Read>(
        &mut self,
        parent: [u8; 16],
        name: &str,
        reader: &mut R,
        expected_size: Option<u64>,
        mut options: KvWriteOptions,
        selected: Option<[u8; 16]>,
        first: Vec<u8>,
        mut carry: Vec<u8>,
        first_final: bool,
        tree: Vec<KvDirectoryProjection>,
    ) -> Result<KvWriteResult> {
        let id = match self.upload_identity.take() {
            Some(id) => id,
            None => crate::random_bytes()?,
        };
        let selected = if options.overwrite {
            let entry = tree
                .iter()
                .find(|d| d.directory_id == parent)
                .and_then(|d| d.entries.iter().find(|e| e.name == name.as_bytes()));
            if let Some(e) = entry {
                options.expected_version = Some(e.version);
                Some(e.dirent_id)
            } else {
                options.expected_version = Some(0);
                selected
            }
        } else {
            selected
        };
        let file = random_kv_node_id(KvNodeType::File)?;
        let seed = SecretSeed::new(crate::random_bytes()?);
        let (content_seed, key) = self.current_content_key(options.read_role)?;
        let keys = derive_kv_keys(content_seed)?;
        let mut metadata = keys.seal_file_seed(file, key, 1, &seed, crate::random_bytes()?)?;
        if let Some(size) = expected_size {
            metadata.custom_metadata = Some(keys.seal_large_file_size(
                file,
                metadata.version,
                size,
                crate::random_bytes()?,
            )?);
        }
        let manifest = Manifest {
            id,
            host: self.host.host_id.as_bytes().to_vec(),
            uid: self.actor_uid.clone(),
            party: self.party.party.as_bytes().to_vec(),
            file,
            parent,
            name: name.to_owned(),
            metadata,
            write_role: options.write_role,
            overwrite: options.overwrite,
            expected_version: options.expected_version,
            selected,
            expected_size,
            seed,
        };
        let bytes = manifest.encoded()?;
        let op = UploadOperation {
            id,
            host_id: manifest.host.clone(),
            uid: manifest.uid.clone(),
            party_id: manifest.party.clone(),
            file_id: file.0,
            manifest_hash: prefixed_hash(MANIFEST_HASH, &bytes),
            state: UploadState::Preparing,
            chunk_count: 0,
            acknowledged_chunks: 0,
            encoded_bytes: 0,
            reserved_bytes: bytes.len() as u64
                + PROTECTED_OVERHEAD
                + MAX_CHUNK_RECORD
                + PROTECTED_OVERHEAD,
            plaintext_bytes: None,
            namespace_child: None,
            adapter_parent: self.adapter_parent,
            receipt_version: None,
            receipt_directories: None,
            receipt_entries: None,
            created_at: crate::now_microseconds()?,
            chunks: Vec::new(),
        };
        let mut hard = HardStateStore::open(&self.host.database_path)?;
        // A durable owner and its charge precede every protected filesystem write.
        hard.record_upload(&op)?;
        if let Err(error) = self
            .protected_store
            .put_if_absent(&ProtectedRecordKey::UploadManifest(&id).encoded(), &bytes)
        {
            let _ = hard.advance_upload(&id, UploadState::NeedsSource);
            return Err(protected_error(error));
        }
        let prepared = (|| -> Result<()> {
            let mut clear = Zeroizing::new(first);
            let mut final_chunk = first_final;
            let mut offset = 0u64;
            let mut encrypted = 0u64;
            let mut sequence = 0u32;
            loop {
                let next = offset
                    .checked_add(clear.len() as u64)
                    .ok_or(invalid("upload length overflow"))?;
                if next > super::MAX_KV_FILE_BYTES
                    || (next == super::MAX_KV_FILE_BYTES && !final_chunk)
                {
                    return Err(invalid("upload exceeds FOKS file size limit"));
                }
                let chunk =
                    seal_kv_chunk(&manifest.seed, file, offset, final_chunk, &clear, encrypted)?;
                encrypted = encrypted
                    .checked_add(chunk.ciphertext.len() as u64)
                    .ok_or(invalid("upload ciphertext length overflow"))?;
                let encoded = Zeroizing::new(chunk.encode()?);
                if encoded.len() as u64 > MAX_CHUNK_RECORD {
                    return Err(invalid("upload canonical chunk exceeds limit"));
                }
                hard.reserve_upload_bytes(&id, encoded.len() as u64 + PROTECTED_OVERHEAD)?;
                self.protected_store
                    .put_if_absent(
                        &ProtectedRecordKey::UploadChunk(&id, sequence).encoded(),
                        &encoded,
                    )
                    .map_err(protected_error)?;
                hard.record_upload_chunk(
                    &id,
                    &UploadChunkRecord {
                        sequence,
                        offset,
                        final_chunk,
                        encoded_bytes: encoded.len() as u64,
                        payload_hash: prefixed_hash(CHUNK_HASH, &encoded),
                    },
                )?;
                offset = next;
                sequence += 1;
                if final_chunk {
                    if !carry.is_empty() || expected_size.is_some_and(|v| v != offset) {
                        return Err(invalid("upload length does not match declared size"));
                    }
                    hard.ready_upload(&id, offset)?;
                    break;
                }
                let (next, next_carry, last) =
                    read_kv_upload_chunk_with_carry(reader, std::mem::take(&mut carry))?;
                clear = Zeroizing::new(next);
                carry = next_carry;
                final_chunk = last;
            }
            Ok(())
        })();
        if let Err(error) = prepared {
            let _ = hard.advance_upload(&id, UploadState::NeedsSource);
            return Err(error);
        }
        let result = self.resume_file_upload(id)?;
        Ok(KvWriteResult {
            node_id: result.receipt.node_id,
            dirent_version: result.receipt.version,
            path: result.path,
        })
    }
    pub fn resume_file_upload(&mut self, id: [u8; 16]) -> Result<KvUploadResult> {
        let database = self.host.database_path.clone();
        if let Some(receipt) = verified_file_upload(
            &database,
            self.protected_store,
            id,
            self.host.host_id.as_bytes(),
            &self.actor_uid,
            self.party.party.as_bytes(),
        )? {
            return Ok(KvUploadResult {
                receipt,
                path: Vec::new(),
            });
        }
        let mut hard = HardStateStore::open(&database)?;
        let op = hard.upload(&id)?.ok_or(invalid("upload not recorded"))?;
        if op.host_id != self.host.host_id.as_bytes()
            || op.uid != self.actor_uid
            || op.party_id != self.party.party.as_bytes()
        {
            return Err(invalid("upload belongs to another account or store"));
        }
        if matches!(
            op.state,
            UploadState::AwaitingSource
                | UploadState::Preparing
                | UploadState::NeedsSource
                | UploadState::Cancelled
                | UploadState::InlineUntracked
        ) {
            return Err(invalid("upload has no resumable prepared content"));
        }
        let manifest = load_manifest(self.protected_store, &op)?;
        if self.adapter_parent.is_some() && self.adapter_parent != op.adapter_parent {
            return Err(invalid("upload adapter binding changed"));
        }
        let previous_adapter = (self.adapter_parent, self.adapter_completion);
        self.adapter_parent = op.adapter_parent;
        self.adapter_completion = op.adapter_parent.is_some();
        self.upload_parent = Some(id);
        let result = (|| -> Result<Vec<KvDirectoryProjection>> {
            if let Some(child) = op.namespace_child {
                // Reconcile existing publication before any content replay or destination refresh.
                let mutation = hard
                    .mutation(&child)?
                    .ok_or(invalid("upload namespace child missing"))?;
                if mutation.state == MutationState::Rejected {
                    return Err(invalid(
                        "upload publication was rejected; original intent retained",
                    ));
                }
                return self.resume_namespace_mutation(child);
            }
            if !matches!(
                op.state,
                UploadState::Ready | UploadState::Uploading | UploadState::ContentComplete
            ) {
                return Err(invalid(
                    "upload needs fresh source preparation or is cancelled",
                ));
            }
            if op.chunk_count as usize != op.chunks.len()
                || !op.chunks.last().is_some_and(|c| c.final_chunk)
            {
                return Err(invalid("upload spool is incomplete"));
            }
            let (_, current) = self.current_content_key(manifest.metadata.key.role)?;
            if current != manifest.metadata.key {
                return Err(invalid(
                    "upload key generation changed; original intent retained",
                ));
            }
            hard.begin_upload_pass(&id)?;
            let mut encrypted = 0u64;
            for row in &op.chunks {
                let bytes = self
                    .protected_store
                    .get(&ProtectedRecordKey::UploadChunk(&id, row.sequence).encoded())
                    .map_err(protected_error)?;
                validate_inventory(
                    &op,
                    ProtectedRecordFamily::UploadChunk(row.sequence),
                    &bytes,
                )?;
                let chunk = KvUploadChunk::decode(&bytes)?;
                encrypted = encrypted
                    .checked_add(chunk.ciphertext.len() as u64)
                    .ok_or(invalid("upload ciphertext length overflow"))?;
                if chunk
                    .final_upload
                    .as_ref()
                    .is_some_and(|v| v.size != encrypted)
                {
                    return Err(invalid("upload final ciphertext total changed"));
                }
                hard.begin_upload_chunk(&id, row.sequence)?;
                if row.sequence == 0 {
                    self.call(KvRequest::UploadInit {
                        file_id: manifest.file.object_id(),
                        metadata: manifest.metadata.clone(),
                        chunk,
                    })?;
                } else {
                    self.call(KvRequest::UploadChunk {
                        file_id: manifest.file.object_id(),
                        chunk,
                    })?;
                }
                hard.acknowledge_upload_chunk(&id, row.sequence)?;
            }
            hard.advance_upload(&id, UploadState::ContentComplete)?;
            let tree = self.sync()?;
            if let Some(selected) = manifest.selected {
                super::write::require_selected_entry(
                    &tree,
                    manifest.parent,
                    &manifest.name,
                    selected,
                    manifest.expected_version,
                )?;
            }
            let result = self.link_uploaded_node_bound(
                tree,
                manifest.parent,
                manifest.name.as_bytes(),
                manifest.file,
                manifest.options(),
                manifest.selected,
            )?;
            Ok(result.path)
        })();
        self.upload_parent = None;
        (self.adapter_parent, self.adapter_completion) = previous_adapter;
        let path = result?;
        let receipt = verified_file_upload(
            &database,
            self.protected_store,
            id,
            self.host.host_id.as_bytes(),
            &self.actor_uid,
            self.party.party.as_bytes(),
        )?
        .ok_or(invalid("upload verification receipt missing"))?;
        foks_client_db::SoftStateStore::open(&self.soft_database_path)?.record_large_file_size(
            self.host.host_id.as_bytes(),
            self.party.party.as_bytes(),
            &manifest.file.0,
            receipt.plaintext_bytes,
        )?;
        Ok(KvUploadResult { receipt, path })
    }
}

pub(crate) fn validate_namespace_upload<S: ProtectedMutationStore + ?Sized>(
    database: &Path,
    store: &mut S,
    id: [u8; 16],
    material: &[u8],
) -> Result<()> {
    let op = HardStateStore::open(database)?
        .upload(&id)?
        .ok_or(invalid("upload missing"))?;
    let manifest = load_manifest(store, &op)?;
    let (_, dirents) = super::write::decode_namespace_material(material)?;
    let [entry] = dirents.as_slice() else {
        return Err(invalid("upload publication requires one entry"));
    };
    if entry.value != manifest.file
        || entry.parent != manifest.parent
        || manifest.selected.is_some_and(|id| entry.id != id)
        || entry.write_role != manifest.write_role
        || manifest
            .expected_version
            .is_some_and(|version| version.checked_add(1) != Some(entry.version))
    {
        return Err(invalid("upload publication differs from prepared identity"));
    }
    Ok(())
}
