//! Authoritative, synchronous SQLite storage for one standalone FOKS host.

#![forbid(unsafe_code)]

mod capabilities;
mod capability_keys;
mod certificates;
mod clock;
mod config;
mod connection;
mod device_nag;
mod error;
mod federation;
mod generic;
mod host;
mod host_rotation;
mod identity;
mod invites;
mod kv;
mod maintenance;
mod merkle;
mod names;
mod passphrases;
mod read;
mod receipts;
mod recovery;
mod schema;
mod team;
mod team_names;
mod transaction;
mod user_mutation;
mod yubi;

pub use capabilities::TeamAdminAuthoritySnapshot;
pub use capabilities::TeamViewAuthoritySnapshot;
pub use capability_keys::{CapabilityKeyGeneration, CapabilityKeyGenerationState};
pub use certificates::{StoredCertificate, StoredCredentialBinding};
pub use clock::{Clock, SystemClock};
pub use config::Config;
pub use connection::{Database, DatabasePathIdentity, Pragmas, ReadDatabase, ReadSnapshot};
pub use device_nag::DeviceNagSnapshot;
pub use error::{Error, Result};
pub use federation::{
    RemoteTeamViewGrant, RemoteTeamViewPermission, RemoteTeamViewPermissionOutcome,
    RemoteUserViewGrant, RemoteUserViewPermission, RemoteUserViewPermissionOutcome,
    StoredRemoteMemberViewToken, TeamGrantAuthority,
};
pub use generic::{
    GenericLinkMutation, GenericMutation, GenericPassphraseAction, GenericPassphraseInfo,
};
pub use host::{BootstrapService, HostBootstrap, StoredHostBootstrap};
pub use host_rotation::{
    HostKeyGeneration, HostKeyGenerationState, HostRotationOperation, HostRotationPhase,
    HostRotationPublication,
};
pub use identity::{CommitOutcome, FailurePoint, IdentityMutation};
pub use invites::{
    InviteConsumption, InviteKind, InvitePolicy, InviteRegime, InviteSnapshot, IssuedInvite,
};
pub use kv::{
    KvDirectoryMutation, KvDirentMutation, KvFileChunkMutation, KvListCursor, KvNodeMutation,
    KvRootMutation, KvVersionCheck, StoredKvDirectory, StoredKvDirent, StoredKvFile,
    StoredKvFileChunk, StoredKvNode, StoredKvRoot,
};
pub use maintenance::{CheckpointReport, MaintenanceReport, StorageReport};
pub use merkle::SqliteNodeReader;
pub use passphrases::{PassphraseMutation, PassphraseSnapshot};
pub use read::{
    identity_snapshot, root_snapshot, GenericChainLinkSnapshot, GenericChainSnapshot,
    IdentitySnapshot, LocalTeamListEntrySnapshot, PukMaterialSnapshot, RootSnapshot,
    TeamLinkSnapshot, TeamMemberSnapshot, TeamSnapshot, UserAuthoritySnapshot,
    UserChainLinkSnapshot, UserChainSnapshot, UserDeviceSnapshot, UserSharedKeySnapshot,
};
pub use receipts::Receipt;
pub use recovery::RecoveryCredentialSnapshot;
pub use schema::{APPLICATION_ID, SCHEMA_VERSION};
pub use team::{
    TeamHeader, TeamMemberMutation, TeamMutation, TeamMutationFailurePoint, TeamParcelMutation,
    TeamRemoteMemberViewTokenMutation, TeamRemovalBoxMutation, TeamSeedChainMutation,
    TeamSharedKeyMutation,
};
pub use user_mutation::{
    AddedCredential, ParcelMutation, SeedChainMutation, SharedKeyMutation, UserMutation,
    UserMutationFailurePoint,
};
pub use yubi::{SubkeyChallengeResult, YubiManagementKeySnapshot};
