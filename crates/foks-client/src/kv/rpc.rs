//! Connection-reusing KV request encoding and response sequencing.

use foks_proto::{
    KvDirectory, KvDirent, KvLargeFileMetadata, KvNodeId, KvPathVersionVector, KvRoot,
    KvSmallFileBox, KvUploadChunk, SecretSeed,
};
use foks_rpc::{
    encode_kv_cache_check_request_at, encode_kv_file_upload_chunk_request_at,
    encode_kv_file_upload_init_request_at, encode_kv_get_dir_request_at,
    encode_kv_get_encrypted_chunk_request_at, encode_kv_get_node_request_at,
    encode_kv_get_root_request_at, encode_kv_list_request_at, encode_kv_lock_acquire_request_at,
    encode_kv_lock_release_request_at, encode_kv_mkdir_request_at, encode_kv_put_request_at,
    encode_kv_put_root_request_at, encode_kv_put_small_file_or_symlink_request_at,
    encode_kv_select_vhost_request, encode_kv_usage_request_at, KvAuth, KvListCursor,
};

use crate::{FoksClient, PinnedHost, PooledConnection, Result};

#[derive(Clone, Debug)]
pub(crate) enum KvRequest {
    Root,
    Usage,
    Directory([u8; 16]),
    List {
        directory: [u8; 16],
        cursor: KvListCursor,
        number: u64,
        load_small_files: bool,
    },
    Node(KvNodeId),
    Chunk {
        file: KvNodeId,
        offset: u64,
    },
    CacheCheck(KvPathVersionVector),
    Mkdir {
        precondition: Option<KvPathVersionVector>,
        directory: KvDirectory,
    },
    Put {
        precondition: KvPathVersionVector,
        dirents: Vec<KvDirent>,
    },
    PutSmall {
        id: KvNodeId,
        boxed: KvSmallFileBox,
    },
    UploadInit {
        file_id: [u8; 16],
        metadata: KvLargeFileMetadata,
        chunk: KvUploadChunk,
    },
    UploadChunk {
        file_id: [u8; 16],
        chunk: KvUploadChunk,
    },
    PutRoot(KvRoot),
    LockAcquire {
        parent: [u8; 16],
        dirent: [u8; 16],
        lock_id: [u8; 16],
        timeout_millis: u64,
    },
    LockRelease {
        parent: [u8; 16],
        dirent: [u8; 16],
        lock_id: [u8; 16],
    },
}

impl KvRequest {
    pub(crate) fn encode(&self, auth: KvAuth<'_>, sequence: u64) -> Result<Vec<u8>> {
        match self {
            Self::Root => Ok(encode_kv_get_root_request_at(auth, sequence)?),
            Self::Usage => Ok(encode_kv_usage_request_at(auth, sequence)?),
            Self::Directory(directory) => {
                Ok(encode_kv_get_dir_request_at(auth, directory, sequence)?)
            }
            Self::List {
                directory,
                cursor,
                number,
                load_small_files,
            } => Ok(encode_kv_list_request_at(
                auth,
                directory,
                *cursor,
                *number,
                *load_small_files,
                sequence,
            )?),
            Self::Node(node) => Ok(encode_kv_get_node_request_at(auth, *node, sequence)?),
            Self::Chunk { file, offset } => Ok(encode_kv_get_encrypted_chunk_request_at(
                auth, *file, *offset, sequence,
            )?),
            Self::CacheCheck(versions) => {
                Ok(encode_kv_cache_check_request_at(auth, versions, sequence)?)
            }
            Self::Mkdir {
                precondition,
                directory,
            } => Ok(encode_kv_mkdir_request_at(
                auth,
                precondition.as_ref(),
                directory,
                sequence,
            )?),
            Self::Put {
                precondition,
                dirents,
            } => Ok(encode_kv_put_request_at(
                auth,
                Some(precondition),
                dirents,
                sequence,
            )?),
            Self::PutSmall { id, boxed } => Ok(encode_kv_put_small_file_or_symlink_request_at(
                auth, *id, boxed, sequence,
            )?),
            Self::UploadInit {
                file_id,
                metadata,
                chunk,
            } => Ok(encode_kv_file_upload_init_request_at(
                auth, *file_id, metadata, chunk, sequence,
            )?),
            Self::UploadChunk { file_id, chunk } => Ok(encode_kv_file_upload_chunk_request_at(
                auth, *file_id, chunk, sequence,
            )?),
            Self::PutRoot(root) => Ok(encode_kv_put_root_request_at(auth, root, sequence)?),
            Self::LockAcquire {
                parent,
                dirent,
                lock_id,
                timeout_millis,
            } => Ok(encode_kv_lock_acquire_request_at(
                auth,
                *parent,
                *dirent,
                *lock_id,
                *timeout_millis,
                sequence,
            )?),
            Self::LockRelease {
                parent,
                dirent,
                lock_id,
            } => Ok(encode_kv_lock_release_request_at(
                auth, *parent, *dirent, *lock_id, sequence,
            )?),
        }
    }

    fn is_void(&self) -> bool {
        matches!(
            self,
            Self::CacheCheck(_)
                | Self::Mkdir { .. }
                | Self::Put { .. }
                | Self::PutSmall { .. }
                | Self::UploadInit { .. }
                | Self::UploadChunk { .. }
                | Self::PutRoot(_)
                | Self::LockAcquire { .. }
                | Self::LockRelease { .. }
        )
    }
}

pub(crate) struct KvConnection {
    pooled: PooledConnection,
}

impl KvConnection {
    pub(crate) fn call(&mut self, auth: KvAuth<'_>, request: &KvRequest) -> Result<Vec<u8>> {
        let encoded = request.encode(auth, 0)?;
        self.pooled.call(&encoded, request.is_void())
    }
}

impl FoksClient {
    pub(crate) fn kv_connection_with_material(
        &self,
        host: &PinnedHost,
        seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<KvConnection> {
        let select = encode_kv_select_vhost_request(&host.host_id)?;
        Ok(KvConnection {
            pooled: self.pooled_connection_with_material(
                host,
                &host.kv_store,
                seed,
                certificate_chain,
                &select,
            )?,
        })
    }
}
