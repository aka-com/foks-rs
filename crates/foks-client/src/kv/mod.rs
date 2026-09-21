//! Authenticated FOKS KV synchronization and mutation sessions.

use std::path::PathBuf;

use foks_client_db::KvDirectoryProjection;
use foks_proto::{KvNodeId, KvParty, Role, SecretSeed};
use foks_rpc::KvAuth;

use self::rpc::KvConnection;
use crate::{FoksClient, PinnedHost, ProtectedMutationStore};

#[cfg(test)]
mod path_tests;
mod rpc;
mod support;
mod sync;
mod write;

// FOKS v0.1.9 applies the same limits to both sides of the large-file
// protocol. Keep them in one place so a file accepted by the writer can
// always be reconstructed by a fresh synchronization.
pub(crate) const MAX_KV_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const MAX_KV_UPLOAD_CHUNK: usize = 4 * 1024 * 1024;

pub use sync::kv_path_version_vector;

#[cfg(test)]
pub(crate) use rpc::KvRequest;
#[cfg(test)]
pub(crate) use support::{read_kv_upload_chunk, read_kv_upload_chunk_with_carry};
#[cfg(test)]
pub(crate) use sync::{read_kv_chunk_with_fetch, read_kv_node_with_fetch};

pub(crate) struct KvPrivateKeyRef<'a> {
    pub(crate) role: Role,
    pub(crate) generation: u64,
    pub(crate) seed: &'a SecretSeed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KvWriteOptions {
    pub read_role: Role,
    pub write_role: Role,
    pub overwrite: bool,
    pub expected_version: Option<u64>,
}

enum OwnedKvAuth {
    User,
    Team([u8; 16]),
}

impl OwnedKvAuth {
    fn borrowed(&self) -> KvAuth<'_> {
        match self {
            Self::User => KvAuth::User,
            Self::Team(token) => KvAuth::Team(token),
        }
    }
}

/// One authenticated, connection-reusing KV session. Mutations use the
/// complete persisted version vector as an optimistic precondition and then
/// converge the SQLite projection through the ordinary incremental sync path.
pub struct KvWriteSession<'a> {
    client: &'a FoksClient,
    host: PinnedHost,
    party: KvParty,
    auth: OwnedKvAuth,
    private_keys: Vec<KvPrivateKeyRef<'a>>,
    protected_store: &'a mut dyn ProtectedMutationStore,
    soft_database_path: PathBuf,
    connection: KvConnection,
    adapter_parent: Option<[u8; 16]>,
    adapter_completion: bool,
    scope: Option<KvPathScope>,
}

/// One path this session resolved and the directories that named it, root
/// first. A mutation addressing the path's final directory cites exactly
/// these directories, so a peer's change to any of them is what the server's
/// precondition check rejects.
struct KvPathScope {
    components: Vec<Vec<u8>>,
    directories: Vec<KvDirectoryProjection>,
}

impl KvPathScope {
    /// The directory this path names, or `None` where the walk stopped short
    /// of the last component.
    fn parent(&self) -> Option<[u8; 16]> {
        if self.directories.len() != self.components.len() + 1 {
            return None;
        }
        self.directories
            .last()
            .map(|directory| directory.directory_id)
    }
}

#[derive(Debug)]
pub struct KvWriteResult {
    pub node_id: KvNodeId,
    pub dirent_version: u64,
    /// The directories along the mutated path, root first, as they were
    /// projected after the mutation. A path-scoped write never traverses the
    /// directories outside its path, so this is not the whole namespace.
    pub path: Vec<KvDirectoryProjection>,
}

/// Plaintext returned by an authenticated read of exactly one KV node.
///
/// Large files are represented separately so callers can use
/// [`KvFetchedChunk`] and avoid materializing the complete file. A directory
/// carries the read role of its active generation, which a path-scoped read
/// cannot take from the parent projection that named it.
///
/// A large file may carry an authenticated plaintext size extension. Legacy
/// and Go-created files omit it; callers still stream those files until the
/// end-of-file flag rather than downloading content just to measure it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KvFetchedNode {
    Directory { read_role: Role },
    SmallFile(Vec<u8>),
    Symlink(Vec<u8>),
    LargeFile { size: Option<u64> },
}

/// One requested range of a large KV file. The plaintext is caller-owned and
/// is never written to the client's soft-state projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvFetchedChunk {
    pub content: Vec<u8>,
    pub eof: bool,
}

impl crate::FoksClient {
    /// Complete only the local tail of an already verified KV journal.
    pub fn finalize_verified_kv_material<S: crate::ProtectedMutationStore + ?Sized>(
        &self,
        database: &std::path::Path,
        protected: &mut S,
        operation: &foks_client_db::MutationOperation,
    ) -> crate::Result<()> {
        write::finalize_verified_material(database, protected, operation)
    }
}
