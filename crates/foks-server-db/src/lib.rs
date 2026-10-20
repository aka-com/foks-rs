//! Authoritative, synchronous SQLite storage for one standalone FOKS host.

#![forbid(unsafe_code)]

mod allocation;
mod capabilities;
mod capability_keys;
mod capacity;
mod certificates;
mod chunk_binding;
mod chunk_collection;
mod chunk_ledger;
mod clock;
pub use chunk_binding::{ChunkStorageBinding, ChunkStorageBindingState};
pub use chunk_collection::ChunkDeletion;
pub use chunk_ledger::{
    ChunkInventoryCheckpoint, ChunkInventoryEntry, ChunkObject, ChunkPhysicalUsage,
    ChunkPublicationPolicy, ChunkReservation, ChunkReservationAction, CHUNK_TEMPORARY_PREFIX,
};
mod config;
mod connection;
mod device_nag;
mod error;
mod federation;
mod generic;
mod host;
mod host_rotation;
mod idempotency;
mod identity;
mod invites;
mod kv;
pub use kv::{
    KV_HISTORICAL_NAME_SELECTION_SQL, KV_LIST_INITIAL_SQL, KV_LIST_MAC_SQL, KV_LIST_TIME_SQL,
    KV_NAME_CONFLICT_SQL, KV_PUBLIC_USAGE_SQL,
};
mod kv_objects;
pub use kv_objects::{ExternalKvChunk, KvChunkKey, KvChunkPayload, StoredKvChunkObject};
mod log_upload;
mod maintenance;
mod merkle;
mod names;
mod passphrases;
mod read;
mod read_dependency;
pub use read_dependency::{ReadDependency, ReadDependencySchema};
pub use schema::read_dependency::{
    READ_DEPENDENCY_SCHEMA, READ_DEPENDENCY_TABLES, READ_DEPENDENCY_TRIGGERS,
};
mod realtime;
mod recovery;
mod schema;
mod sso;
mod sso_access;
mod sso_identity;
mod sso_policy;
mod team;
mod team_invitations;
pub use team_invitations::{
    CertificateUpload, InvitationActor, LocalInvitationAcceptance, RemoteInvitationAcceptance,
};
mod team_names;
mod transaction;
mod user_mutation;
mod waitlist;
mod yubi;

pub use capabilities::{
    TeamAdminAuthoritySnapshot, TeamAdminTokenActivation, TeamAdminTokenBinding,
    TeamAdminTokenIssue, TeamViewAuthoritySnapshot,
};
pub use capability_keys::{CapabilityKeyGeneration, CapabilityKeyGenerationState};
pub use capacity::{
    CapacityReport, CapacityResource, CapacityUsage, KvNamespaceUsage, RealtimeTeamUsage,
};
pub use certificates::{StoredCertificate, StoredCredentialBinding};
pub use clock::{Clock, SystemClock};
pub use config::Config;
pub use connection::{
    Database, DatabasePathIdentity, Pragmas, ReadDatabase, ReadDatabaseOpener, ReadSnapshot,
};
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
pub use idempotency::IdempotencyRecord;
pub use identity::{CommitOutcome, FailurePoint, IdentityMutation};
pub use invites::{
    InviteConsumption, InviteKind, InvitePolicy, InviteRegime, InviteSnapshot, IssuedInvite,
};
pub use kv::{
    KvDirectoryMutation, KvDirentMutation, KvFileChunkMutation, KvListCursor, KvNodeMutation,
    KvRootMutation, KvVersionCheck, StoredKvDirectory, StoredKvDirent, StoredKvFile,
    StoredKvFileChunk, StoredKvNode, StoredKvRoot,
};
pub use log_upload::{
    LogSendBlockMutation, LogSendFileMutation, MAXIMUM_LOG_SEND_BLOCKS,
    MAXIMUM_LOG_SEND_BLOCK_BYTES, MAXIMUM_LOG_SEND_FILES, MAXIMUM_LOG_SEND_FILE_BYTES,
};
pub use maintenance::{CheckpointOutcome, CheckpointReport, MaintenanceReport, StorageReport};
pub use merkle::SqliteNodeReader;
pub use passphrases::{PassphraseMutation, PassphraseSnapshot};
pub use read::{
    identity_snapshot, root_snapshot, GenericChainLinkSnapshot, GenericChainSnapshot,
    IdentitySnapshot, LocalTeamListEntrySnapshot, PukMaterialSnapshot, RootSnapshot,
    TeamLinkSnapshot, TeamMemberSnapshot, TeamRemovalSnapshot, TeamSnapshot, UserAuthoritySnapshot,
    UserChainLinkSnapshot, UserChainSnapshot, UserDeviceSnapshot, UserSharedKeySnapshot,
};
pub use realtime::{
    RealtimeActor, RealtimeCommit, RealtimeInboxState, RealtimeLimits, RealtimeReconcileOutcome,
    RealtimeReconcileReport, RealtimeReconcileState, RealtimeWakeTarget,
};
pub use recovery::RecoveryCredentialSnapshot;
pub use schema::{APPLICATION_ID, SCHEMA_VERSION};
pub use sso::{SsoSession, SsoSessionState};
pub use sso_access::{SsoAccess, SsoAccessState, SsoAccountBinding};
pub use sso_policy::{
    AuthorizationBinding, SsoAccessDecision, SsoPolicy, SsoPolicyTransition,
    SsoProviderBlockReason, SsoRolloutMode, SsoRolloutStatus,
};
pub use team::{
    TeamHeader, TeamLocalViewPermissionMutation, TeamMemberMutation, TeamMutation,
    TeamMutationFailurePoint, TeamParcelMutation, TeamRemoteMemberViewTokenMutation,
    TeamRemovalBoxMutation, TeamRemovalProofMutation, TeamSeedChainMutation, TeamSharedKeyMutation,
};
pub use user_mutation::{
    AddedCredential, ParcelMutation, SeedChainMutation, SharedKeyMutation, UserMutation,
    UserMutationFailurePoint, UsernameMutation,
};
pub use waitlist::{WaitlistEntry, WaitlistPurge};
pub use yubi::{SubkeyChallengeResult, YubiManagementKeySnapshot};

mod web_admin;
pub use web_admin::*;
