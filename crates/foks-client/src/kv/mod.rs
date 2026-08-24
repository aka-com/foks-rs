//! Authenticated FOKS KV synchronization and mutation sessions.

use std::path::PathBuf;

use foks_client_db::KvDirectoryProjection;
use foks_proto::{KvNodeId, KvParty, Role, SecretSeed};
use foks_rpc::KvAuth;

use self::rpc::KvConnection;
use crate::{FoksClient, PinnedHost, ProtectedMutationStore};

mod rpc;
mod support;
mod sync;
mod write;

// FOKS v0.1.9 applies the same limits to both sides of the large-file
// protocol. Keep them in one place so a file accepted by the writer can
// always be reconstructed by a fresh synchronization.
pub(crate) const MAX_KV_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const MAX_KV_UPLOAD_CHUNK: usize = 4 * 1024 * 1024;

#[cfg(test)]
pub(crate) use rpc::KvRequest;
#[cfg(test)]
pub(crate) use support::{read_kv_upload_chunk, read_kv_upload_chunk_with_carry};

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
}

#[derive(Debug)]
pub struct KvWriteResult {
    pub node_id: KvNodeId,
    pub dirent_version: u64,
    pub tree: Vec<KvDirectoryProjection>,
}
