//! Authoritative, synchronous SQLite storage for one standalone FOKS host.

#![forbid(unsafe_code)]

mod certificates;
mod clock;
mod config;
mod connection;
mod error;
mod host;
mod identity;
mod kv;
mod maintenance;
mod merkle;
mod names;
mod read;
mod receipts;
mod schema;
mod transaction;

pub use certificates::StoredCertificate;
pub use clock::{Clock, SystemClock};
pub use config::Config;
pub use connection::{Database, Pragmas, ReadDatabase};
pub use error::{Error, Result};
pub use host::{BootstrapService, HostBootstrap, StoredHostBootstrap};
pub use identity::{CommitOutcome, FailurePoint, IdentityMutation};
pub use kv::{
    KvDirectoryMutation, KvDirentMutation, KvFileChunkMutation, KvNodeMutation, KvRootMutation,
    StoredKvDirectory, StoredKvDirent, StoredKvFile, StoredKvFileChunk, StoredKvNode, StoredKvRoot,
};
pub use maintenance::{CheckpointReport, MaintenanceReport, StorageReport};
pub use merkle::SqliteNodeReader;
pub use read::{identity_snapshot, root_snapshot, IdentitySnapshot, RootSnapshot};
pub use receipts::Receipt;
pub use schema::{APPLICATION_ID, SCHEMA_VERSION};
