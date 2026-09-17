//! Durable SQLite hard state and isolated soft projections for a native FOKS client.
//!
//! This crate is intentionally below the protocol verifier. It preserves the
//! exact signed bytes supplied by that verifier and enforces monotonic pins,
//! but it does not parse Snowpack or verify signatures itself.

#![forbid(unsafe_code)]

mod chat_limits;
pub use chat_limits::ChatLimits;
mod schema;
mod soft;
mod soft_schema;

pub use repositories::protected::{ProtectedOwnerCursor, ProtectedRecordOwner};

pub use repositories::import_readiness::{
    ImportAccount, ImportAccountKind, ImportReadiness, VerifiedImportAccount,
};
pub use repositories::sso::{SsoFlow, SsoFlowState};

pub use repositories::chat::{
    ChatAnchor, ChatOperation, ChatOperationKind, ChatOperationState, ChatScope, ChatSubmission,
};

pub use soft::{
    ChatInboxEntry, ChatInboxScope, ChatInboxState, KnownStore, KnownTeamStore,
    KvDirectoryProjection, KvLargeFileStage, KvProjectedEntry, SoftStateStore, MAX_DISCOVERY_HINTS,
};

use foks_proto::ServiceType;
use foks_snowpack::{decode, encode, Value};
use foks_verify::{
    AuthenticatedMerkleRoot, HostService, MerkleRootEvidence, VerifiedHostSnapshot,
    VerifiedHostSnapshotParts, VerifiedMerkleRoot, VerifiedMerkleRootParts, VerifiedTeamMember,
    VerifiedTeamSnapshot, VerifiedTeamSnapshotParts, VerifiedUserSnapshot,
    VerifiedUserSnapshotParts,
};
use rusqlite::{params, Connection, OptionalExtension as _, Transaction, TransactionBehavior};
use thiserror::Error;

pub use foks_proto::SubmissionHandle;
pub use repositories::adapter::{
    adapter_handle_hash, AdapterClockState, AdapterLedgerState, AdapterSubmission,
    AdapterTimeSample, ADMISSION_AGE_SECONDS, TERMINAL_RETENTION_SECONDS,
};
use schema::{APPLICATION_ID, INITIAL as SCHEMA, VERSION as SCHEMA_VERSION};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredHostSnapshot {
    pub lookup_name: String,
    pub host_id: Vec<u8>,
    pub canonical_name: String,
    pub genesis_key: Vec<u8>,
    pub chain_seqno: u64,
    pub chain_tail_hash: [u8; 32],
    pub chain_bytes: Vec<u8>,
    pub public_zone_bytes: Vec<u8>,
    pub services: Vec<HostService>,
    pub merkle_root: StoredMerkleRoot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HardStateMetadata {
    pub database_id: [u8; 16],
    pub revision: u64,
    pub write_token: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredMerkleRoot {
    pub epoch: u64,
    pub root_hash: [u8; 32],
    pub root_bytes: Vec<u8>,
    pub evidence: MerkleRootEvidence,
    pub authenticated_roots: Vec<AuthenticatedMerkleRoot>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StoredRole {
    role_type: u64,
    visibility: i64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StoredUserDevice {
    device_id: Vec<u8>,
    role: StoredRole,
    hepk_bytes: Vec<u8>,
    subkey_id: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StoredUserSharedKey {
    role: StoredRole,
    generation: u64,
    verify_key: Vec<u8>,
    hepk_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredUserSnapshot {
    pub host_id: Vec<u8>,
    pub uid: Vec<u8>,
    pub chain_seqno: u64,
    pub chain_tail_hash: [u8; 32],
    pub chain_bytes: Vec<u8>,
    pub evidence_bytes: Vec<u8>,
    pub username: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub username_sequence: u64,
    pub merkle_epoch: u64,
    pub merkle_root_hash: [u8; 32],
    pub merkle_root_bytes: Vec<u8>,
    pub devices: Vec<foks_verify::VerifiedUserDevice>,
    pub shared_keys: Vec<foks_verify::VerifiedUserSharedKey>,
}

pub struct VerifiedUserGenericChainSnapshot<'a> {
    pub host_id: &'a [u8],
    pub uid: &'a [u8],
    pub chain_type: u64,
    pub sequence: u64,
    pub tail_hash: Option<[u8; 32]>,
    pub chain_bytes: &'a [u8],
    pub merkle_epoch: u64,
    pub merkle_root_hash: [u8; 32],
}

impl StoredUserSnapshot {
    pub fn parts(&self) -> VerifiedUserSnapshotParts<'_> {
        VerifiedUserSnapshotParts {
            host_id: &self.host_id,
            uid: &self.uid,
            chain_seqno: self.chain_seqno,
            chain_tail_hash: self.chain_tail_hash,
            chain_bytes: &self.chain_bytes,
            evidence_bytes: &self.evidence_bytes,
            username: &self.username,
            username_utf8: &self.username_utf8,
            username_sequence: self.username_sequence,
            merkle_epoch: self.merkle_epoch,
            merkle_root_hash: self.merkle_root_hash,
            merkle_root_bytes: &self.merkle_root_bytes,
            devices: &self.devices,
            shared_keys: &self.shared_keys,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredTeamSnapshot {
    pub host_id: Vec<u8>,
    pub team_id: Vec<u8>,
    pub chain_seqno: u64,
    pub chain_tail_hash: [u8; 32],
    pub chain_bytes: Vec<u8>,
    pub evidence_bytes: Vec<u8>,
    pub team_name: Vec<u8>,
    pub team_name_utf8: Vec<u8>,
    pub team_name_sequence: u64,
    pub index_range: foks_proto::RationalRange,
    pub merkle_epoch: u64,
    pub merkle_root_hash: [u8; 32],
    pub merkle_root_bytes: Vec<u8>,
    pub members: Vec<VerifiedTeamMember>,
    pub shared_keys: Vec<foks_verify::VerifiedUserSharedKey>,
}

/// Application-level work that can be safely resumed after a process crash.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ScheduledJobKind {
    UserRefresh = 1,
    MutationReconcile = 2,
    YubiManagementRefresh = 3,
    FederationRefresh = 4,
    TeamRefresh = 5,
}

impl ScheduledJobKind {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::UserRefresh),
            2 => Ok(Self::MutationReconcile),
            3 => Ok(Self::YubiManagementRefresh),
            4 => Ok(Self::FederationRefresh),
            5 => Ok(Self::TeamRefresh),
            _ => Err(Error::InvalidScheduledJob("unknown scheduled job kind")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledJob {
    pub job_id: [u8; 16],
    pub kind: ScheduledJobKind,
    pub host_id: Vec<u8>,
    pub scope_id: Vec<u8>,
    pub interval_micros: u64,
    pub next_run_at: u64,
    pub failure_count: u64,
    pub lease_until: Option<u64>,
    pub last_completed_at: Option<u64>,
    pub last_error: Option<String>,
    pub updated_at: u64,
}

impl StoredTeamSnapshot {
    pub fn parts(&self) -> VerifiedTeamSnapshotParts<'_> {
        VerifiedTeamSnapshotParts {
            host_id: &self.host_id,
            team_id: &self.team_id,
            chain_seqno: self.chain_seqno,
            chain_tail_hash: self.chain_tail_hash,
            chain_bytes: &self.chain_bytes,
            evidence_bytes: &self.evidence_bytes,
            team_name: &self.team_name,
            team_name_utf8: &self.team_name_utf8,
            team_name_sequence: self.team_name_sequence,
            index_range: &self.index_range,
            merkle_epoch: self.merkle_epoch,
            merkle_root_hash: self.merkle_root_hash,
            merkle_root_bytes: &self.merkle_root_bytes,
            members: &self.members,
            shared_keys: &self.shared_keys,
        }
    }
}

/// The durable result of accepting a verified snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Acceptance {
    Inserted,
    Advanced,
    Unchanged,
}

/// Public mutation classes recorded by the generic write-ahead journal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MutationKind {
    Signup = 1,
    DeviceProvision = 2,
    DeviceRevoke = 3,
    PukRotation = 4,
    KvNamespace = 5,
    KvContent = 6,
    KvRoot = 7,
    KvAdapter = 8,
    UsernameChange = 9,
    BotEnrollment = 10,
    Invitation = 11,
}

impl MutationKind {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Signup),
            2 => Ok(Self::DeviceProvision),
            3 => Ok(Self::DeviceRevoke),
            4 => Ok(Self::PukRotation),
            5 => Ok(Self::KvNamespace),
            6 => Ok(Self::KvContent),
            7 => Ok(Self::KvRoot),
            8 => Ok(Self::KvAdapter),
            9 => Ok(Self::UsernameChange),
            10 => Ok(Self::BotEnrollment),
            11 => Ok(Self::Invitation),
            _ => Err(Error::InvalidMutationOperation("unknown operation kind")),
        }
    }
}

/// Crash-recovery state. `SubmissionUnknown` means the request crossed the
/// process boundary but no definitive response was durably observed; callers
/// must reconcile authenticated server state and must not blindly replay it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MutationState {
    Prepared = 1,
    Submitting = 2,
    SubmissionUnknown = 3,
    RemoteVerified = 4,
    Rejected = 5,
    Finalized = 6,
}

impl MutationState {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Prepared),
            2 => Ok(Self::Submitting),
            3 => Ok(Self::SubmissionUnknown),
            4 => Ok(Self::RemoteVerified),
            5 => Ok(Self::Rejected),
            6 => Ok(Self::Finalized),
            _ => Err(Error::InvalidMutationOperation("unknown operation state")),
        }
    }

    fn can_transition_to(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (Self::Prepared, Self::Submitting | Self::Rejected)
                    | (
                        Self::Submitting,
                        Self::SubmissionUnknown | Self::RemoteVerified | Self::Rejected
                    )
                    | (
                        Self::SubmissionUnknown,
                        Self::RemoteVerified | Self::Rejected
                    )
                    | (Self::RemoteVerified, Self::Finalized)
            )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Finalized | Self::Rejected)
    }
}

/// A public write-ahead record. `material_ref` is an opaque lookup key for a
/// separately protected store and is not secret material itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationOperation {
    pub operation_id: [u8; 16],
    pub kind: MutationKind,
    pub host_id: Vec<u8>,
    pub scope_id: Vec<u8>,
    pub subject_id: Vec<u8>,
    pub expected_version: Option<u64>,
    pub request_hash: [u8; 32],
    pub material_ref: Vec<u8>,
    pub material_hash: [u8; 32],
    pub state: MutationState,
    pub attempt_count: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SignupOperationState {
    Prepared = 1,
    Submitted = 2,
    Verified = 3,
}

impl SignupOperationState {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Prepared),
            2 => Ok(Self::Submitted),
            3 => Ok(Self::Verified),
            _ => Err(Error::InvalidSignupOperation("unknown operation state")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignupOperation {
    pub operation_id: [u8; 16],
    pub host_id: Vec<u8>,
    pub normalized_username: Vec<u8>,
    pub uid: Vec<u8>,
    pub device_id: Vec<u8>,
    pub request_hash: [u8; 32],
    pub state: SignupOperationState,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AdHocTeamOperationState {
    Prepared = 1,
    Submitted = 2,
    Verified = 3,
}

impl AdHocTeamOperationState {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Prepared),
            2 => Ok(Self::Submitted),
            3 => Ok(Self::Verified),
            _ => Err(Error::InvalidAdHocTeamOperation("unknown operation state")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdHocTeamOperation {
    pub operation_id: [u8; 16],
    pub host_id: Vec<u8>,
    pub uid: Vec<u8>,
    pub device_id: Vec<u8>,
    pub team_id: Vec<u8>,
    pub request_hash: [u8; 32],
    pub state: AdHocTeamOperationState,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TeamMutationKind {
    NamedCreation = 1,
    MembershipChange = 2,
    PtkRotation = 3,
    MetadataChange = 4,
}

impl TeamMutationKind {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::NamedCreation),
            2 => Ok(Self::MembershipChange),
            3 => Ok(Self::PtkRotation),
            4 => Ok(Self::MetadataChange),
            _ => Err(Error::InvalidTeamMutation("unknown operation kind")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TeamMutationState {
    Prepared = 1,
    Submitting = 2,
    SubmissionUnknown = 3,
    Submitted = 4,
    Verified = 5,
    Rejected = 6,
    Superseded = 7,
}

impl TeamMutationState {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Prepared),
            2 => Ok(Self::Submitting),
            3 => Ok(Self::SubmissionUnknown),
            4 => Ok(Self::Submitted),
            5 => Ok(Self::Verified),
            6 => Ok(Self::Rejected),
            7 => Ok(Self::Superseded),
            _ => Err(Error::InvalidTeamMutation("unknown operation state")),
        }
    }

    fn can_transition_to(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (
                    Self::Prepared,
                    Self::Submitting | Self::Rejected | Self::Superseded
                ) | (
                    Self::Submitting,
                    Self::SubmissionUnknown
                        | Self::Submitted
                        | Self::Verified
                        | Self::Rejected
                        | Self::Superseded
                ) | (
                    Self::SubmissionUnknown,
                    Self::Submitted | Self::Verified | Self::Rejected | Self::Superseded
                ) | (
                    Self::Submitted,
                    Self::Verified | Self::Rejected | Self::Superseded
                )
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamMutationOperation {
    pub operation_id: [u8; 16],
    pub kind: TeamMutationKind,
    pub host_id: Vec<u8>,
    pub actor_id: Vec<u8>,
    pub device_id: Vec<u8>,
    pub team_id: Vec<u8>,
    pub expected_seqno: u64,
    pub request_hash: [u8; 32],
    pub state: TeamMutationState,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Monotonic recovery stages for admitting a remote party to a local team.
/// The journal never stores the bearer permission or the removal key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FederationSagaState {
    PermissionGranted = 1,
    RemoteVerified = 2,
    LocalPrepared = 3,
    LocalVerified = 4,
    Completed = 5,
    Rejected = 6,
}

impl FederationSagaState {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::PermissionGranted),
            2 => Ok(Self::RemoteVerified),
            3 => Ok(Self::LocalPrepared),
            4 => Ok(Self::LocalVerified),
            5 => Ok(Self::Completed),
            6 => Ok(Self::Rejected),
            _ => Err(Error::InvalidFederationSaga("unknown saga state")),
        }
    }

    fn can_transition_to(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (Self::PermissionGranted, Self::RemoteVerified)
                    | (Self::LocalPrepared, Self::LocalVerified)
                    | (Self::LocalVerified, Self::Completed)
                    | (
                        Self::PermissionGranted
                            | Self::RemoteVerified
                            | Self::LocalPrepared
                            | Self::LocalVerified,
                        Self::Rejected
                    )
            )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Rejected)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FederationSagaOperation {
    pub operation_id: [u8; 16],
    pub local_host_id: Vec<u8>,
    pub remote_host_id: Vec<u8>,
    pub actor_id: Vec<u8>,
    pub local_team_id: Vec<u8>,
    pub remote_party_id: Vec<u8>,
    pub permission_hash: [u8; 32],
    pub destination_role_type: u64,
    pub destination_visibility: i64,
    pub removal_key_commitment: [u8; 32],
    pub state: FederationSagaState,
    pub expected_local_seqno: Option<u64>,
    pub local_mutation_id: Option<[u8; 16]>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("import readiness failed: {0}")]
    ImportReadiness(&'static str),
    #[error("state snapshot inspection failed: {0}")]
    SnapshotInspection(&'static str),
    #[error("submission handle is expired; a new explicit write requires a new handle")]
    AdapterExpired,
    #[error("adapter clock is untrusted; use local clock re-anchoring before new writes")]
    AdapterClockUntrusted,
    #[error("submission handle is too far in the future")]
    AdapterFutureHandle,
    #[error("adapter retention-full; retained evidence cannot be discarded to admit a write")]
    AdapterRetentionFull,
    #[error("adapter active submission capacity is full; recover existing work first")]
    AdapterActiveFull,
    #[error("submission handle is bound to different inputs or authority")]
    AdapterIdentityConflict,
    #[error("adapter cleanup is deferred while protected material or child evidence remains")]
    AdapterCleanupDeferred,

    #[error("OIDC flow state: {0}")]
    SsoState(&'static str),
    #[error("chat state conflict: {0}")]
    ChatConflict(&'static str),
    #[error("invalid chat operation transition: {0}")]
    ChatOperationState(&'static str),
    #[error("chat capacity exceeded: {0}")]
    ChatLimit(&'static str),
    #[error("chat operation not found: {0}")]
    ChatNotFound(&'static str),
    #[error("invalid chat inbox state: {0}")]
    InvalidChatInbox(&'static str),
    #[error("hard-state filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite hard-state operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid persisted Merkle evidence: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("invalid verified host snapshot: {0}")]
    InvalidSnapshot(&'static str),
    #[error("hard-state database path must not be a symbolic link")]
    SymlinkDatabase,
    #[error("hard-state database has application id {found:#x}, expected {expected:#x}")]
    WrongApplicationId { found: i64, expected: i64 },
    #[error(
        "system store is out of date (v{found}), could not auto-update to current version (v{supported})"
    )]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("lookup name {lookup_name:?} is pinned to a different HostID")]
    HostIdentityChanged { lookup_name: String },
    #[error("genesis key does not match the pinned HostID")]
    GenesisChanged,
    #[error("host chain rolled back from sequence {stored} to {received}")]
    ChainRollback { stored: u64, received: u64 },
    #[error("host chain forked at sequence {seqno}")]
    ChainFork { seqno: u64 },
    #[error("host projection changed without a host-chain advance at sequence {seqno}")]
    ProjectionChanged { seqno: u64 },
    #[error("Merkle root rolled back from epoch {stored} to {received}")]
    MerkleRollback { stored: u64, received: u64 },
    #[error("Merkle root forked at epoch {epoch}")]
    MerkleFork { epoch: u64 },
    #[error("verified user references a host that is not pinned")]
    UnknownHost,
    #[error("invalid verified user snapshot: {0}")]
    InvalidUser(&'static str),
    #[error("user chain rolled back from sequence {stored} to {received}")]
    UserRollback { stored: u64, received: u64 },
    #[error("user chain forked at sequence {seqno}")]
    UserFork { seqno: u64 },
    #[error("user projection changed without a chain advance at sequence {seqno}")]
    UserProjectionChanged { seqno: u64 },
    #[error(
        "user generic chain type {chain_type} rolled back from sequence {stored} to {received}"
    )]
    UserGenericRollback {
        chain_type: u64,
        stored: u64,
        received: u64,
    },
    #[error("user generic chain type {chain_type} forked at sequence {seqno}")]
    UserGenericFork { chain_type: u64, seqno: u64 },
    #[error("invalid verified team snapshot: {0}")]
    InvalidTeam(&'static str),
    #[error("team chain rolled back from sequence {stored} to {received}")]
    TeamRollback { stored: u64, received: u64 },
    #[error("team chain forked at sequence {seqno}")]
    TeamFork { seqno: u64 },
    #[error("team projection changed without a chain advance at sequence {seqno}")]
    TeamProjectionChanged { seqno: u64 },
    #[error("{field} value {value} cannot be represented by SQLite")]
    IntegerOutOfRange { field: &'static str, value: u64 },
    #[error("stored {field} value {value} cannot be represented by the protocol")]
    StoredIntegerOutOfRange { field: &'static str, value: i64 },
    #[error("soft-state database has application id {found:#x}, expected {expected:#x}")]
    WrongSoftApplicationId { found: i64, expected: i64 },
    #[error("soft-state database permissions {0:#o} allow group or other access")]
    InsecureSoftPermissions(u32),
    #[error("unsupported soft-state cache schema version {found} at {path}; this build supports version {supported} (cache must be recreated)")]
    UnsupportedSoftSchema {
        path: String,
        found: u32,
        supported: u32,
    },
    #[error("invalid verified KV projection")]
    InvalidKvProjection,
    #[error("persisted federation discovery hint is malformed")]
    InvalidDiscoveryHint,
    #[error("persisted known store is malformed")]
    InvalidKnownStore,
    #[error("KV root rolled back from version {stored} to {received}")]
    KvRootRollback { stored: u64, received: u64 },
    #[error("KV directory rolled back from version {stored} to {received}")]
    KvDirectoryRollback { stored: u64, received: u64 },
    #[error("KV projection conflict: {0}")]
    KvProjectionConflict(&'static str),
    #[error("invalid signup operation: {0}")]
    InvalidSignupOperation(&'static str),
    #[error("invalid ad-hoc team operation: {0}")]
    InvalidAdHocTeamOperation(&'static str),
    #[error("invalid named-team mutation operation: {0}")]
    InvalidTeamMutation(&'static str),
    #[error("invalid federation saga operation: {0}")]
    InvalidFederationSaga(&'static str),
    #[error("invalid generic mutation operation: {0}")]
    InvalidMutationOperation(&'static str),
    #[error("invalid scheduled job: {0}")]
    InvalidScheduledJob(&'static str),
    #[error("OS randomness is unavailable")]
    Randomness,
    #[error("hard-state revision metadata is malformed")]
    InvalidHardStateMetadata,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A connection-owning hard-state repository.
///
/// Callers should keep one instance on their storage worker thread. The API is
/// synchronous by design so a future generic SQLite runtime can own and invoke
/// it without this crate taking an async-runtime dependency.
pub struct HardStateStore {
    connection: Connection,
}

mod repositories;

#[derive(Debug)]
struct StoredHost {
    canonical_name: String,
    genesis_key: Vec<u8>,
    chain_seqno: u64,
    chain_tail_hash: [u8; 32],
    chain_bytes: Vec<u8>,
    public_zone_bytes: Vec<u8>,
}

fn initialize_or_verify(connection: &mut Connection) -> Result<()> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id == 0 && version == 0 {
        let object_count: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if object_count != 0 {
            return Err(Error::WrongApplicationId {
                found: application_id,
                expected: APPLICATION_ID,
            });
        }
        let mut database_id = [0u8; 16];
        getrandom::fill(&mut database_id).map_err(|_| Error::Randomness)?;
        let mut write_token = [0u8; 16];
        getrandom::fill(&mut write_token).map_err(|_| Error::Randomness)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA)?;
        transaction.execute(
            "INSERT INTO hard_state_metadata
                 (singleton, database_id, hard_state_revision, write_token)
             VALUES (1, ?1, 0, ?2)",
            rusqlite::params![database_id.as_slice(), write_token.as_slice()],
        )?;
        install_revision_triggers(&transaction)?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;
        return Ok(());
    }
    if application_id != APPLICATION_ID {
        return Err(Error::WrongApplicationId {
            found: application_id,
            expected: APPLICATION_ID,
        });
    }
    if version != SCHEMA_VERSION {
        return Err(Error::UnsupportedSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn install_revision_triggers(transaction: &Transaction<'_>) -> Result<()> {
    for table in schema::REVISION_TABLES {
        for operation in ["insert", "update", "delete"] {
            transaction.execute_batch(&format!(
                "CREATE TRIGGER hard_state_revision_{table}_{operation}
                 AFTER {operation} ON {table}
                 BEGIN
                   UPDATE hard_state_metadata
                   SET hard_state_revision = hard_state_revision + 1,
                       write_token = randomblob(16)
                   WHERE singleton = 1;
                 END;"
            ))?;
        }
    }
    Ok(())
}

fn validate_snapshot(snapshot: VerifiedHostSnapshotParts<'_>) -> Result<()> {
    if snapshot.lookup_name.trim().is_empty() {
        return Err(Error::InvalidSnapshot("lookup name must not be empty"));
    }
    if snapshot.host_id.is_empty() {
        return Err(Error::InvalidSnapshot("HostID must not be empty"));
    }
    if snapshot.canonical_name.trim().is_empty() {
        return Err(Error::InvalidSnapshot("canonical name must not be empty"));
    }
    if snapshot.genesis_key.is_empty() {
        return Err(Error::InvalidSnapshot("genesis key must not be empty"));
    }
    if snapshot.chain_bytes.is_empty() {
        return Err(Error::InvalidSnapshot("host-chain bytes must not be empty"));
    }
    if snapshot.public_zone_bytes.is_empty() {
        return Err(Error::InvalidSnapshot(
            "signed public-zone bytes must not be empty",
        ));
    }
    if snapshot.merkle_root.root_bytes.is_empty() {
        return Err(Error::InvalidSnapshot(
            "Merkle-root bytes must not be empty",
        ));
    }
    validate_merkle_root(snapshot.merkle_root)?;
    if snapshot
        .services
        .iter()
        .any(|service| service.endpoint_bytes.is_empty())
    {
        return Err(Error::InvalidSnapshot("service endpoint must not be empty"));
    }
    Ok(())
}

fn validate_signup_operation(operation: &SignupOperation) -> Result<()> {
    if operation.host_id.len() != 33
        || !(3..=25).contains(&operation.normalized_username.len())
        || operation.uid.len() != 33
        || operation.device_id.len() != 33
        || operation.created_at > operation.updated_at
    {
        return Err(Error::InvalidSignupOperation(
            "public operation fields are malformed",
        ));
    }
    Ok(())
}

fn validate_mutation_operation(operation: &MutationOperation) -> Result<()> {
    if operation.host_id.len() != 33
        || !matches!(operation.scope_id.len(), 0 | 16 | 33 | 34)
        || !matches!(operation.subject_id.len(), 0 | 16 | 33 | 34)
        || operation.material_ref.is_empty()
        || operation.material_ref.len() > 255
        || operation.updated_at < operation.created_at
    {
        return Err(Error::InvalidMutationOperation(
            "operation fields are malformed",
        ));
    }
    Ok(())
}

fn validate_scheduled_job(job: &ScheduledJob) -> Result<()> {
    if job.host_id.len() != 33 || job.scope_id.len() > 1024 || job.interval_micros == 0 {
        return Err(Error::InvalidScheduledJob(
            "scheduled job fields are malformed",
        ));
    }
    Ok(())
}

fn scheduled_job_from_row(
    job_id: &[u8],
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ScheduledJob> {
    fn invalid(index: usize, message: &'static str) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            std::io::Error::new(std::io::ErrorKind::InvalidData, message).into(),
        )
    }
    let job_id: [u8; 16] = job_id
        .try_into()
        .map_err(|_| invalid(0, "scheduled job ID"))?;
    let kind_value = row.get::<_, i64>(0)?;
    let kind =
        ScheduledJobKind::from_sql(kind_value).map_err(|_| invalid(0, "scheduled job kind"))?;
    let unsigned = |index: usize, field| {
        u64::try_from(row.get::<_, i64>(index)?).map_err(|_| invalid(index, field))
    };
    let optional_unsigned = |index: usize, field| {
        row.get::<_, Option<i64>>(index)?
            .map(|value| u64::try_from(value).map_err(|_| invalid(index, field)))
            .transpose()
    };
    Ok(ScheduledJob {
        job_id,
        kind,
        host_id: row.get(1)?,
        scope_id: row.get(2)?,
        interval_micros: unsigned(3, "scheduled interval")?,
        next_run_at: unsigned(4, "scheduled next run")?,
        failure_count: unsigned(5, "scheduled failure count")?,
        lease_until: optional_unsigned(6, "scheduled lease")?,
        last_completed_at: optional_unsigned(7, "scheduled completion time")?,
        last_error: row.get(8)?,
        updated_at: unsigned(9, "scheduled update time")?,
    })
}

fn mutation_operation_from_row(
    operation_id: [u8; 16],
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<MutationOperation> {
    mutation_operation_from_offset(operation_id, row, 0)
}

fn mutation_operation_from_offset(
    operation_id: [u8; 16],
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<MutationOperation> {
    fn invalid(index: usize, message: &'static str) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            std::io::Error::new(std::io::ErrorKind::InvalidData, message).into(),
        )
    }
    let kind_value = row.get::<_, i64>(offset)?;
    let kind = MutationKind::from_sql(kind_value).map_err(|_| invalid(offset, "mutation kind"))?;
    let expected_value = row.get::<_, Option<i64>>(offset + 4)?;
    let expected_version = expected_value
        .map(|value| u64::try_from(value).map_err(|_| invalid(offset + 4, "expected version")))
        .transpose()?;
    let request_hash = row
        .get::<_, Vec<u8>>(offset + 5)?
        .try_into()
        .map_err(|_| invalid(offset + 5, "request hash"))?;
    let material_hash = row
        .get::<_, Vec<u8>>(offset + 7)?
        .try_into()
        .map_err(|_| invalid(offset + 7, "material hash"))?;
    let state_value = row.get::<_, i64>(offset + 8)?;
    let state =
        MutationState::from_sql(state_value).map_err(|_| invalid(offset + 8, "mutation state"))?;
    let attempt_count = u64::try_from(row.get::<_, i64>(offset + 9)?)
        .map_err(|_| invalid(offset + 9, "attempt count"))?;
    let created_at = u64::try_from(row.get::<_, i64>(offset + 10)?)
        .map_err(|_| invalid(offset + 10, "created time"))?;
    let updated_at = u64::try_from(row.get::<_, i64>(offset + 11)?)
        .map_err(|_| invalid(offset + 11, "updated time"))?;
    Ok(MutationOperation {
        operation_id,
        kind,
        host_id: row.get(offset + 1)?,
        scope_id: row.get(offset + 2)?,
        subject_id: row.get(offset + 3)?,
        expected_version,
        request_hash,
        material_ref: row.get(offset + 6)?,
        material_hash,
        state,
        attempt_count,
        created_at,
        updated_at,
    })
}

fn validate_adhoc_team_operation(operation: &AdHocTeamOperation) -> Result<()> {
    if operation.host_id.len() != 33
        || operation.uid.len() != 33
        || operation.device_id.len() != 33
        || operation.team_id.len() != 33
        || operation.created_at > operation.updated_at
    {
        return Err(Error::InvalidAdHocTeamOperation(
            "public operation fields are malformed",
        ));
    }
    Ok(())
}

fn validate_team_mutation(operation: &TeamMutationOperation) -> Result<()> {
    if operation.host_id.len() != 33
        || operation.actor_id.len() != 33
        || !matches!(operation.device_id.len(), 33 | 34)
        || operation.team_id.len() != 33
        || operation.expected_seqno == 0
        || operation.created_at > operation.updated_at
    {
        return Err(Error::InvalidTeamMutation(
            "public operation fields are malformed",
        ));
    }
    Ok(())
}

fn validate_user_snapshot(snapshot: VerifiedUserSnapshotParts<'_>) -> Result<()> {
    if snapshot.host_id.is_empty()
        || snapshot.uid.len() != 33
        || snapshot.chain_seqno == 0
        || snapshot.chain_bytes.is_empty()
        || snapshot.evidence_bytes.is_empty()
        || !(3..=25).contains(&snapshot.username.len())
        || snapshot.username_utf8.is_empty()
        || snapshot.username_sequence == 0
        || snapshot.merkle_root_bytes.is_empty()
        || snapshot.devices.is_empty()
        || snapshot.shared_keys.is_empty()
        || snapshot.devices.iter().any(|device| {
            !matches!(device.device_id.len(), 33 | 34)
                || !valid_stored_role(stored_role(device.role))
                || device.hepk_bytes.is_empty()
                || device
                    .subkey_id
                    .as_ref()
                    .is_some_and(|subkey| subkey.len() != 33)
        })
        || snapshot.shared_keys.iter().any(|key| {
            key.verify_key.len() != 33
                || !valid_stored_role(stored_role(key.role))
                || key.generation == 0
                || key.hepk_bytes.is_empty()
        })
    {
        return Err(Error::InvalidUser("required field is missing or malformed"));
    }
    Ok(())
}

fn validate_team_snapshot(snapshot: VerifiedTeamSnapshotParts<'_>) -> Result<()> {
    foks_verify::validate_rational_range(snapshot.index_range)
        .map_err(|_| Error::InvalidTeam("team index range is malformed"))?;
    for member in snapshot.members {
        if let Some(range) = &member.index_range {
            foks_verify::validate_rational_range(range)
                .map_err(|_| Error::InvalidTeam("member index range is malformed"))?;
        }
        let is_team = member.party_id.first().is_some_and(|kind| {
            matches!(
                *kind,
                foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
            )
        });
        if is_team != member.index_range.is_some() {
            return Err(Error::InvalidTeam(
                "member index range binding is malformed",
            ));
        }
    }
    if snapshot.host_id.is_empty()
        || snapshot.team_id.len() != 33
        || snapshot.chain_seqno == 0
        || snapshot.chain_bytes.is_empty()
        || snapshot.evidence_bytes.is_empty()
        || snapshot.team_name.is_empty()
        || snapshot.team_name_utf8.is_empty()
        || snapshot.merkle_root_bytes.is_empty()
        || snapshot.members.is_empty()
        || snapshot.shared_keys.is_empty()
        || snapshot.members.iter().any(|member| {
            member.party_id.len() != 33
                || member
                    .scoped_host_id
                    .as_ref()
                    .is_some_and(|host| host.len() != 33)
                || !valid_stored_role(stored_role(member.source_role))
                || !valid_stored_role(stored_role(member.role))
                || member.generation == 0
                || member.verify_key.len() != 33
        })
        || snapshot.shared_keys.iter().any(|key| {
            !valid_stored_role(stored_role(key.role))
                || key.generation == 0
                || key.verify_key.len() != 33
                || key.hepk_bytes.is_empty()
        })
    {
        return Err(Error::InvalidTeam("required field is missing or malformed"));
    }
    Ok(())
}

fn valid_stored_role(role: StoredRole) -> bool {
    match role.role_type {
        1 => (-32_768..32_768).contains(&role.visibility),
        2 | 3 => role.visibility == 0,
        _ => false,
    }
}

fn protocol_role(role: StoredRole) -> Result<foks_proto::Role> {
    match (role.role_type, role.visibility) {
        (1, visibility) => i16::try_from(visibility)
            .map(foks_proto::Role::member)
            .map_err(|_| Error::InvalidUser("stored member visibility is invalid")),
        (2, 0) => Ok(foks_proto::Role::ADMIN),
        (3, 0) => Ok(foks_proto::Role::OWNER),
        _ => Err(Error::InvalidUser("stored role is invalid")),
    }
}

fn load_user_snapshot(
    connection: &Connection,
    host_id: &[u8],
    uid: &[u8],
) -> Result<Option<StoredUserSnapshot>> {
    let row = connection
        .query_row(
            "SELECT chain_seqno, chain_tail_hash, chain_bytes, evidence_bytes, username, \
             username_utf8, username_sequence, merkle_epoch, merkle_root_hash, \
             merkle_root_bytes FROM users WHERE host_id = ?1 AND uid = ?2",
            params![host_id, uid],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                    row.get::<_, Vec<u8>>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        seqno,
        tail,
        chain_bytes,
        evidence_bytes,
        username,
        username_utf8,
        username_sequence,
        merkle_epoch,
        root_hash,
        root_bytes,
    )) = row
    else {
        return Ok(None);
    };
    let devices = load_user_devices(connection, host_id, uid)?
        .into_iter()
        .map(|device| {
            Ok(foks_verify::VerifiedUserDevice {
                device_id: device.device_id,
                role: protocol_role(device.role)?,
                hepk_bytes: device.hepk_bytes,
                subkey_id: device.subkey_id,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let shared_keys = load_user_shared_keys(connection, host_id, uid)?
        .into_iter()
        .map(|key| {
            Ok(foks_verify::VerifiedUserSharedKey {
                role: protocol_role(key.role)?,
                generation: key.generation,
                verify_key: key.verify_key,
                hepk_bytes: key.hepk_bytes,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(StoredUserSnapshot {
        host_id: host_id.to_vec(),
        uid: uid.to_vec(),
        chain_seqno: stored_unsigned("user chain sequence", seqno)?,
        chain_tail_hash: fixed_hash(tail, "stored user-chain tail has an invalid length")?,
        chain_bytes,
        evidence_bytes,
        username,
        username_utf8,
        username_sequence: stored_unsigned("username sequence", username_sequence)?,
        merkle_epoch: stored_unsigned("user Merkle epoch", merkle_epoch)?,
        merkle_root_hash: fixed_hash(root_hash, "stored user Merkle hash has an invalid length")?,
        merkle_root_bytes: root_bytes,
        devices,
        shared_keys,
    }))
}

fn load_user_devices(
    connection: &Connection,
    host_id: &[u8],
    uid: &[u8],
) -> Result<Vec<StoredUserDevice>> {
    let mut statement = connection.prepare(
        "SELECT device_id, role_type, role_visibility, hepk_bytes, subkey_id FROM user_devices \
         WHERE host_id = ?1 AND uid = ?2 \
         ORDER BY device_id, role_type, role_visibility, hepk_bytes",
    )?;
    let rows = statement.query_map(params![host_id, uid], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Option<Vec<u8>>>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (device_id, role_type, visibility, hepk_bytes, subkey_id) = row?;
        Ok(StoredUserDevice {
            device_id,
            role: StoredRole {
                role_type: stored_unsigned("device role type", role_type)?,
                visibility,
            },
            hepk_bytes,
            subkey_id,
        })
    })
    .collect()
}

fn load_user_shared_keys(
    connection: &Connection,
    host_id: &[u8],
    uid: &[u8],
) -> Result<Vec<StoredUserSharedKey>> {
    let mut statement = connection.prepare(
        "SELECT role_type, role_visibility, generation, verify_key, hepk_bytes \
         FROM user_shared_keys WHERE host_id = ?1 AND uid = ?2 \
         ORDER BY role_type, role_visibility, generation, verify_key, hepk_bytes",
    )?;
    let rows = statement.query_map(params![host_id, uid], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Vec<u8>>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (role_type, visibility, generation, verify_key, hepk_bytes) = row?;
        Ok(StoredUserSharedKey {
            role: StoredRole {
                role_type: stored_unsigned("shared-key role type", role_type)?,
                visibility,
            },
            generation: stored_unsigned("shared-key generation", generation)?,
            verify_key,
            hepk_bytes,
        })
    })
    .collect()
}

fn load_team_snapshot(
    connection: &Connection,
    host_id: &[u8],
    team_id: &[u8],
) -> Result<Option<StoredTeamSnapshot>> {
    let row = connection
        .query_row(
            "SELECT chain_seqno, chain_tail_hash, chain_bytes, evidence_bytes, team_name, \
             team_name_utf8, team_name_sequence, index_low_infinity, index_low_base, \
             index_low_exponent, index_high_infinity, index_high_base, index_high_exponent, \
             merkle_epoch, merkle_root_hash, merkle_root_bytes \
             FROM teams WHERE host_id = ?1 AND team_id = ?2",
            params![host_id, team_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, bool>(10)?,
                    row.get::<_, Vec<u8>>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, Vec<u8>>(14)?,
                    row.get::<_, Vec<u8>>(15)?,
                ))
            },
        )
        .optional()?;
    let Some((
        seqno,
        tail,
        chain_bytes,
        evidence_bytes,
        team_name,
        team_name_utf8,
        team_name_sequence,
        index_low_infinity,
        index_low_base,
        index_low_exponent,
        index_high_infinity,
        index_high_base,
        index_high_exponent,
        merkle_epoch,
        root_hash,
        root_bytes,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(StoredTeamSnapshot {
        host_id: host_id.to_vec(),
        team_id: team_id.to_vec(),
        chain_seqno: stored_unsigned("team chain sequence", seqno)?,
        chain_tail_hash: fixed_hash(tail, "stored team-chain tail has an invalid length")?,
        chain_bytes,
        evidence_bytes,
        team_name,
        team_name_utf8,
        team_name_sequence: stored_unsigned("team-name sequence", team_name_sequence)?,
        index_range: foks_proto::RationalRange {
            low: foks_proto::Rational {
                infinity: index_low_infinity,
                base: index_low_base,
                exponent: index_low_exponent,
            },
            high: foks_proto::Rational {
                infinity: index_high_infinity,
                base: index_high_base,
                exponent: index_high_exponent,
            },
        },
        merkle_epoch: stored_unsigned("team Merkle epoch", merkle_epoch)?,
        merkle_root_hash: fixed_hash(root_hash, "stored team Merkle hash has an invalid length")?,
        merkle_root_bytes: root_bytes,
        members: load_team_members(connection, host_id, team_id)?,
        shared_keys: load_team_shared_keys(connection, host_id, team_id)?,
    }))
}

fn load_team_members(
    connection: &Connection,
    host_id: &[u8],
    team_id: &[u8],
) -> Result<Vec<VerifiedTeamMember>> {
    let mut statement = connection.prepare(
        "SELECT party_id, scoped_host_id, source_role_type, source_role_visibility, role_type, \
         role_visibility, generation, verify_key, hepk_fingerprint, removal_key_commitment, \
         index_range_present, index_low_infinity, index_low_base, index_low_exponent, \
         index_high_infinity, index_high_base, index_high_exponent FROM team_members \
         WHERE host_id = ?1 AND team_id = ?2 ORDER BY party_id, scoped_host_id, \
         source_role_type, source_role_visibility",
    )?;
    let rows = statement.query_map(params![host_id, team_id], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, Vec<u8>>(7)?,
            row.get::<_, Vec<u8>>(8)?,
            row.get::<_, Vec<u8>>(9)?,
            row.get::<_, bool>(10)?,
            row.get::<_, bool>(11)?,
            row.get::<_, Vec<u8>>(12)?,
            row.get::<_, i64>(13)?,
            row.get::<_, bool>(14)?,
            row.get::<_, Vec<u8>>(15)?,
            row.get::<_, i64>(16)?,
        ))
    })?;
    rows.map(|row| {
        let (
            party,
            scope,
            source_type,
            source_visibility,
            role_type,
            role_visibility,
            generation,
            verify_key,
            fingerprint,
            removal_key_commitment,
            index_range_present,
            index_low_infinity,
            index_low_base,
            index_low_exponent,
            index_high_infinity,
            index_high_base,
            index_high_exponent,
        ) = row?;
        Ok(VerifiedTeamMember {
            party_id: party,
            scoped_host_id: (!scope.is_empty()).then_some(scope),
            source_role: protocol_role(StoredRole {
                role_type: stored_unsigned("team source role type", source_type)?,
                visibility: source_visibility,
            })?,
            role: protocol_role(StoredRole {
                role_type: stored_unsigned("team role type", role_type)?,
                visibility: role_visibility,
            })?,
            generation: stored_unsigned("team member generation", generation)?,
            verify_key,
            hepk_fingerprint: fixed_hash(
                fingerprint,
                "stored team member HEPK fingerprint has an invalid length",
            )?,
            removal_key_commitment: if removal_key_commitment.is_empty() {
                None
            } else {
                Some(fixed_hash(
                    removal_key_commitment,
                    "stored team removal-key commitment has an invalid length",
                )?)
            },
            index_range: index_range_present.then_some(foks_proto::RationalRange {
                low: foks_proto::Rational {
                    infinity: index_low_infinity,
                    base: index_low_base,
                    exponent: index_low_exponent,
                },
                high: foks_proto::Rational {
                    infinity: index_high_infinity,
                    base: index_high_base,
                    exponent: index_high_exponent,
                },
            }),
        })
    })
    .collect()
}

fn load_team_shared_keys(
    connection: &Connection,
    host_id: &[u8],
    team_id: &[u8],
) -> Result<Vec<foks_verify::VerifiedUserSharedKey>> {
    let mut statement = connection.prepare(
        "SELECT role_type, role_visibility, generation, verify_key, hepk_bytes \
         FROM team_shared_keys WHERE host_id = ?1 AND team_id = ?2 \
         ORDER BY role_type, role_visibility",
    )?;
    let rows = statement.query_map(params![host_id, team_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Vec<u8>>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (role_type, visibility, generation, verify_key, hepk_bytes) = row?;
        Ok(foks_verify::VerifiedUserSharedKey {
            role: protocol_role(StoredRole {
                role_type: stored_unsigned("PTK role type", role_type)?,
                visibility,
            })?,
            generation: stored_unsigned("PTK generation", generation)?,
            verify_key,
            hepk_bytes,
        })
    })
    .collect()
}

fn sqlite_integer(field: &'static str, value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::IntegerOutOfRange { field, value })
}

fn stored_unsigned(field: &'static str, value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::StoredIntegerOutOfRange { field, value })
}

fn stored_role(role: foks_proto::Role) -> StoredRole {
    StoredRole {
        role_type: role.protocol_value(),
        visibility: i64::from(role.visibility().unwrap_or(0)),
    }
}

fn stored_devices(devices: &[foks_verify::VerifiedUserDevice]) -> Vec<StoredUserDevice> {
    devices
        .iter()
        .map(|device| StoredUserDevice {
            device_id: device.device_id.clone(),
            role: stored_role(device.role),
            hepk_bytes: device.hepk_bytes.clone(),
            subkey_id: device.subkey_id.clone(),
        })
        .collect()
}

fn stored_shared_keys(keys: &[foks_verify::VerifiedUserSharedKey]) -> Vec<StoredUserSharedKey> {
    keys.iter()
        .map(|key| StoredUserSharedKey {
            role: stored_role(key.role),
            generation: key.generation,
            verify_key: key.verify_key.clone(),
            hepk_bytes: key.hepk_bytes.clone(),
        })
        .collect()
}

fn load_host_row(connection: &Connection, host_id: &[u8]) -> Result<Option<StoredHost>> {
    connection
        .query_row(
            "SELECT canonical_name, genesis_key, chain_seqno, chain_tail_hash, chain_bytes, \
             public_zone_bytes \
             FROM hosts WHERE host_id = ?1",
            [host_id],
            |row| {
                let seqno = row.get::<_, i64>(2)?;
                let tail = row.get::<_, Vec<u8>>(3)?;
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    seqno,
                    tail,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?
        .map(
            |(canonical_name, genesis_key, chain_seqno, tail, chain_bytes, public_zone_bytes)| {
                Ok(StoredHost {
                    canonical_name,
                    genesis_key,
                    chain_seqno: stored_unsigned("host-chain sequence", chain_seqno)?,
                    chain_tail_hash: fixed_hash(
                        tail,
                        "stored host-chain tail has an invalid length",
                    )?,
                    chain_bytes,
                    public_zone_bytes,
                })
            },
        )
        .transpose()
}

fn normalized_services(services: &[HostService]) -> Vec<HostService> {
    let mut services = services.to_vec();
    services.sort_unstable_by_key(|service| service.service_type);
    services
}

fn load_services(connection: &Connection, host_id: &[u8]) -> Result<Vec<HostService>> {
    let mut statement = connection.prepare(
        "SELECT service_type, endpoint_bytes FROM host_services \
         WHERE host_id = ?1 ORDER BY service_type",
    )?;
    let rows = statement.query_map([host_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    rows.map(|row| {
        let (service_type, endpoint_bytes) = row?;
        Ok(HostService {
            service_type: ServiceType::try_from(stored_unsigned("service type", service_type)?)
                .map_err(|_| Error::InvalidSnapshot("stored service type is unknown"))?,
            endpoint_bytes,
        })
    })
    .collect()
}

fn accept_merkle_root(
    connection: &Connection,
    host_id: &[u8],
    root: VerifiedMerkleRootParts<'_>,
    epoch: i64,
) -> Result<bool> {
    validate_merkle_root(root)?;
    let stored = connection
        .query_row(
            "SELECT epoch, root_hash, evidence_kind, anchor_epoch, evidence_bytes
             FROM merkle_heads WHERE host_id = ?1",
            [host_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .optional()?;
    if let Some((stored_epoch, stored_hash, _, _, _)) = &stored {
        let stored_epoch = stored_unsigned("Merkle epoch", *stored_epoch)?;
        match root.epoch.cmp(&stored_epoch) {
            std::cmp::Ordering::Less => {
                return Err(Error::MerkleRollback {
                    stored: stored_epoch,
                    received: root.epoch,
                });
            }
            std::cmp::Ordering::Equal => {
                let stored_root_bytes = connection.query_row(
                    "SELECT r.root_bytes \
                     FROM merkle_heads h JOIN merkle_roots r \
                     ON r.host_id = h.host_id AND r.epoch = h.epoch \
                     WHERE h.host_id = ?1 AND h.epoch = ?2",
                    params![host_id, epoch],
                    |row| row.get::<_, Option<Vec<u8>>>(0),
                )?;
                if stored_hash.as_slice() != root.root_hash
                    || stored_root_bytes.as_deref() != Some(root.root_bytes)
                {
                    return Err(Error::MerkleFork { epoch: root.epoch });
                }
                accept_authenticated_roots(connection, host_id, root.authenticated_roots)?;
                return Ok(false);
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    accept_authenticated_roots(connection, host_id, root.authenticated_roots)?;
    let refreshed_evidence;
    let evidence = if let Some((stored_epoch, _, kind, anchor, bytes)) = &stored {
        let stored_epoch = stored_unsigned("Merkle epoch", *stored_epoch)?;
        if root.epoch > stored_epoch
            && matches!(root.evidence, MerkleRootEvidence::SignedBootstrap(_))
        {
            refreshed_evidence = MerkleRootEvidence::SignedRefresh {
                signed_root: match root.evidence {
                    MerkleRootEvidence::SignedBootstrap(bytes) => bytes.clone(),
                    _ => unreachable!(),
                },
                prior_epoch: stored_epoch,
                prior: Box::new(decode_evidence(*kind, *anchor, bytes.clone())?),
            };
            &refreshed_evidence
        } else {
            root.evidence
        }
    } else {
        root.evidence
    };
    let (evidence_kind, anchor_epoch, evidence_bytes) = encode_evidence(evidence)?;
    connection.execute(
        "INSERT INTO merkle_heads \
         (host_id, epoch, root_hash, evidence_kind, anchor_epoch, evidence_bytes) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(host_id) DO UPDATE SET \
         epoch = excluded.epoch, root_hash = excluded.root_hash, \
         evidence_kind = excluded.evidence_kind, anchor_epoch = excluded.anchor_epoch, \
         evidence_bytes = excluded.evidence_bytes",
        params![
            host_id,
            epoch,
            root.root_hash.as_slice(),
            evidence_kind,
            anchor_epoch,
            evidence_bytes,
        ],
    )?;
    Ok(true)
}

fn load_snapshot(
    connection: &Connection,
    lookup_name: &str,
    host_id: &[u8],
) -> Result<StoredHostSnapshot> {
    let host = load_host_row(connection, host_id)?
        .ok_or(Error::InvalidSnapshot("lookup points to a missing host"))?;
    let merkle = connection.query_row(
        "SELECT h.epoch, h.root_hash, r.root_bytes, h.evidence_kind, \
         h.anchor_epoch, h.evidence_bytes \
         FROM merkle_heads h JOIN merkle_roots r \
         ON r.host_id = h.host_id AND r.epoch = h.epoch WHERE h.host_id = ?1",
        [host_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        },
    )?;
    Ok(StoredHostSnapshot {
        lookup_name: lookup_name.to_owned(),
        host_id: host_id.to_vec(),
        canonical_name: host.canonical_name,
        genesis_key: host.genesis_key,
        chain_seqno: host.chain_seqno,
        chain_tail_hash: host.chain_tail_hash,
        chain_bytes: host.chain_bytes,
        public_zone_bytes: host.public_zone_bytes,
        services: load_services(connection, host_id)?,
        merkle_root: StoredMerkleRoot {
            epoch: stored_unsigned("Merkle epoch", merkle.0)?,
            root_hash: fixed_hash(merkle.1, "stored Merkle root has an invalid length")?,
            root_bytes: merkle.2.ok_or(Error::InvalidSnapshot(
                "stored Merkle head has no root bytes",
            ))?,
            evidence: decode_evidence(merkle.3, merkle.4, merkle.5)?,
            authenticated_roots: load_authenticated_roots(connection, host_id)?,
        },
    })
}

fn validate_merkle_root(root: VerifiedMerkleRootParts<'_>) -> Result<()> {
    if root.root_bytes.is_empty() || root.authenticated_roots.is_empty() {
        return Err(Error::InvalidSnapshot(
            "Merkle root must carry bytes and authenticated history",
        ));
    }
    let mut roots = root.authenticated_roots.to_vec();
    roots.sort_unstable_by_key(|entry| entry.epoch);
    if roots.windows(2).any(|pair| pair[0].epoch == pair[1].epoch) {
        return Err(Error::InvalidSnapshot(
            "authenticated Merkle epochs must be unique",
        ));
    }
    let Some(head) = roots.iter().find(|entry| entry.epoch == root.epoch) else {
        return Err(Error::InvalidSnapshot(
            "authenticated Merkle history must contain its head",
        ));
    };
    if head.root_hash != root.root_hash || head.root_bytes.as_deref() != Some(root.root_bytes) {
        return Err(Error::InvalidSnapshot(
            "authenticated Merkle head does not match the snapshot",
        ));
    }
    match &root.evidence {
        MerkleRootEvidence::SignedBootstrap(bytes) if bytes.is_empty() => {
            Err(Error::InvalidSnapshot("signed Merkle evidence is empty"))
        }
        MerkleRootEvidence::SignedRefresh { signed_root, .. } if signed_root.is_empty() => {
            Err(Error::InvalidSnapshot("signed Merkle evidence is empty"))
        }
        MerkleRootEvidence::SignedRefresh {
            prior_epoch, prior, ..
        } if *prior_epoch < root.epoch => validate_evidence_order(prior, *prior_epoch),
        MerkleRootEvidence::SignedRefresh { .. } => Err(Error::InvalidSnapshot(
            "Merkle evidence anchors must strictly descend",
        )),
        MerkleRootEvidence::SkipPath { signed_root, .. } if signed_root.is_empty() => {
            Err(Error::InvalidSnapshot("signed Merkle evidence is empty"))
        }
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            ..
        } if *anchor_epoch >= root.epoch || historical_response.is_empty() => Err(
            Error::InvalidSnapshot("Merkle skip-path evidence is malformed"),
        ),
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            prior,
            ..
        } => validate_evidence_order(prior, *anchor_epoch),
        _ => Ok(()),
    }
}

fn accept_authenticated_roots(
    connection: &Connection,
    host_id: &[u8],
    roots: &[AuthenticatedMerkleRoot],
) -> Result<()> {
    for root in roots {
        let epoch = sqlite_integer("authenticated Merkle epoch", root.epoch)?;
        let existing = connection
            .query_row(
                "SELECT root_hash, root_bytes FROM merkle_roots \
                 WHERE host_id = ?1 AND epoch = ?2",
                params![host_id, epoch],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
            )
            .optional()?;
        if let Some((hash, bytes)) = existing {
            if hash.as_slice() != root.root_hash
                || bytes
                    .as_ref()
                    .zip(root.root_bytes.as_ref())
                    .is_some_and(|(stored, received)| stored != received)
            {
                return Err(Error::MerkleFork { epoch: root.epoch });
            }
            if bytes.is_none() && root.root_bytes.is_some() {
                connection.execute(
                    "UPDATE merkle_roots SET root_bytes = ?3 \
                     WHERE host_id = ?1 AND epoch = ?2",
                    params![host_id, epoch, root.root_bytes],
                )?;
            }
        } else {
            connection.execute(
                "INSERT INTO merkle_roots (host_id, epoch, root_hash, root_bytes) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![host_id, epoch, root.root_hash.as_slice(), root.root_bytes],
            )?;
        }
    }
    Ok(())
}

fn load_authenticated_roots(
    connection: &Connection,
    host_id: &[u8],
) -> Result<Vec<AuthenticatedMerkleRoot>> {
    let mut statement = connection.prepare(
        "SELECT epoch, root_hash, root_bytes FROM merkle_roots \
         WHERE host_id = ?1 ORDER BY epoch",
    )?;
    let rows = statement.query_map([host_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Option<Vec<u8>>>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (epoch, hash, root_bytes) = row?;
        Ok(AuthenticatedMerkleRoot {
            epoch: stored_unsigned("Merkle epoch", epoch)?,
            root_hash: fixed_hash(hash, "stored Merkle root has an invalid length")?,
            root_bytes,
        })
    })
    .collect()
}

fn validate_evidence_order(evidence: &MerkleRootEvidence, upper: u64) -> Result<()> {
    match evidence {
        MerkleRootEvidence::SignedBootstrap(bytes) if bytes.is_empty() => {
            Err(Error::InvalidSnapshot("signed Merkle evidence is empty"))
        }
        MerkleRootEvidence::SignedBootstrap(_) => Ok(()),
        MerkleRootEvidence::SignedRefresh {
            signed_root,
            prior_epoch,
            prior,
        } if !signed_root.is_empty() && *prior_epoch < upper => {
            validate_evidence_order(prior, *prior_epoch)
        }
        MerkleRootEvidence::SignedRefresh { .. } => Err(Error::InvalidSnapshot(
            "Merkle evidence anchors must strictly descend",
        )),
        MerkleRootEvidence::SkipPath { signed_root, .. } if signed_root.is_empty() => {
            Err(Error::InvalidSnapshot("signed Merkle evidence is empty"))
        }
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            prior,
            ..
        } if *anchor_epoch < upper && !historical_response.is_empty() => {
            validate_evidence_order(prior, *anchor_epoch)
        }
        MerkleRootEvidence::SkipPath { .. } => Err(Error::InvalidSnapshot(
            "Merkle evidence anchors must strictly descend",
        )),
    }
}

fn encode_evidence(evidence: &MerkleRootEvidence) -> Result<(i64, Option<i64>, Vec<u8>)> {
    let (kind, anchor) = match evidence {
        MerkleRootEvidence::SignedBootstrap(_) => (1, None),
        MerkleRootEvidence::SignedRefresh { prior_epoch, .. } => {
            (3, Some(sqlite_integer("Merkle prior epoch", *prior_epoch)?))
        }
        MerkleRootEvidence::SkipPath { anchor_epoch, .. } => (
            2,
            Some(sqlite_integer("Merkle anchor epoch", *anchor_epoch)?),
        ),
    };
    Ok((kind, anchor, encode(&evidence_value(evidence))?))
}

fn evidence_value(evidence: &MerkleRootEvidence) -> Value {
    match evidence {
        MerkleRootEvidence::SignedBootstrap(bytes) => {
            Value::Array(vec![Value::Unsigned(0), Value::Binary(bytes.clone())])
        }
        MerkleRootEvidence::SignedRefresh {
            signed_root,
            prior_epoch,
            prior,
        } => Value::Array(vec![
            Value::Unsigned(2),
            Value::Binary(signed_root.clone()),
            Value::Unsigned(*prior_epoch),
            evidence_value(prior),
        ]),
        MerkleRootEvidence::SkipPath {
            signed_root,
            anchor_epoch,
            historical_response,
            prior,
        } => Value::Array(vec![
            Value::Unsigned(1),
            Value::Unsigned(*anchor_epoch),
            Value::Binary(historical_response.clone()),
            evidence_value(prior),
            Value::Binary(signed_root.clone()),
        ]),
    }
}

fn decode_evidence(
    kind: i64,
    anchor_epoch: Option<i64>,
    bytes: Vec<u8>,
) -> Result<MerkleRootEvidence> {
    let evidence = evidence_from_value(&decode(&bytes)?, 0)?;
    match (&evidence, kind, anchor_epoch) {
        (MerkleRootEvidence::SignedBootstrap(_), 1, None) => Ok(evidence),
        (MerkleRootEvidence::SignedRefresh { prior_epoch, .. }, 3, Some(stored_prior))
            if *prior_epoch == stored_unsigned("Merkle prior epoch", stored_prior)? =>
        {
            Ok(evidence)
        }
        (MerkleRootEvidence::SkipPath { anchor_epoch, .. }, 2, Some(stored_anchor))
            if *anchor_epoch == stored_unsigned("Merkle anchor epoch", stored_anchor)? =>
        {
            Ok(evidence)
        }
        _ => Err(Error::InvalidSnapshot(
            "stored Merkle evidence has an invalid shape",
        )),
    }
}

fn evidence_from_value(value: &Value, depth: usize) -> Result<MerkleRootEvidence> {
    if depth > 4096 {
        return Err(Error::InvalidSnapshot(
            "persisted Merkle evidence is too deep",
        ));
    }
    let Value::Array(fields) = value else {
        return Err(Error::InvalidSnapshot(
            "persisted Merkle evidence is not an array",
        ));
    };
    match fields.as_slice() {
        [Value::Unsigned(0), Value::Binary(bytes)] => {
            Ok(MerkleRootEvidence::SignedBootstrap(bytes.clone()))
        }
        [Value::Unsigned(2), Value::Binary(signed_root), Value::Unsigned(prior_epoch), prior] => {
            Ok(MerkleRootEvidence::SignedRefresh {
                signed_root: signed_root.clone(),
                prior_epoch: *prior_epoch,
                prior: Box::new(evidence_from_value(prior, depth + 1)?),
            })
        }
        [Value::Unsigned(1), Value::Unsigned(anchor_epoch), Value::Binary(historical_response), prior, Value::Binary(signed_root)]
            if !signed_root.is_empty() =>
        {
            Ok(MerkleRootEvidence::SkipPath {
                signed_root: signed_root.clone(),
                anchor_epoch: *anchor_epoch,
                historical_response: historical_response.clone(),
                prior: Box::new(evidence_from_value(prior, depth + 1)?),
            })
        }
        _ => Err(Error::InvalidSnapshot(
            "persisted Merkle evidence has an invalid shape",
        )),
    }
}

fn fixed_hash(bytes: Vec<u8>, error: &'static str) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| Error::InvalidSnapshot(error))
}

fn encoded_array_is_prefix(stored: &[u8], received: &[u8]) -> Result<bool> {
    let Value::Array(stored) = decode(stored)? else {
        return Err(Error::InvalidSnapshot(
            "stored chain is not an encoded array",
        ));
    };
    let Value::Array(received) = decode(received)? else {
        return Err(Error::InvalidSnapshot(
            "received chain is not an encoded array",
        ));
    };
    Ok(stored.len() <= received.len() && stored.iter().zip(&received).all(|(a, b)| a == b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_schema_error_describes_failed_auto_update() {
        let error = Error::UnsupportedSchema {
            found: 23,
            supported: 27,
        };

        assert_eq!(
            error.to_string(),
            "system store is out of date (v23), could not auto-update to current version (v27)"
        );
    }

    fn chain(values: &[u64]) -> Vec<u8> {
        encode(&Value::Array(
            values.iter().copied().map(Value::Unsigned).collect(),
        ))
        .unwrap()
    }

    #[derive(Clone)]
    struct TestMerkleRoot {
        epoch: u64,
        root_hash: [u8; 32],
        root_bytes: Vec<u8>,
        evidence: MerkleRootEvidence,
        authenticated_roots: Vec<AuthenticatedMerkleRoot>,
    }

    impl TestMerkleRoot {
        fn parts(&self) -> VerifiedMerkleRootParts<'_> {
            VerifiedMerkleRootParts {
                epoch: self.epoch,
                root_hash: self.root_hash,
                root_bytes: &self.root_bytes,
                evidence: &self.evidence,
                authenticated_roots: &self.authenticated_roots,
            }
        }
    }

    #[derive(Clone)]
    struct TestHostSnapshot {
        lookup_name: String,
        host_id: Vec<u8>,
        canonical_name: String,
        genesis_key: Vec<u8>,
        chain_seqno: u64,
        chain_tail_hash: [u8; 32],
        chain_bytes: Vec<u8>,
        public_zone_bytes: Vec<u8>,
        services: Vec<HostService>,
        merkle_root: TestMerkleRoot,
    }

    #[derive(Clone)]
    struct TestUserSnapshot {
        host_id: Vec<u8>,
        uid: Vec<u8>,
        chain_seqno: u64,
        chain_tail_hash: [u8; 32],
        chain_bytes: Vec<u8>,
        evidence_bytes: Vec<u8>,
        username: Vec<u8>,
        username_utf8: Vec<u8>,
        username_sequence: u64,
        merkle_epoch: u64,
        merkle_root_hash: [u8; 32],
        merkle_root_bytes: Vec<u8>,
        devices: Vec<foks_verify::VerifiedUserDevice>,
        shared_keys: Vec<foks_verify::VerifiedUserSharedKey>,
    }

    impl TestUserSnapshot {
        fn parts(&self) -> VerifiedUserSnapshotParts<'_> {
            VerifiedUserSnapshotParts {
                host_id: &self.host_id,
                uid: &self.uid,
                chain_seqno: self.chain_seqno,
                chain_tail_hash: self.chain_tail_hash,
                chain_bytes: &self.chain_bytes,
                evidence_bytes: &self.evidence_bytes,
                username: &self.username,
                username_utf8: &self.username_utf8,
                username_sequence: self.username_sequence,
                merkle_epoch: self.merkle_epoch,
                merkle_root_hash: self.merkle_root_hash,
                merkle_root_bytes: &self.merkle_root_bytes,
                devices: &self.devices,
                shared_keys: &self.shared_keys,
            }
        }
    }

    #[derive(Clone)]
    struct TestTeamSnapshot {
        host_id: Vec<u8>,
        team_id: Vec<u8>,
        chain_seqno: u64,
        chain_tail_hash: [u8; 32],
        chain_bytes: Vec<u8>,
        evidence_bytes: Vec<u8>,
        team_name: Vec<u8>,
        team_name_utf8: Vec<u8>,
        team_name_sequence: u64,
        index_range: foks_proto::RationalRange,
        merkle_epoch: u64,
        merkle_root_hash: [u8; 32],
        merkle_root_bytes: Vec<u8>,
        members: Vec<VerifiedTeamMember>,
        shared_keys: Vec<foks_verify::VerifiedUserSharedKey>,
    }

    impl TestTeamSnapshot {
        fn parts(&self) -> VerifiedTeamSnapshotParts<'_> {
            VerifiedTeamSnapshotParts {
                host_id: &self.host_id,
                team_id: &self.team_id,
                chain_seqno: self.chain_seqno,
                chain_tail_hash: self.chain_tail_hash,
                chain_bytes: &self.chain_bytes,
                evidence_bytes: &self.evidence_bytes,
                team_name: &self.team_name,
                team_name_utf8: &self.team_name_utf8,
                team_name_sequence: self.team_name_sequence,
                index_range: &self.index_range,
                merkle_epoch: self.merkle_epoch,
                merkle_root_hash: self.merkle_root_hash,
                merkle_root_bytes: &self.merkle_root_bytes,
                members: &self.members,
                shared_keys: &self.shared_keys,
            }
        }
    }

    impl TestHostSnapshot {
        fn parts(&self) -> VerifiedHostSnapshotParts<'_> {
            VerifiedHostSnapshotParts {
                lookup_name: &self.lookup_name,
                host_id: &self.host_id,
                canonical_name: &self.canonical_name,
                genesis_key: &self.genesis_key,
                chain_seqno: self.chain_seqno,
                chain_tail_hash: self.chain_tail_hash,
                chain_bytes: &self.chain_bytes,
                public_zone_bytes: &self.public_zone_bytes,
                services: &self.services,
                merkle_root: self.merkle_root.parts(),
            }
        }
    }

    #[test]
    fn stored_unsigned_values_reject_negative_rows() {
        assert!(matches!(
            stored_unsigned("test", -1),
            Err(Error::StoredIntegerOutOfRange {
                field: "test",
                value: -1
            })
        ));
    }

    #[test]
    fn database_identity_and_revision_cover_jobs_and_mutation_journals() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.db");
        let mut store = HardStateStore::open(&path).unwrap();
        let host = snapshot();
        let initial = store.metadata().unwrap();
        store.accept_host_parts(host.parts()).unwrap();
        let pinned = store.metadata().unwrap();
        assert_eq!(initial.database_id, pinned.database_id);
        assert!(pinned.revision > initial.revision);
        assert_ne!(pinned.write_token, initial.write_token);

        store
            .register_scheduled_job(&ScheduledJob {
                job_id: [5; 16],
                kind: ScheduledJobKind::UserRefresh,
                host_id: host.host_id.clone(),
                scope_id: vec![7; 33],
                interval_micros: 1_000,
                next_run_at: 100,
                failure_count: 0,
                lease_until: None,
                last_completed_at: None,
                last_error: None,
                updated_at: 90,
            })
            .unwrap();
        let scheduled = store.metadata().unwrap();
        assert!(scheduled.revision > pinned.revision);
        assert_ne!(scheduled.write_token, pinned.write_token);

        store
            .record_mutation(&MutationOperation {
                operation_id: [6; 16],
                kind: MutationKind::DeviceProvision,
                host_id: host.host_id,
                scope_id: vec![1; 33],
                subject_id: vec![2; 33],
                expected_version: Some(2),
                request_hash: [3; 32],
                material_ref: b"credential/device/6".to_vec(),
                material_hash: [4; 32],
                state: MutationState::Prepared,
                attempt_count: 0,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();
        let journaled = store.metadata().unwrap();
        assert!(journaled.revision > scheduled.revision);
        assert_ne!(journaled.write_token, scheduled.write_token);

        let other = HardStateStore::open(&directory.path().join("other.db"))
            .unwrap()
            .metadata()
            .unwrap();
        assert_ne!(journaled.database_id, other.database_id);
    }

    #[test]
    fn every_hard_state_table_has_revision_triggers() {
        use std::collections::BTreeSet;

        let directory = tempfile::tempdir().unwrap();
        let store = HardStateStore::open(&directory.path().join("hard.db")).unwrap();
        let tables = {
            let mut statement = store
                .connection
                .prepare(
                    "SELECT name FROM sqlite_schema
                     WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                     ORDER BY name",
                )
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<BTreeSet<_>, _>>()
                .unwrap()
        };
        let mut expected = schema::REVISION_TABLES
            .iter()
            .map(|table| (*table).to_owned())
            .collect::<BTreeSet<_>>();
        expected.insert("hard_state_metadata".to_owned());
        assert_eq!(tables, expected);

        for table in schema::REVISION_TABLES {
            let count: i64 = store
                .connection
                .query_row(
                    "SELECT count(*) FROM sqlite_schema
                     WHERE type = 'trigger' AND tbl_name = ?1
                       AND name LIKE 'hard_state_revision_%'",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 3, "revision trigger coverage changed for {table}");
        }
    }

    #[test]
    fn rename_capacity_and_cleanup_preserve_unknown_receipts() {
        let (_directory, mut store) = store();
        let host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let mut op = MutationOperation {
            operation_id: [1; 16],
            kind: MutationKind::UsernameChange,
            host_id: host.host_id.clone(),
            scope_id: vec![1; 33],
            subject_id: vec![2; 33],
            expected_version: Some(1),
            request_hash: [3; 32],
            material_ref: vec![4],
            material_hash: [5; 32],
            state: MutationState::Prepared,
            attempt_count: 0,
            created_at: 100,
            updated_at: 100,
        };
        for i in 1..=32 {
            op.operation_id = [i; 16];
            op.expected_version = Some(u64::from(i));
            op.material_ref = vec![i];
            store.record_mutation(&op).unwrap();
            let mut concurrent = op.clone();
            concurrent.operation_id = [i + 64; 16];
            concurrent.expected_version = Some(1000);
            concurrent.material_ref = vec![i + 64];
            assert!(store.record_mutation(&concurrent).is_err());
            store
                .begin_mutation_submission(&op.operation_id, 101)
                .unwrap();
            store
                .advance_mutation(&op.operation_id, MutationState::SubmissionUnknown, 102)
                .unwrap();
        }
        op.operation_id = [33; 16];
        op.expected_version = Some(33);
        op.material_ref = vec![33];
        assert!(store.record_mutation(&op).is_err());
        assert!(store
            .expired_username_change_receipts(&host.host_id, &op.scope_id, 1000)
            .unwrap()
            .is_empty());
        store.delete_username_change_receipt(&[1; 16]).unwrap();
        assert!(store.mutation(&[1; 16]).unwrap().is_some());
        store
            .advance_mutation(&[1; 16], MutationState::Rejected, 103)
            .unwrap();
        let receipts = store
            .expired_username_change_receipts(&host.host_id, &op.scope_id, 1000)
            .unwrap();
        assert_eq!(receipts.len(), 1);
        store
            .delete_username_change_receipt(&receipts[0].operation_id)
            .unwrap();
        store.record_mutation(&op).unwrap();
        assert_eq!(
            store
                .username_changes(&host.host_id, &op.scope_id, &op.subject_id)
                .unwrap()
                .len(),
            32
        );
    }

    #[test]
    fn generic_mutation_wal_never_replays_an_ambiguous_submission() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.db");
        let mut store = HardStateStore::open(&path).unwrap();
        let host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let operation = MutationOperation {
            operation_id: [6; 16],
            kind: MutationKind::DeviceProvision,
            host_id: host.host_id,
            scope_id: vec![1; 33],
            subject_id: vec![2; 33],
            expected_version: Some(2),
            request_hash: [3; 32],
            material_ref: b"credential/device/6".to_vec(),
            material_hash: [4; 32],
            state: MutationState::Prepared,
            attempt_count: 0,
            created_at: 100,
            updated_at: 100,
        };
        store.record_mutation(&operation).unwrap();
        store.begin_mutation_submission(&[6; 16], 101).unwrap();
        assert!(store.begin_mutation_submission(&[6; 16], 102).is_err());

        drop(store);
        let mut reopened = HardStateStore::open(&path).unwrap();
        assert_eq!(
            reopened.pending_mutations(&operation.host_id).unwrap()[0].state,
            MutationState::Submitting
        );
        reopened
            .advance_mutation(&[6; 16], MutationState::SubmissionUnknown, 102)
            .unwrap();
        assert!(reopened.begin_mutation_submission(&[6; 16], 103).is_err());
        reopened
            .advance_mutation(&[6; 16], MutationState::RemoteVerified, 104)
            .unwrap();
        assert_eq!(
            reopened.pending_mutations(&operation.host_id).unwrap()[0].state,
            MutationState::RemoteVerified
        );
        reopened
            .advance_mutation(&[6; 16], MutationState::Finalized, 105)
            .unwrap();
        assert!(reopened
            .pending_mutations(&operation.host_id)
            .unwrap()
            .is_empty());
        assert_eq!(
            reopened.mutation(&[6; 16]).unwrap().unwrap().attempt_count,
            1
        );
    }

    #[test]
    fn adapter_child_binding_is_atomic_and_rejects_wrong_scope_and_terminal_parent() {
        let (_directory, mut store) = store();
        let host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let parent = MutationOperation {
            operation_id: [41; 16],
            kind: MutationKind::KvAdapter,
            host_id: host.host_id,
            scope_id: vec![1; 33],
            subject_id: vec![2; 33],
            expected_version: None,
            request_hash: [3; 32],
            material_ref: vec![41; 16],
            material_hash: [4; 32],
            state: MutationState::Prepared,
            attempt_count: 0,
            created_at: 100,
            updated_at: 100,
        };
        store
            .record_adapter_submission(
                SubmissionHandle::new(200_000, [41; 16]),
                &parent,
                adapter_time(200_000, 0),
            )
            .unwrap();
        let mut child = parent.clone();
        child.operation_id = [42; 16];
        child.kind = MutationKind::KvNamespace;
        child.scope_id = parent.subject_id.clone();
        child.subject_id = vec![5; 16];
        assert!(store
            .record_child_mutation(&child, &parent.operation_id, true)
            .is_err());
        assert!(store.mutation(&child.operation_id).unwrap().is_none());
        store
            .begin_mutation_submission(&parent.operation_id, 101)
            .unwrap();
        child.scope_id = vec![7; 33];
        assert!(store
            .record_child_mutation(&child, &parent.operation_id, true)
            .is_err());
        assert!(store.mutation(&child.operation_id).unwrap().is_none());
        child.scope_id = parent.subject_id.clone();
        store
            .record_child_mutation(&child, &parent.operation_id, true)
            .unwrap();
        assert_eq!(
            store.mutation_children(&parent.operation_id).unwrap().len(),
            1
        );
        assert!(store
            .record_child_mutation(&child, &parent.operation_id, false)
            .is_err());
        assert_eq!(
            store.mutation_children(&parent.operation_id).unwrap().len(),
            1
        );
        store
            .advance_mutation(&parent.operation_id, MutationState::Rejected, 102)
            .unwrap();
        child.operation_id = [43; 16];
        assert!(store
            .record_child_mutation(&child, &parent.operation_id, true)
            .is_err());
        assert!(store.mutation(&child.operation_id).unwrap().is_none());
    }

    #[test]
    fn user_chain_mutations_reserve_one_live_chain_position() {
        let (_directory, mut store) = store();
        let host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let first = MutationOperation {
            operation_id: [6; 16],
            kind: MutationKind::PukRotation,
            host_id: host.host_id,
            scope_id: vec![1; 33],
            subject_id: vec![2; 33],
            expected_version: Some(2),
            request_hash: [3; 32],
            material_ref: b"credential/puk/first".to_vec(),
            material_hash: [4; 32],
            state: MutationState::Prepared,
            attempt_count: 0,
            created_at: 100,
            updated_at: 100,
        };
        let second = MutationOperation {
            operation_id: [7; 16],
            request_hash: [8; 32],
            material_ref: b"credential/puk/second".to_vec(),
            material_hash: [9; 32],
            ..first.clone()
        };
        store.record_mutation(&first).unwrap();
        assert!(store.record_mutation(&second).is_err());
        store
            .advance_mutation(&first.operation_id, MutationState::Rejected, 101)
            .unwrap();
        store.record_mutation(&second).unwrap();
    }

    #[test]
    fn application_binding_lookups_separate_pending_resume_from_finalizable_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = HardStateStore::open(&directory.path().join("hard.db")).unwrap();
        let host = snapshot();
        let host_id = host.host_id.clone();
        store.accept_host_parts(host.parts()).unwrap();
        let scope_id = vec![2; 33];
        let subject_id = vec![3; 33];

        for (operation_id, kind, host, scope, subject, created_at) in [
            (
                [1; 16],
                MutationKind::DeviceProvision,
                host_id.clone(),
                scope_id.clone(),
                subject_id.clone(),
                100,
            ),
            (
                [2; 16],
                MutationKind::DeviceProvision,
                host_id.clone(),
                scope_id.clone(),
                subject_id.clone(),
                200,
            ),
            (
                [3; 16],
                MutationKind::Signup,
                host_id.clone(),
                scope_id.clone(),
                subject_id.clone(),
                300,
            ),
        ] {
            store
                .record_mutation(&MutationOperation {
                    operation_id,
                    kind,
                    host_id: host,
                    scope_id: scope,
                    subject_id: subject,
                    expected_version: None,
                    request_hash: [5; 32],
                    material_ref: operation_id.to_vec(),
                    material_hash: [6; 32],
                    state: MutationState::Prepared,
                    attempt_count: 0,
                    created_at,
                    updated_at: created_at,
                })
                .unwrap();
        }
        store.begin_mutation_submission(&[2; 16], 201).unwrap();
        store
            .advance_mutation(&[2; 16], MutationState::RemoteVerified, 202)
            .unwrap();
        store
            .record_mutation(&MutationOperation {
                operation_id: [4; 16],
                kind: MutationKind::DeviceProvision,
                host_id: host_id.clone(),
                scope_id: scope_id.clone(),
                subject_id: subject_id.clone(),
                expected_version: None,
                request_hash: [7; 32],
                material_ref: vec![4; 16],
                material_hash: [8; 32],
                state: MutationState::Prepared,
                attempt_count: 0,
                created_at: 300,
                updated_at: 300,
            })
            .unwrap();

        let loaded = store
            .latest_mutation_for_binding(
                &host_id,
                MutationKind::DeviceProvision,
                &scope_id,
                &subject_id,
            )
            .unwrap()
            .unwrap();
        assert_eq!(loaded.operation_id, [4; 16]);
        assert_eq!(loaded.state, MutationState::Prepared);
        let finalizable = store
            .latest_finalizable_mutation_for_binding(
                &host_id,
                MutationKind::DeviceProvision,
                &scope_id,
                &subject_id,
            )
            .unwrap()
            .unwrap();
        assert_eq!(finalizable.operation_id, [2; 16]);
        assert_eq!(finalizable.state, MutationState::RemoteVerified);
        assert!(store
            .latest_mutation_for_binding(
                &host_id,
                MutationKind::DeviceProvision,
                &scope_id,
                &[7; 33],
            )
            .unwrap()
            .is_none());
    }

    #[test]
    fn signup_operation_journal_is_public_and_monotonic() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.db");
        let mut store = HardStateStore::open(&path).unwrap();
        let host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let operation = SignupOperation {
            operation_id: [9; 16],
            host_id: host.host_id,
            normalized_username: b"newuser".to_vec(),
            uid: vec![1; 33],
            device_id: vec![4; 33],
            request_hash: [8; 32],
            state: SignupOperationState::Prepared,
            created_at: 100,
            updated_at: 100,
        };
        store.record_signup_operation(&operation).unwrap();
        assert_eq!(
            store.signup_operation(&[9; 16]).unwrap(),
            Some(operation.clone())
        );
        assert_eq!(
            store
                .signup_operation_for_credential(
                    &operation.host_id,
                    &operation.uid,
                    &operation.device_id,
                )
                .unwrap(),
            Some(operation.clone())
        );
        assert!(store
            .signup_operation_for_credential(&operation.host_id, &operation.uid, &[5; 33])
            .unwrap()
            .is_none());
        store
            .advance_signup_operation(&[9; 16], SignupOperationState::Submitted, 101)
            .unwrap();
        assert_eq!(
            store.signup_operation(&[9; 16]).unwrap().unwrap().state,
            SignupOperationState::Submitted
        );
        assert!(store
            .advance_signup_operation(&[9; 16], SignupOperationState::Prepared, 102)
            .is_err());
        assert!(store
            .advance_signup_operation(&[9; 16], SignupOperationState::Submitted, 100)
            .is_err());
        assert!(store
            .advance_signup_operation(&[9; 16], SignupOperationState::Verified, 99)
            .is_err());
        store
            .advance_signup_operation(&[9; 16], SignupOperationState::Verified, 102)
            .unwrap();

        let replacement = SignupOperation {
            operation_id: [8; 16],
            state: SignupOperationState::Prepared,
            created_at: 200,
            updated_at: 200,
            ..operation
        };
        store.record_signup_operation(&replacement).unwrap();
        assert_eq!(
            store
                .signup_operation_for_credential(
                    &replacement.host_id,
                    &replacement.uid,
                    &replacement.device_id,
                )
                .unwrap(),
            Some(replacement)
        );
    }

    #[test]
    fn adhoc_team_operation_journal_is_public_unique_and_monotonic() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.db");
        let mut store = HardStateStore::open(&path).unwrap();
        let host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let operation = AdHocTeamOperation {
            operation_id: [7; 16],
            host_id: host.host_id,
            uid: vec![1; 33],
            device_id: vec![4; 33],
            team_id: vec![20; 33],
            request_hash: [8; 32],
            state: AdHocTeamOperationState::Prepared,
            created_at: 200,
            updated_at: 200,
        };
        store.record_adhoc_team_operation(&operation).unwrap();
        assert_eq!(
            store.adhoc_team_operation(&[7; 16]).unwrap(),
            Some(operation.clone())
        );
        store
            .advance_adhoc_team_operation(&[7; 16], AdHocTeamOperationState::Submitted, 201)
            .unwrap();
        store
            .advance_adhoc_team_operation(&[7; 16], AdHocTeamOperationState::Verified, 202)
            .unwrap();
        assert_eq!(
            store.adhoc_team_operation(&[7; 16]).unwrap().unwrap().state,
            AdHocTeamOperationState::Verified
        );
        assert!(store
            .advance_adhoc_team_operation(&[7; 16], AdHocTeamOperationState::Submitted, 203)
            .is_err());
        let duplicate = AdHocTeamOperation {
            operation_id: [6; 16],
            ..operation
        };
        assert!(store.record_adhoc_team_operation(&duplicate).is_err());
    }

    #[test]
    fn team_mutation_rejection_releases_only_the_rejected_chain_position() {
        let (_directory, mut store) = store();
        let host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let operation = TeamMutationOperation {
            operation_id: [5; 16],
            kind: TeamMutationKind::NamedCreation,
            host_id: host.host_id,
            actor_id: vec![1; 33],
            device_id: vec![4; 33],
            team_id: vec![3; 33],
            expected_seqno: 1,
            request_hash: [6; 32],
            state: TeamMutationState::Prepared,
            created_at: 300,
            updated_at: 300,
        };
        store.record_team_mutation(&operation).unwrap();
        assert_eq!(
            store.team_mutation(&operation.operation_id).unwrap(),
            Some(operation.clone())
        );
        assert_eq!(
            store
                .team_mutation_at(
                    &operation.host_id,
                    &operation.team_id,
                    operation.expected_seqno
                )
                .unwrap(),
            Some(operation.clone())
        );
        let duplicate_transition = TeamMutationOperation {
            operation_id: [4; 16],
            kind: TeamMutationKind::MembershipChange,
            ..operation.clone()
        };
        assert!(store.record_team_mutation(&duplicate_transition).is_err());
        store
            .advance_team_mutation(&operation.operation_id, TeamMutationState::Rejected, 301)
            .unwrap();
        assert!(store
            .advance_team_mutation(&operation.operation_id, TeamMutationState::Submitted, 303)
            .is_err());
        store.record_team_mutation(&duplicate_transition).unwrap();
        assert_eq!(
            store
                .team_mutation_at(
                    &duplicate_transition.host_id,
                    &duplicate_transition.team_id,
                    duplicate_transition.expected_seqno,
                )
                .unwrap()
                .unwrap()
                .operation_id,
            duplicate_transition.operation_id
        );
        store
            .advance_team_mutation(
                &duplicate_transition.operation_id,
                TeamMutationState::Submitting,
                302,
            )
            .unwrap();
        store
            .advance_team_mutation(
                &duplicate_transition.operation_id,
                TeamMutationState::SubmissionUnknown,
                303,
            )
            .unwrap();
        // SubmissionUnknown can now be rejected to unblock a wedged sole-client
        // (fix for team mutation recovery wedge).
        store
            .advance_team_mutation(
                &duplicate_transition.operation_id,
                TeamMutationState::Rejected,
                304,
            )
            .unwrap();
        // Multiple terminal attempts can legitimately occupy one sequence.
        // Sequence lookup chooses the newest attempt, while recovery of
        // caller-durable material must remain bound to its exact operation ID.
        assert_eq!(
            store
                .team_mutation_at(
                    &operation.host_id,
                    &operation.team_id,
                    operation.expected_seqno,
                )
                .unwrap()
                .unwrap()
                .operation_id,
            duplicate_transition.operation_id
        );
        let original = store
            .team_mutation(&operation.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(original.operation_id, operation.operation_id);
        assert_eq!(original.state, TeamMutationState::Rejected);
        // Rejected is terminal and releases the chain position.
        assert!(store
            .advance_team_mutation(
                &duplicate_transition.operation_id,
                TeamMutationState::Submitted,
                305,
            )
            .is_err());
        let fresh = TeamMutationOperation {
            operation_id: [6; 16],
            state: TeamMutationState::Prepared,
            created_at: 306,
            updated_at: 306,
            ..duplicate_transition.clone()
        };
        store.record_team_mutation(&fresh).unwrap();
        store
            .advance_team_mutation(&fresh.operation_id, TeamMutationState::Submitting, 307)
            .unwrap();
        store
            .advance_team_mutation(&fresh.operation_id, TeamMutationState::Submitted, 307)
            .unwrap();
        store
            .advance_team_mutation(&fresh.operation_id, TeamMutationState::Verified, 308)
            .unwrap();
        let stale_third = TeamMutationOperation {
            operation_id: [3; 16],
            state: TeamMutationState::Prepared,
            ..fresh
        };
        assert!(store.record_team_mutation(&stale_third).is_err());

        let superseded = TeamMutationOperation {
            operation_id: [2; 16],
            expected_seqno: 2,
            created_at: 400,
            updated_at: 400,
            ..stale_third
        };
        store.record_team_mutation(&superseded).unwrap();
        store
            .advance_team_mutation(&superseded.operation_id, TeamMutationState::Submitting, 401)
            .unwrap();
        store
            .advance_team_mutation(
                &superseded.operation_id,
                TeamMutationState::SubmissionUnknown,
                402,
            )
            .unwrap();
        store
            .advance_team_mutation(&superseded.operation_id, TeamMutationState::Superseded, 403)
            .unwrap();
        assert!(store
            .advance_team_mutation(&superseded.operation_id, TeamMutationState::Verified, 404,)
            .is_err());
        let replacement = TeamMutationOperation {
            operation_id: [1; 16],
            state: TeamMutationState::Prepared,
            ..superseded
        };
        store.record_team_mutation(&replacement).unwrap();
    }

    #[test]
    fn team_mutation_records_as_submitting_and_survives_a_backwards_clock() {
        let (_directory, mut store) = store();
        let host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let leftover = TeamMutationOperation {
            operation_id: [5; 16],
            kind: TeamMutationKind::NamedCreation,
            host_id: host.host_id.clone(),
            actor_id: vec![1; 33],
            device_id: vec![4; 33],
            team_id: vec![3; 33],
            expected_seqno: 1,
            request_hash: [6; 32],
            state: TeamMutationState::Prepared,
            created_at: 300,
            updated_at: 300,
        };
        store.record_team_mutation(&leftover).unwrap();
        let operation = TeamMutationOperation {
            operation_id: [7; 16],
            request_hash: [8; 32],
            ..leftover.clone()
        };
        store
            .record_and_begin_team_mutation(&operation, 300)
            .unwrap();
        assert_eq!(
            store
                .team_mutation(&leftover.operation_id)
                .unwrap()
                .unwrap()
                .state,
            TeamMutationState::Superseded
        );
        let stored = store
            .team_mutation(&operation.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.state, TeamMutationState::Submitting);
        store
            .advance_team_mutation(&operation.operation_id, TeamMutationState::Submitted, 1)
            .unwrap();
        let advanced = store
            .team_mutation(&operation.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(advanced.state, TeamMutationState::Submitted);
        assert!(advanced.updated_at > stored.updated_at);
    }

    #[test]
    fn federation_saga_is_secret_free_idempotent_and_monotonic() {
        let (_directory, mut store) = store();
        let mut host = snapshot();
        host.host_id = [vec![foks_proto::ENTITY_HOST], vec![1; 32]].concat();
        store.accept_host_parts(host.parts()).unwrap();
        let operation = FederationSagaOperation {
            operation_id: [41; 16],
            local_host_id: host.host_id,
            remote_host_id: [vec![foks_proto::ENTITY_HOST], vec![2; 32]].concat(),
            actor_id: [vec![foks_proto::ENTITY_USER], vec![3; 32]].concat(),
            local_team_id: [vec![foks_proto::ENTITY_NAMED_TEAM], vec![4; 32]].concat(),
            remote_party_id: [vec![foks_proto::ENTITY_AD_HOC_TEAM], vec![5; 32]].concat(),
            permission_hash: [6; 32],
            destination_role_type: 1,
            destination_visibility: 0,
            removal_key_commitment: [7; 32],
            state: FederationSagaState::PermissionGranted,
            expected_local_seqno: None,
            local_mutation_id: None,
            created_at: 100,
            updated_at: 100,
        };
        let before = store.metadata().unwrap().revision;
        store.record_federation_saga(&operation).unwrap();
        store.record_federation_saga(&operation).unwrap();
        assert!(store.metadata().unwrap().revision > before);
        assert_eq!(
            store.federation_saga(&operation.operation_id).unwrap(),
            Some(operation.clone())
        );

        let conflicting = FederationSagaOperation {
            permission_hash: [8; 32],
            ..operation.clone()
        };
        assert!(store.record_federation_saga(&conflicting).is_err());
        let malformed = FederationSagaOperation {
            actor_id: [vec![foks_proto::ENTITY_NAMED_TEAM], vec![3; 32]].concat(),
            ..operation.clone()
        };
        assert!(store.record_federation_saga(&malformed).is_err());
        assert!(store
            .advance_federation_saga(
                &operation.operation_id,
                FederationSagaState::LocalPrepared,
                101,
            )
            .is_err());
        store
            .advance_federation_saga(
                &operation.operation_id,
                FederationSagaState::RemoteVerified,
                101,
            )
            .unwrap();
        store
            .prepare_federation_local_mutation(
                &operation.operation_id,
                &TeamMutationOperation {
                    operation_id: [9; 16],
                    kind: TeamMutationKind::MembershipChange,
                    host_id: operation.local_host_id.clone(),
                    actor_id: operation.actor_id.clone(),
                    device_id: [vec![foks_proto::ENTITY_DEVICE], vec![10; 32]].concat(),
                    team_id: operation.local_team_id.clone(),
                    expected_seqno: 2,
                    request_hash: [11; 32],
                    state: TeamMutationState::Prepared,
                    created_at: 102,
                    updated_at: 102,
                },
                102,
            )
            .unwrap();
        let local_mutation = store.team_mutation(&[9; 16]).unwrap().unwrap();
        store
            .prepare_federation_local_mutation(&operation.operation_id, &local_mutation, 102)
            .unwrap();
        let changed_checkpoint = TeamMutationOperation {
            expected_seqno: 3,
            ..local_mutation.clone()
        };
        assert!(store
            .prepare_federation_local_mutation(&operation.operation_id, &changed_checkpoint, 103,)
            .is_err());
        let competing_saga = FederationSagaOperation {
            operation_id: [42; 16],
            remote_party_id: [vec![foks_proto::ENTITY_AD_HOC_TEAM], vec![12; 32]].concat(),
            permission_hash: [13; 32],
            state: FederationSagaState::PermissionGranted,
            expected_local_seqno: None,
            local_mutation_id: None,
            created_at: 110,
            updated_at: 110,
            ..operation.clone()
        };
        store.record_federation_saga(&competing_saga).unwrap();
        store
            .advance_federation_saga(
                &competing_saga.operation_id,
                FederationSagaState::RemoteVerified,
                111,
            )
            .unwrap();
        let competing_mutation = TeamMutationOperation {
            operation_id: [12; 16],
            created_at: 112,
            updated_at: 112,
            ..local_mutation.clone()
        };
        assert!(store
            .prepare_federation_local_mutation(
                &competing_saga.operation_id,
                &competing_mutation,
                112,
            )
            .is_err());
        assert!(store
            .team_mutation(&competing_mutation.operation_id)
            .unwrap()
            .is_none());
        let unchanged = store
            .federation_saga(&competing_saga.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.state, FederationSagaState::RemoteVerified);
        assert_eq!(unchanged.expected_local_seqno, None);
        assert_eq!(unchanged.local_mutation_id, None);
        store
            .advance_federation_saga(
                &competing_saga.operation_id,
                FederationSagaState::Rejected,
                113,
            )
            .unwrap();
        store
            .advance_federation_saga(
                &operation.operation_id,
                FederationSagaState::LocalVerified,
                103,
            )
            .unwrap();
        assert_eq!(
            store
                .pending_federation_sagas(&operation.local_host_id)
                .unwrap()
                .len(),
            1
        );
        store
            .advance_federation_saga(&operation.operation_id, FederationSagaState::Completed, 104)
            .unwrap();
        assert!(store
            .pending_federation_sagas(&operation.local_host_id)
            .unwrap()
            .is_empty());
    }

    include!("adapter_tests.rs");

    fn snapshot() -> TestHostSnapshot {
        TestHostSnapshot {
            lookup_name: "foks.example".into(),
            host_id: vec![1; 33],
            canonical_name: "foks.example".into(),
            genesis_key: vec![2; 32],
            chain_seqno: 4,
            chain_tail_hash: [3; 32],
            chain_bytes: chain(&[1, 2, 3, 4]),
            public_zone_bytes: vec![4; 96],
            services: vec![
                HostService {
                    service_type: ServiceType::Registration,
                    endpoint_bytes: vec![5; 20],
                },
                HostService {
                    service_type: ServiceType::User,
                    endpoint_bytes: vec![6; 24],
                },
            ],
            merkle_root: TestMerkleRoot {
                epoch: 11,
                root_hash: [7; 32],
                root_bytes: vec![9; 80],
                evidence: MerkleRootEvidence::SignedBootstrap(vec![8; 96]),
                authenticated_roots: vec![AuthenticatedMerkleRoot {
                    epoch: 11,
                    root_hash: [7; 32],
                    root_bytes: Some(vec![9; 80]),
                }],
            },
        }
    }

    fn store() -> (tempfile::TempDir, HardStateStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = HardStateStore::open(&directory.path().join("hard.sqlite")).unwrap();
        (directory, store)
    }

    fn stored(snapshot: &TestHostSnapshot) -> StoredHostSnapshot {
        let parts = snapshot.parts();
        let root = parts.merkle_root;
        StoredHostSnapshot {
            lookup_name: parts.lookup_name.to_owned(),
            host_id: parts.host_id.to_vec(),
            canonical_name: parts.canonical_name.to_owned(),
            genesis_key: parts.genesis_key.to_vec(),
            chain_seqno: parts.chain_seqno,
            chain_tail_hash: parts.chain_tail_hash,
            chain_bytes: parts.chain_bytes.to_vec(),
            public_zone_bytes: parts.public_zone_bytes.to_vec(),
            services: parts.services.to_vec(),
            merkle_root: StoredMerkleRoot {
                epoch: root.epoch,
                root_hash: root.root_hash,
                root_bytes: root.root_bytes.to_vec(),
                evidence: root.evidence.clone(),
                authenticated_roots: root.authenticated_roots.to_vec(),
            },
        }
    }

    fn user_snapshot() -> TestUserSnapshot {
        TestUserSnapshot {
            host_id: vec![1; 33],
            uid: vec![1; 33],
            chain_seqno: 1,
            chain_tail_hash: [21; 32],
            chain_bytes: chain(&[21]),
            evidence_bytes: vec![22; 80],
            username: b"fixtureuser".to_vec(),
            username_utf8: b"FixtureUser".to_vec(),
            username_sequence: 1,
            merkle_epoch: 11,
            merkle_root_hash: [7; 32],
            merkle_root_bytes: vec![9; 80],
            devices: vec![foks_verify::VerifiedUserDevice {
                device_id: vec![4; 33],
                role: foks_proto::Role::OWNER,
                hepk_bytes: vec![25; 1280],
                subkey_id: None,
            }],
            shared_keys: vec![foks_verify::VerifiedUserSharedKey {
                role: foks_proto::Role::OWNER,
                generation: 1,
                verify_key: vec![14; 33],
                hepk_bytes: vec![26; 1280],
            }],
        }
    }

    fn team_snapshot() -> TestTeamSnapshot {
        TestTeamSnapshot {
            host_id: vec![1; 33],
            team_id: vec![3; 33],
            chain_seqno: 1,
            chain_tail_hash: [31; 32],
            chain_bytes: chain(&[31]),
            evidence_bytes: vec![32; 80],
            team_name: b"fixtureteam".to_vec(),
            team_name_utf8: b"FixtureTeam".to_vec(),
            team_name_sequence: 1,
            index_range: foks_proto::RationalRange {
                low: foks_proto::Rational {
                    infinity: false,
                    base: vec![1],
                    exponent: 0,
                },
                high: foks_proto::Rational {
                    infinity: true,
                    base: Vec::new(),
                    exponent: 0,
                },
            },
            merkle_epoch: 11,
            merkle_root_hash: [7; 32],
            merkle_root_bytes: vec![9; 80],
            members: vec![
                VerifiedTeamMember {
                    party_id: vec![1; 33],
                    scoped_host_id: None,
                    source_role: foks_proto::Role::OWNER,
                    role: foks_proto::Role::OWNER,
                    generation: 2,
                    verify_key: vec![14; 33],
                    hepk_fingerprint: [33; 32],
                    removal_key_commitment: Some([36; 32]),
                    index_range: None,
                },
                VerifiedTeamMember {
                    party_id: vec![3; 33],
                    scoped_host_id: Some(vec![2; 33]),
                    source_role: foks_proto::Role::ADMIN,
                    role: foks_proto::Role::member(0),
                    generation: 1,
                    verify_key: vec![14; 33],
                    hepk_fingerprint: [37; 32],
                    removal_key_commitment: Some([38; 32]),
                    index_range: Some(foks_proto::RationalRange {
                        low: foks_proto::Rational {
                            infinity: false,
                            base: vec![0x20],
                            exponent: 0,
                        },
                        high: foks_proto::Rational {
                            infinity: false,
                            base: vec![0x30],
                            exponent: 0,
                        },
                    }),
                },
            ],
            shared_keys: vec![foks_verify::VerifiedUserSharedKey {
                role: foks_proto::Role::OWNER,
                generation: 1,
                verify_key: vec![15; 33],
                hepk_bytes: vec![34; 1280],
            }],
        }
    }

    #[test]
    fn exact_authenticated_bytes_round_trip() {
        let (_directory, mut store) = store();
        let snapshot = snapshot();
        assert_eq!(
            store.accept_host_parts(snapshot.parts()).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.accept_host_parts(snapshot.parts()).unwrap(),
            Acceptance::Unchanged
        );
        assert_eq!(
            store.host_for_lookup("foks.example").unwrap(),
            Some(stored(&snapshot))
        );
    }

    #[test]
    fn discovery_name_cannot_change_host_identity() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut replacement = snapshot();
        replacement.host_id = vec![9; 33];
        assert!(matches!(
            store.accept_host_parts(replacement.parts()),
            Err(Error::HostIdentityChanged { .. })
        ));
    }

    #[test]
    fn same_sequence_chain_fork_is_rejected() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut fork = snapshot();
        fork.chain_seqno = 4;
        fork.chain_tail_hash = [9; 32];
        fork.chain_bytes = chain(&[1, 2, 3, 9]);
        assert!(matches!(
            store.accept_host_parts(fork.parts()),
            Err(Error::ChainFork { seqno: 4 })
        ));
    }

    #[test]
    fn chain_rollback_is_rejected() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut rollback = snapshot();
        rollback.chain_seqno = 3;
        rollback.chain_tail_hash = [3; 32];
        rollback.chain_bytes = chain(&[1, 2, 3]);
        assert!(matches!(
            store.accept_host_parts(rollback.parts()),
            Err(Error::ChainRollback {
                stored: 4,
                received: 3
            })
        ));
    }

    #[test]
    fn merkle_fork_and_rollback_are_rejected() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();

        let mut fork = snapshot();
        fork.merkle_root.root_hash = [9; 32];
        fork.merkle_root.authenticated_roots[0].root_hash = [9; 32];
        assert!(matches!(
            store.accept_host_parts(fork.parts()),
            Err(Error::MerkleFork { epoch: 11 })
        ));

        let mut rollback = snapshot();
        rollback.merkle_root.epoch = 10;
        rollback.merkle_root.authenticated_roots[0].epoch = 10;
        assert!(matches!(
            store.accept_host_parts(rollback.parts()),
            Err(Error::MerkleRollback {
                stored: 11,
                received: 10
            })
        ));
    }

    #[test]
    fn same_merkle_root_accepts_a_different_verified_evidence_path() {
        let (_directory, mut store) = store();
        let original = snapshot();
        store.accept_host_parts(original.parts()).unwrap();

        let mut direct = original;
        direct.merkle_root.evidence = MerkleRootEvidence::SignedBootstrap(vec![0x55; 96]);
        assert_eq!(
            store.accept_host_parts(direct.parts()).unwrap(),
            Acceptance::Unchanged
        );
    }

    #[test]
    fn signed_refresh_evidence_must_descend_to_a_prior_anchor() {
        let (_directory, mut store) = store();
        let mut invalid = snapshot();
        invalid.merkle_root.evidence = MerkleRootEvidence::SignedRefresh {
            signed_root: vec![0x55; 96],
            prior_epoch: invalid.merkle_root.epoch,
            prior: Box::new(MerkleRootEvidence::SignedBootstrap(vec![0x44; 96])),
        };
        assert!(matches!(
            store.accept_host_parts(invalid.parts()),
            Err(Error::InvalidSnapshot(
                "Merkle evidence anchors must strictly descend"
            ))
        ));
    }

    #[test]
    fn legacy_or_empty_signed_skip_evidence_is_rejected() {
        let prior = Value::Array(vec![Value::Unsigned(0), Value::Binary(vec![0x44; 96])]);
        let legacy = Value::Array(vec![
            Value::Unsigned(1),
            Value::Unsigned(1),
            Value::Binary(vec![0x55; 96]),
            prior.clone(),
        ]);
        assert!(matches!(
            evidence_from_value(&legacy, 0),
            Err(Error::InvalidSnapshot(
                "persisted Merkle evidence has an invalid shape"
            ))
        ));

        let empty_signed = Value::Array(vec![
            Value::Unsigned(1),
            Value::Unsigned(1),
            Value::Binary(vec![0x55; 96]),
            prior,
            Value::Binary(Vec::new()),
        ]);
        assert!(evidence_from_value(&empty_signed, 0).is_err());
    }

    #[test]
    fn stale_merkle_root_rolls_back_a_chain_update_atomically() {
        let (_directory, mut store) = store();
        let original = snapshot();
        store.accept_host_parts(original.parts()).unwrap();

        let mut update = snapshot();
        update.chain_seqno = 5;
        update.chain_tail_hash = [10; 32];
        update.chain_bytes = chain(&[1, 2, 3, 4, 10]);
        update.services[0].endpoint_bytes = vec![11; 20];
        update.merkle_root.epoch = 10;
        update.merkle_root.authenticated_roots[0].epoch = 10;
        assert!(matches!(
            store.accept_host_parts(update.parts()),
            Err(Error::MerkleRollback { .. })
        ));
        assert_eq!(
            store.host_for_lookup("foks.example").unwrap(),
            Some(stored(&original))
        );
    }

    #[test]
    fn chain_and_merkle_advance_together() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut update = snapshot();
        update.canonical_name = "new.foks.example".into();
        update.chain_seqno = 5;
        update.chain_tail_hash = [10; 32];
        update.chain_bytes = chain(&[1, 2, 3, 4, 10]);
        update.services[0].endpoint_bytes = vec![11; 20];
        update.merkle_root.epoch = 12;
        update.merkle_root.root_hash = [12; 32];
        update.merkle_root.root_bytes = vec![13; 80];
        update.merkle_root.evidence = MerkleRootEvidence::SkipPath {
            signed_root: vec![14; 96],
            anchor_epoch: 11,
            historical_response: vec![12; 96],
            prior: Box::new(snapshot().merkle_root.evidence),
        };
        update
            .merkle_root
            .authenticated_roots
            .push(AuthenticatedMerkleRoot {
                epoch: 12,
                root_hash: [12; 32],
                root_bytes: Some(vec![13; 80]),
            });
        assert_eq!(
            store.accept_host_parts(update.parts()).unwrap(),
            Acceptance::Advanced
        );
        let loaded = store.host_for_lookup("foks.example").unwrap().unwrap();
        assert_eq!(loaded, stored(&update));
    }

    #[test]
    fn independently_verified_public_zone_can_refresh_without_chain_advance() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut changed = snapshot();
        changed.canonical_name = "moved.foks.example".into();
        changed.public_zone_bytes = vec![42; 96];
        changed.services[0].endpoint_bytes = vec![12; 20];
        assert_eq!(
            store.accept_host_parts(changed.parts()).unwrap(),
            Acceptance::Advanced
        );
        assert_eq!(
            store.host_for_lookup("foks.example").unwrap(),
            Some(stored(&changed))
        );
    }

    #[test]
    fn database_reopens_without_losing_pins() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite");
        {
            let mut store = HardStateStore::open(&path).unwrap();
            store.accept_host_parts(snapshot().parts()).unwrap();
        }
        let store = HardStateStore::open(&path).unwrap();
        assert_eq!(
            store.host_for_lookup("foks.example").unwrap(),
            Some(stored(&snapshot()))
        );
    }

    #[test]
    fn user_projection_is_atomic_and_monotonic() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let user = user_snapshot();
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Unchanged
        );
        let mut alternate_evidence = user.clone();
        alternate_evidence.evidence_bytes = vec![0x5a; 96];
        assert_eq!(
            store.accept_user_parts(alternate_evidence.parts()).unwrap(),
            Acceptance::Unchanged
        );

        let mut fork = user.clone();
        let mut fork_hash = fork.chain_tail_hash;
        fork_hash[0] ^= 1;
        fork.chain_tail_hash = fork_hash;
        assert!(matches!(
            store.accept_user_parts(fork.parts()),
            Err(Error::UserFork { seqno: 1 })
        ));

        let mut changed_projection = user.clone();
        let mut changed_hash = changed_projection.merkle_root_hash;
        changed_hash[0] ^= 1;
        changed_projection.merkle_root_hash = changed_hash;
        assert!(matches!(
            store.accept_user_parts(changed_projection.parts()),
            Err(Error::MerkleFork { epoch: 11 })
        ));

        let mut changed_device = user;
        changed_device.devices[0].hepk_bytes[0] ^= 1;
        assert!(matches!(
            store.accept_user_parts(changed_device.parts()),
            Err(Error::UserProjectionChanged { seqno: 1 })
        ));
    }

    #[test]
    fn no_passphrase_attestation_requires_an_accepted_user_and_is_durable() {
        let (directory, mut store) = store();
        let user = user_snapshot();
        assert!(matches!(
            store.attest_user_has_no_passphrase(&user.host_id, &user.uid),
            Err(Error::InvalidUser(_))
        ));
        store.accept_host_parts(snapshot().parts()).unwrap();
        store.accept_user_parts(user.parts()).unwrap();
        store
            .attest_user_has_no_passphrase(&user.host_id, &user.uid)
            .unwrap();
        assert!(store
            .user_has_no_passphrase_attestation(&user.host_id, &user.uid)
            .unwrap());
        drop(store);
        let mut reopened = HardStateStore::open(&directory.path().join("hard.sqlite")).unwrap();
        assert!(reopened
            .user_has_no_passphrase_attestation(&user.host_id, &user.uid)
            .unwrap());
        reopened
            .clear_user_no_passphrase_attestation(&user.host_id, &user.uid)
            .unwrap();
        assert!(!reopened
            .user_has_no_passphrase_attestation(&user.host_id, &user.uid)
            .unwrap());
        let trusted_hash = [0xa5; 32];
        reopened
            .trust_user_passphrase_parcel(&user.host_id, &user.uid, &trusted_hash)
            .unwrap();
        assert_eq!(
            reopened
                .trusted_user_passphrase_parcel_hash(&user.host_id, &user.uid)
                .unwrap(),
            Some(trusted_hash)
        );
        reopened
            .attest_user_has_no_passphrase(&user.host_id, &user.uid)
            .unwrap();
        assert_eq!(
            reopened
                .trusted_user_passphrase_parcel_hash(&user.host_id, &user.uid)
                .unwrap(),
            None
        );
    }

    #[test]
    fn user_generic_chain_rejects_rollback_and_fork_at_a_new_merkle_head() {
        let (_directory, mut store) = store();
        let mut host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let mut user = user_snapshot();
        store.accept_user_parts(user.parts()).unwrap();
        let generic_chain = |links: &[u64]| {
            let mut values = vec![Value::Unsigned(foks_proto::CHAIN_TYPE_USER_SETTINGS)];
            values.extend(links.iter().copied().map(Value::Unsigned));
            encode(&Value::Array(values)).unwrap()
        };
        let first_chain = generic_chain(&[0x41]);
        store
            .attest_user_has_no_passphrase(&user.host_id, &user.uid)
            .unwrap();
        assert_eq!(
            store
                .accept_verified_user_generic_chain(&VerifiedUserGenericChainSnapshot {
                    host_id: &user.host_id,
                    uid: &user.uid,
                    chain_type: foks_proto::CHAIN_TYPE_USER_SETTINGS,
                    sequence: 1,
                    tail_hash: Some([0x42; 32]),
                    chain_bytes: &first_chain,
                    merkle_epoch: user.merkle_epoch,
                    merkle_root_hash: user.merkle_root_hash,
                })
                .unwrap(),
            Acceptance::Inserted
        );
        assert!(!store
            .user_has_no_passphrase_attestation(&user.host_id, &user.uid)
            .unwrap());

        host.merkle_root.epoch = 12;
        host.merkle_root.root_hash = [8; 32];
        host.merkle_root.root_bytes = vec![10; 80];
        host.merkle_root.authenticated_roots = vec![AuthenticatedMerkleRoot {
            epoch: 12,
            root_hash: [8; 32],
            root_bytes: Some(vec![10; 80]),
        }];
        store.accept_host_parts(host.parts()).unwrap();
        user.merkle_epoch = 12;
        user.merkle_root_hash = [8; 32];
        user.merkle_root_bytes = vec![10; 80];
        store.accept_user_parts(user.parts()).unwrap();

        assert!(matches!(
            store.accept_verified_user_generic_chain(&VerifiedUserGenericChainSnapshot {
                host_id: &user.host_id,
                uid: &user.uid,
                chain_type: foks_proto::CHAIN_TYPE_USER_SETTINGS,
                sequence: 0,
                tail_hash: None,
                chain_bytes: &generic_chain(&[]),
                merkle_epoch: 12,
                merkle_root_hash: [8; 32],
            }),
            Err(Error::UserGenericRollback {
                stored: 1,
                received: 0,
                ..
            })
        ));
        assert!(matches!(
            store.accept_verified_user_generic_chain(&VerifiedUserGenericChainSnapshot {
                host_id: &user.host_id,
                uid: &user.uid,
                chain_type: foks_proto::CHAIN_TYPE_USER_SETTINGS,
                sequence: 1,
                tail_hash: Some([0x44; 32]),
                chain_bytes: &generic_chain(&[0x43]),
                merkle_epoch: 12,
                merkle_root_hash: [8; 32],
            }),
            Err(Error::UserGenericFork { seqno: 1, .. })
        ));
    }

    #[test]
    fn team_projection_is_atomic_monotonic_and_round_trips() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let team = team_snapshot();
        assert_eq!(
            store.accept_team_parts(team.parts()).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.accept_team_parts(team.parts()).unwrap(),
            Acceptance::Unchanged
        );
        let loaded = store
            .team_for_host(&team.host_id, &team.team_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.chain_bytes, team.chain_bytes);
        assert_eq!(loaded.evidence_bytes, team.evidence_bytes);
        assert_eq!(loaded.index_range, team.index_range);
        assert_eq!(loaded.members, team.members);
        assert_eq!(loaded.shared_keys, team.shared_keys);
        assert!(store
            .connection
            .execute("UPDATE teams SET index_low_base = ?1", [vec![1; 33]])
            .is_err());
        assert!(store
            .connection
            .execute("UPDATE teams SET index_low_exponent = 4097", [])
            .is_err());
        assert!(store
            .connection
            .execute("UPDATE team_members SET index_high_exponent = -4097", [])
            .is_err());

        let mut alternate_evidence = team.clone();
        alternate_evidence.evidence_bytes = vec![0xa5; 96];
        assert_eq!(
            store.accept_team_parts(alternate_evidence.parts()).unwrap(),
            Acceptance::Unchanged
        );

        let mut changed = team.clone();
        changed.members[0].hepk_fingerprint[0] ^= 1;
        assert!(matches!(
            store.accept_team_parts(changed.parts()),
            Err(Error::TeamProjectionChanged { seqno: 1 })
        ));

        let mut changed_commitment = team.clone();
        changed_commitment.members[0]
            .removal_key_commitment
            .as_mut()
            .unwrap()[0] ^= 1;
        assert!(matches!(
            store.accept_team_parts(changed_commitment.parts()),
            Err(Error::TeamProjectionChanged { seqno: 1 })
        ));

        let mut fork = team;
        fork.chain_tail_hash[0] ^= 1;
        assert!(matches!(
            store.accept_team_parts(fork.parts()),
            Err(Error::TeamFork { seqno: 1 })
        ));
    }

    #[test]
    fn historical_root_cannot_win_a_race_with_the_current_host_head() {
        let (_directory, mut store) = store();
        let original = snapshot();
        store.accept_host_parts(original.parts()).unwrap();

        let mut advanced = original.clone();
        advanced.merkle_root.epoch = 12;
        advanced.merkle_root.root_hash = [12; 32];
        advanced.merkle_root.root_bytes = vec![13; 80];
        advanced.merkle_root.evidence = MerkleRootEvidence::SkipPath {
            signed_root: vec![14; 96],
            anchor_epoch: 11,
            historical_response: vec![12; 96],
            prior: Box::new(original.merkle_root.evidence.clone()),
        };
        advanced
            .merkle_root
            .authenticated_roots
            .push(AuthenticatedMerkleRoot {
                epoch: 12,
                root_hash: [12; 32],
                root_bytes: Some(vec![13; 80]),
            });
        store.accept_host_parts(advanced.parts()).unwrap();

        assert!(matches!(
            store.accept_user_parts(user_snapshot().parts()),
            Err(Error::MerkleRollback {
                stored: 12,
                received: 11
            })
        ));
        assert!(matches!(
            store.accept_team_parts(team_snapshot().parts()),
            Err(Error::MerkleRollback {
                stored: 12,
                received: 11
            })
        ));
        assert!(store
            .user_for_host(&original.host_id, &user_snapshot().uid)
            .unwrap()
            .is_none());
        assert!(store
            .team_for_host(&original.host_id, &team_snapshot().team_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn yubi_device_ids_are_valid_durable_user_state() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut user = user_snapshot();
        user.devices[0].device_id = vec![8; 34];
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Unchanged
        );
    }

    #[test]
    fn longer_chain_forks_are_rejected() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();

        let mut host_fork = snapshot();
        host_fork.chain_seqno = 5;
        host_fork.chain_tail_hash = [44; 32];
        host_fork.chain_bytes = chain(&[1, 2, 9, 4, 5]);
        assert!(matches!(
            store.accept_host_parts(host_fork.parts()),
            Err(Error::ChainFork { seqno: 5 })
        ));

        let user = user_snapshot();
        store.accept_user_parts(user.parts()).unwrap();
        let mut user_fork = user;
        user_fork.chain_seqno = 2;
        user_fork.chain_tail_hash = [45; 32];
        user_fork.chain_bytes = chain(&[99, 2]);
        assert!(matches!(
            store.accept_user_parts(user_fork.parts()),
            Err(Error::UserFork { seqno: 2 })
        ));
    }

    #[test]
    fn unchanged_user_chain_can_be_reauthenticated_at_a_new_root() {
        let (_directory, mut store) = store();
        let mut host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let mut user = user_snapshot();
        store.accept_user_parts(user.parts()).unwrap();

        host.merkle_root.epoch = 12;
        host.merkle_root.root_hash = [12; 32];
        host.merkle_root.root_bytes = vec![13; 80];
        host.merkle_root.evidence = MerkleRootEvidence::SkipPath {
            signed_root: vec![14; 96],
            anchor_epoch: 11,
            historical_response: vec![12; 96],
            prior: Box::new(snapshot().merkle_root.evidence),
        };
        host.merkle_root
            .authenticated_roots
            .push(AuthenticatedMerkleRoot {
                epoch: 12,
                root_hash: [12; 32],
                root_bytes: Some(vec![13; 80]),
            });
        store.accept_host_parts(host.parts()).unwrap();

        user.merkle_epoch = 12;
        user.merkle_root_hash = [12; 32];
        user.merkle_root_bytes = vec![13; 80];
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Advanced
        );
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Unchanged
        );
    }
}

mod inspection;
