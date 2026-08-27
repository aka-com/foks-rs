//! Authoritative, synchronous SQLite storage for one standalone FOKS host.

#![forbid(unsafe_code)]

mod capabilities;
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
mod recovery;
mod schema;
mod team;
mod team_names;
mod transaction;
mod user_mutation;

pub use capabilities::TeamAdminAuthoritySnapshot;
pub use capabilities::TeamViewAuthoritySnapshot;
pub use certificates::{StoredCertificate, StoredCredentialBinding};
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
pub use read::{
    identity_snapshot, root_snapshot, IdentitySnapshot, PukMaterialSnapshot, RootSnapshot,
    TeamLinkSnapshot, TeamMemberSnapshot, TeamSnapshot, UserAuthoritySnapshot,
    UserChainLinkSnapshot, UserChainSnapshot, UserDeviceSnapshot, UserSharedKeySnapshot,
};
pub use receipts::Receipt;
pub use recovery::RecoveryCredentialSnapshot;
pub use schema::{APPLICATION_ID, SCHEMA_VERSION};
pub use team::{
    TeamHeader, TeamMemberMutation, TeamMutation, TeamMutationFailurePoint, TeamParcelMutation,
    TeamRemovalBoxMutation, TeamSeedChainMutation, TeamSharedKeyMutation,
};
pub use user_mutation::{
    AddedCredential, ParcelMutation, SeedChainMutation, SharedKeyMutation, UserMutation,
    UserMutationFailurePoint,
};
