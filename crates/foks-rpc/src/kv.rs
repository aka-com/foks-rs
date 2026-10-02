//! KV authorization, cursors, and request encoding.

use crate::call::{encode_call, encode_select_vhost};
use crate::generated::{
    KV_CACHE_CHECK_METHOD_POSITION, KV_FILE_UPLOAD_CHUNK_METHOD_POSITION,
    KV_FILE_UPLOAD_INIT_METHOD_POSITION, KV_GET_DIR_METHOD_POSITION,
    KV_GET_ENCRYPTED_CHUNK_METHOD_POSITION, KV_GET_METHOD_POSITION, KV_GET_NODE_METHOD_POSITION,
    KV_GET_ROOT_METHOD_POSITION, KV_LIST_METHOD_POSITION, KV_LOCK_ACQUIRE_METHOD_POSITION,
    KV_LOCK_RELEASE_METHOD_POSITION, KV_MKDIR_METHOD_POSITION, KV_PUT_METHOD_POSITION,
    KV_PUT_ROOT_METHOD_POSITION, KV_PUT_SMALL_FILE_OR_SYMLINK_METHOD_POSITION,
    KV_SELECT_VHOST_METHOD_POSITION, KV_STORE_PROTOCOL_ID, KV_USAGE_METHOD_POSITION,
};
use crate::Result;
use foks_proto::{
    EntityId, KvDirectory, KvDirent, KvLargeFileMetadata, KvNodeId, KvPathVersionVector,
    KvSmallFileBox, KvUploadChunk,
};
use foks_snowpack::{encode, Value};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvAuth<'a> {
    User,
    Team(&'a [u8; 16]),
}

impl KvAuth<'_> {
    fn to_value(self) -> Value {
        match self {
            Self::User => Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
            Self::Team(token) => Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Binary(token.to_vec())),
                ))),
            ]),
        }
    }
}

pub(super) fn kv_request_header(
    auth: KvAuth<'_>,
    precondition: Option<&KvPathVersionVector>,
) -> Value {
    Value::Array(vec![
        auth.to_value(),
        precondition.map_or(Value::Null, KvPathVersionVector::to_value),
    ])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvListCursor {
    None,
    Mac([u8; 32]),
    Time(u64),
}

impl KvListCursor {
    fn to_value(self) -> Value {
        match self {
            Self::None => Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
            Self::Mac(mac) => Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((b"1".to_vec(), Box::new(Value::Binary(mac.to_vec()))))),
            ]),
            Self::Time(time) => Value::Array(vec![
                Value::Unsigned(2),
                Value::Variant(Some((b"2".to_vec(), Box::new(Value::Unsigned(time))))),
            ]),
        }
    }
}

/// Starts a KV connection by selecting the host committed by the pinned host
/// chain. Subsequent calls on the same connection use sequence number 1.
pub fn encode_kv_select_vhost_request(host: &EntityId) -> Result<Vec<u8>> {
    encode_select_vhost(KV_STORE_PROTOCOL_ID, KV_SELECT_VHOST_METHOD_POSITION, host)
}

pub fn encode_kv_get_root_request(auth: KvAuth<'_>) -> Result<Vec<u8>> {
    encode_kv_get_root_request_at(auth, 1)
}

pub fn encode_kv_mkdir_request_at(
    auth: KvAuth<'_>,
    precondition: Option<&KvPathVersionVector>,
    directory: &KvDirectory,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_MKDIR_METHOD_POSITION,
        Value::Array(vec![
            kv_request_header(auth, precondition),
            directory.to_value(),
        ]),
        sequence,
    )
}

pub fn encode_kv_put_request_at(
    auth: KvAuth<'_>,
    precondition: Option<&KvPathVersionVector>,
    dirents: &[KvDirent],
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_PUT_METHOD_POSITION,
        Value::Array(vec![
            kv_request_header(auth, precondition),
            Value::Array(dirents.iter().map(KvDirent::to_value).collect()),
        ]),
        sequence,
    )
}

pub fn encode_kv_put_root_request_at(
    auth: KvAuth<'_>,
    root: &foks_proto::KvRoot,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_PUT_ROOT_METHOD_POSITION,
        Value::Array(vec![auth.to_value(), root.to_value()]),
        sequence,
    )
}

pub fn encode_kv_file_upload_init_request_at(
    auth: KvAuth<'_>,
    file_id: [u8; 16],
    metadata: &KvLargeFileMetadata,
    chunk: &KvUploadChunk,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_FILE_UPLOAD_INIT_METHOD_POSITION,
        Value::Array(vec![
            auth.to_value(),
            Value::Binary(file_id.to_vec()),
            metadata.to_value(),
            chunk.to_value(),
        ]),
        sequence,
    )
}

pub fn encode_kv_file_upload_chunk_request_at(
    auth: KvAuth<'_>,
    file_id: [u8; 16],
    chunk: &KvUploadChunk,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_FILE_UPLOAD_CHUNK_METHOD_POSITION,
        Value::Array(vec![
            auth.to_value(),
            Value::Binary(file_id.to_vec()),
            chunk.to_value(),
        ]),
        sequence,
    )
}

pub fn encode_kv_put_small_file_or_symlink_request_at(
    auth: KvAuth<'_>,
    id: KvNodeId,
    boxed: &KvSmallFileBox,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_PUT_SMALL_FILE_OR_SYMLINK_METHOD_POSITION,
        Value::Array(vec![auth.to_value(), id.to_value(), boxed.to_value()]),
        sequence,
    )
}

pub fn encode_kv_lock_acquire_request_at(
    auth: KvAuth<'_>,
    parent: [u8; 16],
    dirent: [u8; 16],
    lock_id: [u8; 16],
    timeout_millis: u64,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_lock_request_at(
        KV_LOCK_ACQUIRE_METHOD_POSITION,
        auth,
        parent,
        dirent,
        lock_id,
        Some(timeout_millis),
        sequence,
    )
}

pub fn encode_kv_lock_release_request_at(
    auth: KvAuth<'_>,
    parent: [u8; 16],
    dirent: [u8; 16],
    lock_id: [u8; 16],
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_lock_request_at(
        KV_LOCK_RELEASE_METHOD_POSITION,
        auth,
        parent,
        dirent,
        lock_id,
        None,
        sequence,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_kv_lock_request_at(
    method: u64,
    auth: KvAuth<'_>,
    parent: [u8; 16],
    dirent: [u8; 16],
    lock_id: [u8; 16],
    timeout_millis: Option<u64>,
    sequence: u64,
) -> Result<Vec<u8>> {
    let lock = Value::Array(vec![
        Value::Array(vec![
            Value::Binary(parent.to_vec()),
            Value::Binary(dirent.to_vec()),
        ]),
        Value::Binary(lock_id.to_vec()),
    ]);
    let mut fields = vec![auth.to_value(), lock];
    if let Some(timeout) = timeout_millis {
        fields.push(Value::Unsigned(timeout));
    }
    encode_kv_call_at(method, Value::Array(fields), sequence)
}

pub fn encode_kv_get_root_request_at(auth: KvAuth<'_>, sequence: u64) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_GET_ROOT_METHOD_POSITION,
        Value::Array(vec![auth.to_value()]),
        sequence,
    )
}

pub fn encode_kv_get_request_at(
    auth: KvAuth<'_>,
    precondition: Option<&KvPathVersionVector>,
    parent: &[u8; 16],
    names: &[(u64, [u8; 32])],
    follow: u64,
    sequence: u64,
) -> Result<Vec<u8>> {
    let names = if names.is_empty() {
        Value::Null
    } else {
        Value::Array(
            names
                .iter()
                .map(|(version, mac)| {
                    Value::Array(vec![Value::Unsigned(*version), Value::Binary(mac.to_vec())])
                })
                .collect(),
        )
    };
    encode_kv_call_at(
        KV_GET_METHOD_POSITION,
        Value::Array(vec![
            kv_request_header(auth, precondition),
            Value::Array(vec![Value::Binary(parent.to_vec()), names]),
            Value::Unsigned(follow),
        ]),
        sequence,
    )
}

pub fn encode_kv_get_dir_request(auth: KvAuth<'_>, directory: &[u8; 16]) -> Result<Vec<u8>> {
    encode_kv_get_dir_request_at(auth, directory, 1)
}

pub fn encode_kv_get_dir_request_at(
    auth: KvAuth<'_>,
    directory: &[u8; 16],
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_GET_DIR_METHOD_POSITION,
        Value::Array(vec![auth.to_value(), Value::Binary(directory.to_vec())]),
        sequence,
    )
}

pub fn encode_kv_list_request(
    auth: KvAuth<'_>,
    directory: &[u8; 16],
    cursor: KvListCursor,
    number: u64,
    load_small_files: bool,
) -> Result<Vec<u8>> {
    encode_kv_list_request_at(auth, directory, cursor, number, load_small_files, 1)
}

pub fn encode_kv_list_request_at(
    auth: KvAuth<'_>,
    directory: &[u8; 16],
    cursor: KvListCursor,
    number: u64,
    load_small_files: bool,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_LIST_METHOD_POSITION,
        Value::Array(vec![
            auth.to_value(),
            Value::Binary(directory.to_vec()),
            Value::Array(vec![
                cursor.to_value(),
                Value::Unsigned(number),
                Value::Bool(load_small_files),
            ]),
        ]),
        sequence,
    )
}

pub fn encode_kv_get_node_request(auth: KvAuth<'_>, node: KvNodeId) -> Result<Vec<u8>> {
    encode_kv_get_node_request_at(auth, node, 1)
}

pub fn encode_kv_get_node_request_at(
    auth: KvAuth<'_>,
    node: KvNodeId,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_GET_NODE_METHOD_POSITION,
        Value::Array(vec![auth.to_value(), node.to_value()]),
        sequence,
    )
}

pub fn encode_kv_get_encrypted_chunk_request(
    auth: KvAuth<'_>,
    file: KvNodeId,
    offset: u64,
) -> Result<Vec<u8>> {
    encode_kv_get_encrypted_chunk_request_at(auth, file, offset, 1)
}

pub fn encode_kv_get_encrypted_chunk_request_at(
    auth: KvAuth<'_>,
    file: KvNodeId,
    offset: u64,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_GET_ENCRYPTED_CHUNK_METHOD_POSITION,
        Value::Array(vec![
            auth.to_value(),
            Value::Binary(file.object_id().to_vec()),
            Value::Unsigned(offset),
        ]),
        sequence,
    )
}

pub fn encode_kv_cache_check_request(
    auth: KvAuth<'_>,
    versions: &KvPathVersionVector,
) -> Result<Vec<u8>> {
    encode_kv_cache_check_request_at(auth, versions, 1)
}

pub fn encode_kv_cache_check_request_at(
    auth: KvAuth<'_>,
    versions: &KvPathVersionVector,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_CACHE_CHECK_METHOD_POSITION,
        Value::Array(vec![Value::Array(vec![
            auth.to_value(),
            versions.to_value(),
        ])]),
        sequence,
    )
}

pub fn encode_kv_usage_request_at(auth: KvAuth<'_>, sequence: u64) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_USAGE_METHOD_POSITION,
        Value::Array(vec![auth.to_value()]),
        sequence,
    )
}

pub(super) fn encode_kv_call_at(method: u64, argument: Value, sequence: u64) -> Result<Vec<u8>> {
    encode_call(KV_STORE_PROTOCOL_ID, method, &encode(&argument)?, sequence)
}
