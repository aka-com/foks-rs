//! Connection-reusing KV request encoding and response sequencing.

use std::io::Write as _;
use std::net::TcpStream;

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
    encode_kv_select_vhost_request, read_response, read_void_response, KvAuth, KvListCursor,
};

use crate::{authenticated_tls_roots, Error, FoksClient, PinnedHost, Result};

#[derive(Clone, Debug)]
pub(crate) enum KvRequest {
    Root,
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
    stream: rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
    next_sequence: u64,
    maximum_frame_length: usize,
}

impl KvConnection {
    pub(crate) fn call(&mut self, auth: KvAuth<'_>, request: &KvRequest) -> Result<Vec<u8>> {
        let sequence = self.next_sequence;
        let encoded = request.encode(auth, sequence)?;
        self.stream
            .write_all(&encoded)
            .map_err(foks_rpc::Error::Io)?;
        self.stream.flush().map_err(foks_rpc::Error::Io)?;
        let result = if request.is_void() {
            read_void_response(&mut self.stream, self.maximum_frame_length, sequence)
                .map(|()| Vec::new())
        } else {
            read_response(&mut self.stream, self.maximum_frame_length, sequence)
        };
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(Error::KvResponse("RPC sequence overflow"))?;
        result.map_err(Into::into)
    }
}

impl FoksClient {
    pub(crate) fn kv_connection_with_material(
        &self,
        host: &PinnedHost,
        seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<KvConnection> {
        let tcp = self.connect_tcp(&host.kv_store)?;
        let roots = authenticated_tls_roots(host)?;
        let config = self.tls_config_material(&roots, Some((seed, certificate_chain)))?;
        let mut tls = self.connect_tls(&host.kv_store, tcp, config)?;
        tls.write_all(&encode_kv_select_vhost_request(&host.host_id)?)
            .map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_void_response(&mut tls, self.maximum_frame_length, 0)?;
        Ok(KvConnection {
            stream: tls,
            next_sequence: 1,
            maximum_frame_length: self.maximum_frame_length,
        })
    }
}
