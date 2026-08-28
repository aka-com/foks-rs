//! Durable SQLite hard state and isolated soft projections for a native FOKS client.
//!
//! This crate is intentionally below the protocol verifier. It preserves the
//! exact signed bytes supplied by that verifier and enforces monotonic pins,
//! but it does not parse Snowpack or verify signatures itself.

#![forbid(unsafe_code)]

mod schema;
mod soft;
mod soft_schema;

pub use soft::{KvDirectoryProjection, KvLargeFileStage, KvProjectedEntry, SoftStateStore};

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use foks_proto::ServiceType;
use foks_snowpack::{decode, encode, Value};
use foks_verify::{
    AuthenticatedMerkleRoot, HostService, MerkleRootEvidence, VerifiedHostSnapshot,
    VerifiedHostSnapshotParts, VerifiedMerkleRoot, VerifiedMerkleRootParts, VerifiedTeamMember,
    VerifiedTeamSnapshot, VerifiedTeamSnapshotParts, VerifiedUserSnapshot,
    VerifiedUserSnapshotParts,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension as _, TransactionBehavior};
use thiserror::Error;

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
}

impl ScheduledJobKind {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::UserRefresh),
            2 => Ok(Self::MutationReconcile),
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
    Verified = 4,
    Rejected = 5,
}

impl MutationState {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Prepared),
            2 => Ok(Self::Submitting),
            3 => Ok(Self::SubmissionUnknown),
            4 => Ok(Self::Verified),
            5 => Ok(Self::Rejected),
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
                        Self::SubmissionUnknown | Self::Verified | Self::Rejected
                    )
                    | (Self::SubmissionUnknown, Self::Verified | Self::Rejected)
            )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Verified | Self::Rejected)
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
}

impl TeamMutationKind {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::NamedCreation),
            2 => Ok(Self::MembershipChange),
            3 => Ok(Self::PtkRotation),
            _ => Err(Error::InvalidTeamMutation("unknown operation kind")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TeamMutationState {
    Prepared = 1,
    Submitted = 2,
    Verified = 3,
    Rejected = 4,
    Superseded = 5,
}

impl TeamMutationState {
    fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Prepared),
            2 => Ok(Self::Submitted),
            3 => Ok(Self::Verified),
            4 => Ok(Self::Rejected),
            5 => Ok(Self::Superseded),
            _ => Err(Error::InvalidTeamMutation("unknown operation state")),
        }
    }

    fn can_transition_to(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (
                    Self::Prepared,
                    Self::Submitted | Self::Rejected | Self::Superseded
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

#[derive(Debug, Error)]
pub enum Error {
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
    #[error("hard-state schema version {found} is unsupported; this build supports {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("lookup name {lookup_name:?} is pinned to a different HostID")]
    HostIdentityChanged { lookup_name: String },
    #[error("the genesis key changed for an already-pinned HostID")]
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
    #[error("soft-state schema version {found} is unsupported; this build supports {supported}")]
    UnsupportedSoftSchema { found: u32, supported: u32 },
    #[error("invalid verified KV projection")]
    InvalidKvProjection,
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
    #[error("invalid generic mutation operation: {0}")]
    InvalidMutationOperation(&'static str),
    #[error("invalid scheduled job: {0}")]
    InvalidScheduledJob(&'static str),
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

impl HardStateStore {
    /// Opens or initializes a FOKS hard-state database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        // SQLite cannot combine CREATE and NOFOLLOW on every supported build.
        // create_new is itself symlink-safe and gives NOFOLLOW an existing
        // inode to open without weakening the subsequent database open.
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(Error::SymlinkDatabase);
        }
        // macOS exposes /tmp as a symlink to /private/tmp. SQLite's NOFOLLOW
        // rejects symlinks in any path component, so resolve safe parent
        // aliases after separately rejecting a symlink at the database leaf.
        let database_path = path.canonicalize()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = Connection::open_with_flags(database_path, flags)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "trusted_schema", false)?;
        connection.pragma_update(None, "secure_delete", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        initialize_or_verify(&mut connection)?;
        Ok(Self { connection })
    }

    /// Durably records a mutation before any network submission. Protected
    /// material identified by `material_ref` must already be committed by the
    /// caller; an orphaned material record is safe, while a journal row with
    /// missing material is not recoverable.
    pub fn record_mutation(&mut self, operation: &MutationOperation) -> Result<()> {
        validate_mutation_operation(operation)?;
        if operation.state != MutationState::Prepared
            || operation.attempt_count != 0
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidMutationOperation(
                "new operation must be unattempted and prepared",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        self.connection.execute(
            "INSERT INTO mutation_operations (
                operation_id, operation_kind, host_id, scope_id, subject_id,
                expected_version, request_hash, material_ref, material_hash,
                state, attempt_count, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                operation.operation_id.as_slice(),
                operation.kind as u8,
                operation.host_id,
                operation.scope_id,
                operation.subject_id,
                operation
                    .expected_version
                    .map(|value| sqlite_integer("mutation expected version", value))
                    .transpose()?,
                operation.request_hash.as_slice(),
                operation.material_ref,
                operation.material_hash.as_slice(),
                operation.state as u8,
                sqlite_integer("mutation attempt count", operation.attempt_count)?,
                sqlite_integer("mutation created time", operation.created_at)?,
                sqlite_integer("mutation updated time", operation.updated_at)?,
            ],
        )?;
        Ok(())
    }

    /// Atomically marks the one and only initial submission attempt. Once this
    /// commits, a crash is treated as an ambiguous response and recovery must
    /// inspect authenticated server state rather than reposting the request.
    pub fn begin_mutation_submission(
        &mut self,
        operation_id: &[u8; 16],
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (state, created_at, previous_updated_at, attempts) = transaction
            .query_row(
                "SELECT state, created_at, updated_at, attempt_count
                 FROM mutation_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidMutationOperation("operation is not recorded"))?;
        if MutationState::from_sql(state)? != MutationState::Prepared
            || attempts != 0
            || updated_at < stored_unsigned("mutation created time", created_at)?
            || updated_at < stored_unsigned("mutation updated time", previous_updated_at)?
        {
            return Err(Error::InvalidMutationOperation(
                "mutation cannot be submitted more than once",
            ));
        }
        transaction.execute(
            "UPDATE mutation_operations
             SET state = ?2, attempt_count = 1, updated_at = ?3
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                MutationState::Submitting as u8,
                sqlite_integer("mutation updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn advance_mutation(
        &mut self,
        operation_id: &[u8; 16],
        state: MutationState,
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, created_at, previous_updated_at) = transaction
            .query_row(
                "SELECT state, created_at, updated_at
                 FROM mutation_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidMutationOperation("operation is not recorded"))?;
        let current = MutationState::from_sql(current)?;
        if !current.can_transition_to(state)
            || updated_at < stored_unsigned("mutation created time", created_at)?
            || updated_at < stored_unsigned("mutation updated time", previous_updated_at)?
        {
            return Err(Error::InvalidMutationOperation(
                "operation state transition is invalid",
            ));
        }
        transaction.execute(
            "UPDATE mutation_operations SET state = ?2, updated_at = ?3
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                state as u8,
                sqlite_integer("mutation updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn mutation(&self, operation_id: &[u8; 16]) -> Result<Option<MutationOperation>> {
        self.connection
            .query_row(
                "SELECT operation_kind, host_id, scope_id, subject_id,
                        expected_version, request_hash, material_ref, material_hash, state,
                        attempt_count, created_at, updated_at
                 FROM mutation_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| mutation_operation_from_row(*operation_id, row),
            )
            .optional()?
            .map(Ok)
            .transpose()
    }

    /// Returns all nonterminal operations in deterministic creation order for
    /// startup reconciliation.
    pub fn pending_mutations(&self, host_id: &[u8]) -> Result<Vec<MutationOperation>> {
        let mut statement = self.connection.prepare(
            "SELECT operation_id, operation_kind, host_id, scope_id, subject_id,
                    expected_version, request_hash, material_ref, material_hash, state,
                    attempt_count, created_at, updated_at
             FROM mutation_operations
             WHERE host_id = ?1 AND state IN (1, 2, 3)
             ORDER BY created_at, operation_id",
        )?;
        let rows = statement.query_map([host_id], |row| {
            let operation_id = row.get::<_, Vec<u8>>(0)?;
            let operation_id: [u8; 16] = operation_id.try_into().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    16,
                    rusqlite::types::Type::Blob,
                    "invalid mutation operation ID".into(),
                )
            })?;
            mutation_operation_from_offset(operation_id, row, 1)
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    /// Registers a resumable job, or refreshes its interval without delaying
    /// work that was already due. A job ID can never be rebound to another
    /// host, scope, or kind.
    pub fn register_scheduled_job(&mut self, job: &ScheduledJob) -> Result<()> {
        validate_scheduled_job(job)?;
        if job.failure_count != 0
            || job.lease_until.is_some()
            || job.last_completed_at.is_some()
            || job.last_error.is_some()
        {
            return Err(Error::InvalidScheduledJob(
                "new jobs cannot contain execution state",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&job.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        let existing = self
            .connection
            .query_row(
                "SELECT job_kind, host_id, scope_id FROM scheduled_jobs WHERE job_id = ?1",
                [job.job_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        if existing.is_some_and(|(kind, host_id, scope_id)| {
            kind != job.kind as i64 || host_id != job.host_id || scope_id != job.scope_id
        }) {
            return Err(Error::InvalidScheduledJob("job ID binding changed"));
        }
        self.connection.execute(
            "INSERT INTO scheduled_jobs (
                job_id, job_kind, host_id, scope_id, interval_micros,
                next_run_at, failure_count, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
             ON CONFLICT(job_id) DO UPDATE SET
                interval_micros = excluded.interval_micros,
                next_run_at = MIN(scheduled_jobs.next_run_at, excluded.next_run_at),
                updated_at = MAX(scheduled_jobs.updated_at, excluded.updated_at)",
            params![
                job.job_id.as_slice(),
                job.kind as u8,
                job.host_id,
                job.scope_id,
                sqlite_integer("scheduled interval", job.interval_micros)?,
                sqlite_integer("scheduled next run", job.next_run_at)?,
                sqlite_integer("scheduled update time", job.updated_at)?,
            ],
        )?;
        Ok(())
    }

    /// Claims due jobs under an expiring lease. An abandoned claim becomes
    /// runnable again after `lease_until`, so only idempotent work belongs in
    /// this scheduler.
    pub fn claim_due_scheduled_jobs(
        &mut self,
        now: u64,
        lease_until: u64,
        limit: u64,
    ) -> Result<Vec<ScheduledJob>> {
        if limit == 0 || lease_until <= now {
            return Err(Error::InvalidScheduledJob("invalid claim bounds"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now_sql = sqlite_integer("scheduled claim time", now)?;
        let lease_sql = sqlite_integer("scheduled lease time", lease_until)?;
        let limit_sql = sqlite_integer("scheduled claim limit", limit)?;
        let ids = {
            let mut statement = transaction.prepare(
                "SELECT job_id FROM scheduled_jobs
                 WHERE next_run_at <= ?1 AND (lease_until IS NULL OR lease_until <= ?1)
                 ORDER BY next_run_at, job_id LIMIT ?2",
            )?;
            let ids = statement
                .query_map(params![now_sql, limit_sql], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ids
        };
        for id in &ids {
            transaction.execute(
                "UPDATE scheduled_jobs SET lease_until = ?2, updated_at = ?3 WHERE job_id = ?1",
                params![id, lease_sql, now_sql],
            )?;
        }
        let mut jobs = Vec::with_capacity(ids.len());
        for id in ids {
            jobs.push(transaction.query_row(
                "SELECT job_kind, host_id, scope_id, interval_micros, next_run_at,
                        failure_count, lease_until, last_completed_at, last_error, updated_at
                 FROM scheduled_jobs WHERE job_id = ?1",
                [id.as_slice()],
                |row| scheduled_job_from_row(&id, row),
            )?);
        }
        transaction.commit()?;
        Ok(jobs)
    }

    pub fn complete_scheduled_job(
        &mut self,
        job_id: &[u8; 16],
        claimed_until: u64,
        next_run_at: u64,
        completed_at: u64,
    ) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE scheduled_jobs SET
                next_run_at = ?3, failure_count = 0, lease_until = NULL,
                last_completed_at = ?4, last_error = NULL, updated_at = ?4
             WHERE job_id = ?1 AND lease_until = ?2",
            params![
                job_id.as_slice(),
                sqlite_integer("scheduled claimed lease", claimed_until)?,
                sqlite_integer("scheduled next run", next_run_at)?,
                sqlite_integer("scheduled completion time", completed_at)?,
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidScheduledJob("scheduled job lease was lost"));
        }
        Ok(())
    }

    pub fn fail_scheduled_job(
        &mut self,
        job_id: &[u8; 16],
        claimed_until: u64,
        next_run_at: u64,
        failed_at: u64,
        error: &str,
    ) -> Result<()> {
        if error.is_empty() || error.len() > 1024 {
            return Err(Error::InvalidScheduledJob("invalid scheduled job error"));
        }
        let changed = self.connection.execute(
            "UPDATE scheduled_jobs SET
                next_run_at = ?3, failure_count = failure_count + 1,
                lease_until = NULL, last_error = ?5, updated_at = ?4
             WHERE job_id = ?1 AND lease_until = ?2",
            params![
                job_id.as_slice(),
                sqlite_integer("scheduled claimed lease", claimed_until)?,
                sqlite_integer("scheduled next run", next_run_at)?,
                sqlite_integer("scheduled failure time", failed_at)?,
                error,
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidScheduledJob("scheduled job lease was lost"));
        }
        Ok(())
    }

    pub fn scheduled_job(&self, job_id: &[u8; 16]) -> Result<Option<ScheduledJob>> {
        self.connection
            .query_row(
                "SELECT job_kind, host_id, scope_id, interval_micros, next_run_at,
                        failure_count, lease_until, last_completed_at, last_error, updated_at
                 FROM scheduled_jobs WHERE job_id = ?1",
                [job_id.as_slice()],
                |row| scheduled_job_from_row(job_id, row),
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn next_scheduled_run(&self) -> Result<Option<u64>> {
        let value = self.connection.query_row(
            "SELECT MIN(MAX(next_run_at, COALESCE(lease_until, next_run_at)))
             FROM scheduled_jobs",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        value
            .map(|value| stored_unsigned("next scheduled run", value))
            .transpose()
    }

    pub fn remove_scheduled_job(&mut self, job_id: &[u8; 16]) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM scheduled_jobs WHERE job_id = ?1",
            [job_id.as_slice()],
        )? == 1)
    }

    /// Records only the public fingerprint of a prepared signup. The caller's
    /// encrypted credential store remains authoritative for retry material.
    pub fn record_signup_operation(&mut self, operation: &SignupOperation) -> Result<()> {
        validate_signup_operation(operation)?;
        if operation.state != SignupOperationState::Prepared
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidSignupOperation(
                "new operation must be in the prepared state",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        self.connection.execute(
            "INSERT INTO signup_operations (
                operation_id, host_id, normalized_username, uid, device_id,
                request_hash, state, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                operation.operation_id.as_slice(),
                operation.host_id,
                operation.normalized_username,
                operation.uid,
                operation.device_id,
                operation.request_hash.as_slice(),
                operation.state as u8,
                sqlite_integer("signup created time", operation.created_at)?,
                sqlite_integer("signup updated time", operation.updated_at)?,
            ],
        )?;
        Ok(())
    }

    pub fn advance_signup_operation(
        &mut self,
        operation_id: &[u8; 16],
        state: SignupOperationState,
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT state, created_at, updated_at
                 FROM signup_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidSignupOperation("operation is not recorded"))?;
        let current_state = SignupOperationState::from_sql(current.0)?;
        let created_at = stored_unsigned("signup created time", current.1)?;
        let previous_updated_at = stored_unsigned("signup updated time", current.2)?;
        if updated_at < created_at
            || updated_at < previous_updated_at
            || (state as u8) < current_state as u8
            || (state as u8) > (current_state as u8).saturating_add(1)
        {
            return Err(Error::InvalidSignupOperation(
                "operation state transition is not monotonic",
            ));
        }
        transaction.execute(
            "UPDATE signup_operations SET state = ?2, updated_at = ?3 WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                state as u8,
                sqlite_integer("signup updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn signup_operation(&self, operation_id: &[u8; 16]) -> Result<Option<SignupOperation>> {
        self.connection
            .query_row(
                "SELECT host_id, normalized_username, uid, device_id, request_hash,
                        state, created_at, updated_at
                 FROM signup_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?
            .map(|row| {
                Ok(SignupOperation {
                    operation_id: *operation_id,
                    host_id: row.0,
                    normalized_username: row.1,
                    uid: row.2,
                    device_id: row.3,
                    request_hash: row.4.try_into().map_err(|_| {
                        Error::InvalidSignupOperation("stored request hash has the wrong length")
                    })?,
                    state: SignupOperationState::from_sql(row.5)?,
                    created_at: stored_unsigned("signup created time", row.6)?,
                    updated_at: stored_unsigned("signup updated time", row.7)?,
                })
            })
            .transpose()
    }

    /// Locates an interrupted signup from public identities that can be
    /// re-derived from caller-retained device and PUK seeds. Nonterminal rows
    /// are preferred so a process can recover even when its randomly generated
    /// operation ID was never returned to the caller.
    pub fn signup_operation_for_credential(
        &self,
        host_id: &[u8],
        uid: &[u8],
        device_id: &[u8],
    ) -> Result<Option<SignupOperation>> {
        let operation_id = self
            .connection
            .query_row(
                "SELECT operation_id FROM signup_operations
                 WHERE host_id = ?1 AND uid = ?2 AND device_id = ?3
                 ORDER BY CASE WHEN state = 3 THEN 1 ELSE 0 END,
                          updated_at DESC, operation_id DESC
                 LIMIT 1",
                rusqlite::params![host_id, uid, device_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|bytes| {
                bytes.try_into().map_err(|_| {
                    Error::InvalidSignupOperation("stored operation ID has the wrong length")
                })
            })
            .transpose()?;
        operation_id
            .as_ref()
            .map(|operation_id| self.signup_operation(operation_id))
            .transpose()
            .map(Option::flatten)
    }

    /// Records the public identity and request fingerprint for a prepared
    /// ad-hoc team creation. PTK seeds are deliberately caller-owned.
    pub fn record_adhoc_team_operation(&mut self, operation: &AdHocTeamOperation) -> Result<()> {
        validate_adhoc_team_operation(operation)?;
        if operation.state != AdHocTeamOperationState::Prepared
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidAdHocTeamOperation(
                "new operation must be in the prepared state",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        self.connection.execute(
            "INSERT INTO adhoc_team_operations (
                operation_id, host_id, uid, device_id, team_id, request_hash,
                state, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                operation.operation_id.as_slice(),
                operation.host_id,
                operation.uid,
                operation.device_id,
                operation.team_id,
                operation.request_hash.as_slice(),
                operation.state as u8,
                sqlite_integer("ad-hoc team created time", operation.created_at)?,
                sqlite_integer("ad-hoc team updated time", operation.updated_at)?,
            ],
        )?;
        Ok(())
    }

    pub fn advance_adhoc_team_operation(
        &mut self,
        operation_id: &[u8; 16],
        state: AdHocTeamOperationState,
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT state, created_at, updated_at
                 FROM adhoc_team_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidAdHocTeamOperation(
                "operation is not recorded",
            ))?;
        let current_state = AdHocTeamOperationState::from_sql(current.0)?;
        let created_at = stored_unsigned("ad-hoc team created time", current.1)?;
        let previous_updated_at = stored_unsigned("ad-hoc team updated time", current.2)?;
        if updated_at < created_at
            || updated_at < previous_updated_at
            || (state as u8) < current_state as u8
            || (state as u8) > (current_state as u8).saturating_add(1)
        {
            return Err(Error::InvalidAdHocTeamOperation(
                "operation state transition is not monotonic",
            ));
        }
        transaction.execute(
            "UPDATE adhoc_team_operations SET state = ?2, updated_at = ?3
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                state as u8,
                sqlite_integer("ad-hoc team updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn adhoc_team_operation(
        &self,
        operation_id: &[u8; 16],
    ) -> Result<Option<AdHocTeamOperation>> {
        self.connection
            .query_row(
                "SELECT host_id, uid, device_id, team_id, request_hash,
                        state, created_at, updated_at
                 FROM adhoc_team_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?
            .map(|row| {
                Ok(AdHocTeamOperation {
                    operation_id: *operation_id,
                    host_id: row.0,
                    uid: row.1,
                    device_id: row.2,
                    team_id: row.3,
                    request_hash: row.4.try_into().map_err(|_| {
                        Error::InvalidAdHocTeamOperation("stored request hash has the wrong length")
                    })?,
                    state: AdHocTeamOperationState::from_sql(row.5)?,
                    created_at: stored_unsigned("ad-hoc team created time", row.6)?,
                    updated_at: stored_unsigned("ad-hoc team updated time", row.7)?,
                })
            })
            .transpose()
    }

    pub fn record_team_mutation(&mut self, operation: &TeamMutationOperation) -> Result<()> {
        validate_team_mutation(operation)?;
        if operation.state != TeamMutationState::Prepared
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidTeamMutation(
                "new operation must be in the prepared state",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        self.connection.execute(
            "INSERT INTO team_mutation_operations (
                operation_id, operation_kind, host_id, actor_id, device_id,
                team_id, expected_seqno, request_hash, state, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                operation.operation_id.as_slice(),
                operation.kind as u8,
                operation.host_id,
                operation.actor_id,
                operation.device_id,
                operation.team_id,
                sqlite_integer("team mutation sequence", operation.expected_seqno)?,
                operation.request_hash.as_slice(),
                operation.state as u8,
                sqlite_integer("team mutation created time", operation.created_at)?,
                sqlite_integer("team mutation updated time", operation.updated_at)?,
            ],
        )?;
        Ok(())
    }

    pub fn advance_team_mutation(
        &mut self,
        operation_id: &[u8; 16],
        state: TeamMutationState,
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT state, created_at, updated_at FROM team_mutation_operations
                 WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidTeamMutation("operation is not recorded"))?;
        let current_state = TeamMutationState::from_sql(current.0)?;
        let created_at = stored_unsigned("team mutation created time", current.1)?;
        let previous_updated_at = stored_unsigned("team mutation updated time", current.2)?;
        if updated_at < created_at
            || updated_at < previous_updated_at
            || !current_state.can_transition_to(state)
        {
            return Err(Error::InvalidTeamMutation(
                "operation state transition is invalid",
            ));
        }
        transaction.execute(
            "UPDATE team_mutation_operations SET state = ?2, updated_at = ?3
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                state as u8,
                sqlite_integer("team mutation updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn team_mutation(&self, operation_id: &[u8; 16]) -> Result<Option<TeamMutationOperation>> {
        self.connection
            .query_row(
                "SELECT operation_kind, host_id, actor_id, device_id, team_id,
                        expected_seqno, request_hash, state, created_at, updated_at
                 FROM team_mutation_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                    ))
                },
            )
            .optional()?
            .map(|row| {
                Ok(TeamMutationOperation {
                    operation_id: *operation_id,
                    kind: TeamMutationKind::from_sql(row.0)?,
                    host_id: row.1,
                    actor_id: row.2,
                    device_id: row.3,
                    team_id: row.4,
                    expected_seqno: stored_unsigned("team mutation sequence", row.5)?,
                    request_hash: row.6.try_into().map_err(|_| {
                        Error::InvalidTeamMutation("stored request hash has the wrong length")
                    })?,
                    state: TeamMutationState::from_sql(row.7)?,
                    created_at: stored_unsigned("team mutation created time", row.8)?,
                    updated_at: stored_unsigned("team mutation updated time", row.9)?,
                })
            })
            .transpose()
    }

    /// Finds the unique journal row occupying one authenticated team-chain
    /// position. This supports crash recovery when the secret-derived
    /// operation ID was not returned to the caller before interruption.
    pub fn team_mutation_at(
        &self,
        host_id: &[u8],
        team_id: &[u8],
        expected_seqno: u64,
    ) -> Result<Option<TeamMutationOperation>> {
        let sequence = sqlite_integer("team mutation sequence", expected_seqno)?;
        let operation_id = self
            .connection
            .query_row(
                "SELECT operation_id FROM team_mutation_operations
                 WHERE host_id = ?1 AND team_id = ?2 AND expected_seqno = ?3
                 ORDER BY CASE
                              WHEN state IN (1, 2) THEN 0
                              WHEN state = 3 THEN 1
                              ELSE 2
                          END,
                          updated_at DESC, operation_id DESC
                 LIMIT 1",
                rusqlite::params![host_id, team_id, sequence],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|bytes| {
                bytes.try_into().map_err(|_| {
                    Error::InvalidTeamMutation("stored operation ID has the wrong length")
                })
            })
            .transpose()?;
        operation_id
            .as_ref()
            .map(|operation_id| self.team_mutation(operation_id))
            .transpose()
            .map(Option::flatten)
    }

    /// Atomically accepts a complete, already-verified host snapshot.
    ///
    /// A failed monotonicity check rolls back every part of the update,
    /// including the host chain when the accompanying Merkle root is stale.
    pub fn accept_verified_host(&mut self, snapshot: &VerifiedHostSnapshot) -> Result<Acceptance> {
        let snapshot = snapshot.parts();
        self.accept_host_parts(snapshot)
    }

    fn accept_host_parts(&mut self, snapshot: VerifiedHostSnapshotParts<'_>) -> Result<Acceptance> {
        validate_snapshot(snapshot)?;
        let chain_seqno = sqlite_integer("chain sequence", snapshot.chain_seqno)?;
        let merkle_epoch = sqlite_integer("Merkle epoch", snapshot.merkle_root.epoch)?;
        let mut service_types = snapshot
            .services
            .iter()
            .map(|service| sqlite_integer("service type", service.service_type.protocol_value()))
            .collect::<Result<Vec<_>>>()?;
        service_types.sort_unstable();
        if service_types.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::InvalidSnapshot("service types must be unique"));
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let pinned_host_id = transaction
            .query_row(
                "SELECT host_id FROM host_lookups WHERE lookup_name = ?1",
                [&snapshot.lookup_name],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if pinned_host_id
            .as_ref()
            .is_some_and(|host_id| host_id != snapshot.host_id)
        {
            return Err(Error::HostIdentityChanged {
                lookup_name: snapshot.lookup_name.to_owned(),
            });
        }

        let stored = load_host_row(&transaction, snapshot.host_id)?;
        let chain_advanced = match &stored {
            None => {
                transaction.execute(
                    "INSERT INTO hosts (host_id, canonical_name, genesis_key, chain_seqno, \
                     chain_tail_hash, chain_bytes, public_zone_bytes) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        snapshot.host_id,
                        snapshot.canonical_name,
                        snapshot.genesis_key,
                        chain_seqno,
                        snapshot.chain_tail_hash.as_slice(),
                        snapshot.chain_bytes,
                        snapshot.public_zone_bytes,
                    ],
                )?;
                true
            }
            Some(stored) => {
                if stored.genesis_key != snapshot.genesis_key {
                    return Err(Error::GenesisChanged);
                }
                match snapshot.chain_seqno.cmp(&stored.chain_seqno) {
                    std::cmp::Ordering::Less => {
                        return Err(Error::ChainRollback {
                            stored: stored.chain_seqno,
                            received: snapshot.chain_seqno,
                        });
                    }
                    std::cmp::Ordering::Equal => {
                        if stored.chain_tail_hash != snapshot.chain_tail_hash
                            || stored.chain_bytes != snapshot.chain_bytes
                        {
                            return Err(Error::ChainFork {
                                seqno: snapshot.chain_seqno,
                            });
                        }
                        let stored_services = load_services(&transaction, snapshot.host_id)?;
                        if stored.canonical_name != snapshot.canonical_name
                            || stored.public_zone_bytes != snapshot.public_zone_bytes
                            || stored_services != normalized_services(snapshot.services)
                        {
                            return Err(Error::ProjectionChanged {
                                seqno: snapshot.chain_seqno,
                            });
                        }
                        false
                    }
                    std::cmp::Ordering::Greater => {
                        if !encoded_array_is_prefix(&stored.chain_bytes, snapshot.chain_bytes)? {
                            return Err(Error::ChainFork {
                                seqno: stored.chain_seqno.saturating_add(1),
                            });
                        }
                        transaction.execute(
                            "UPDATE hosts SET canonical_name = ?2, genesis_key = ?3, \
                             chain_seqno = ?4, chain_tail_hash = ?5, chain_bytes = ?6, \
                             public_zone_bytes = ?7 \
                             WHERE host_id = ?1",
                            params![
                                snapshot.host_id,
                                snapshot.canonical_name,
                                snapshot.genesis_key,
                                chain_seqno,
                                snapshot.chain_tail_hash.as_slice(),
                                snapshot.chain_bytes,
                                snapshot.public_zone_bytes,
                            ],
                        )?;
                        true
                    }
                }
            }
        };

        transaction.execute(
            "INSERT OR IGNORE INTO host_lookups (lookup_name, host_id) VALUES (?1, ?2)",
            params![snapshot.lookup_name, snapshot.host_id],
        )?;

        if chain_advanced {
            transaction.execute(
                "DELETE FROM host_services WHERE host_id = ?1",
                [&snapshot.host_id],
            )?;
            for service in snapshot.services {
                transaction.execute(
                    "INSERT INTO host_services \
                     (host_id, service_type, endpoint_bytes, valid_at_chain_seqno) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        snapshot.host_id,
                        sqlite_integer("service type", service.service_type.protocol_value())?,
                        service.endpoint_bytes,
                        chain_seqno,
                    ],
                )?;
            }
        }

        let merkle_advanced = accept_merkle_root(
            &transaction,
            snapshot.host_id,
            snapshot.merkle_root,
            merkle_epoch,
        )?;
        transaction.commit()?;

        Ok(if stored.is_none() {
            Acceptance::Inserted
        } else if chain_advanced || merkle_advanced {
            Acceptance::Advanced
        } else {
            Acceptance::Unchanged
        })
    }

    /// Atomically pins a verified user chain, its authenticating Merkle root,
    /// and the public device/PUK projection. No private key material is stored.
    pub fn accept_verified_user(&mut self, snapshot: &VerifiedUserSnapshot) -> Result<Acceptance> {
        let snapshot = snapshot.parts();
        self.accept_user_parts(snapshot)
    }

    fn accept_user_parts(&mut self, snapshot: VerifiedUserSnapshotParts<'_>) -> Result<Acceptance> {
        validate_user_snapshot(snapshot)?;
        let chain_seqno = sqlite_integer("user chain sequence", snapshot.chain_seqno)?;
        let merkle_epoch = sqlite_integer("user Merkle epoch", snapshot.merkle_epoch)?;
        let mut devices = snapshot.devices.to_vec();
        devices.sort();
        let mut shared_keys = snapshot.shared_keys.to_vec();
        shared_keys.sort();
        if devices
            .windows(2)
            .any(|pair| pair[0].device_id == pair[1].device_id)
            || shared_keys
                .windows(2)
                .any(|pair| pair[0].role == pair[1].role)
        {
            return Err(Error::InvalidUser(
                "device IDs and current shared-key roles must be unique",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let host_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&snapshot.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        let accepted_root = transaction
            .query_row(
                "SELECT root_hash FROM merkle_roots \
                 WHERE host_id = ?1 AND epoch = ?2",
                params![snapshot.host_id, merkle_epoch],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if accepted_root.as_deref() != Some(snapshot.merkle_root_hash.as_slice()) {
            return Err(Error::InvalidUser(
                "user projection is not bound to an accepted Merkle root",
            ));
        }
        let stored = transaction
            .query_row(
                "SELECT chain_seqno, chain_tail_hash, chain_bytes, evidence_bytes, username, \
                 username_utf8, username_sequence, merkle_epoch, merkle_root_hash, \
                 merkle_root_bytes FROM users \
                 WHERE host_id = ?1 AND uid = ?2",
                params![snapshot.host_id, snapshot.uid],
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
        let acceptance = match &stored {
            None => Acceptance::Inserted,
            Some((
                stored_seqno,
                tail,
                chain,
                _evidence,
                username,
                username_utf8,
                username_sequence,
                epoch,
                root_hash,
                root_bytes,
            )) => {
                let stored_seqno = stored_unsigned("user chain sequence", *stored_seqno)?;
                let stored_epoch = stored_unsigned("user Merkle epoch", *epoch)?;
                if snapshot.chain_seqno < stored_seqno {
                    return Err(Error::UserRollback {
                        stored: stored_seqno,
                        received: snapshot.chain_seqno,
                    });
                }
                if snapshot.chain_seqno == stored_seqno {
                    if tail.as_slice() != snapshot.chain_tail_hash || chain != snapshot.chain_bytes
                    {
                        return Err(Error::UserFork {
                            seqno: snapshot.chain_seqno,
                        });
                    }
                    if username != snapshot.username
                        || username_utf8 != snapshot.username_utf8
                        || stored_unsigned("username sequence", *username_sequence)?
                            != snapshot.username_sequence
                        || load_user_devices(&transaction, snapshot.host_id, snapshot.uid)?
                            != stored_devices(&devices)
                        || load_user_shared_keys(&transaction, snapshot.host_id, snapshot.uid)?
                            != stored_shared_keys(&shared_keys)
                    {
                        return Err(Error::UserProjectionChanged {
                            seqno: snapshot.chain_seqno,
                        });
                    }
                    match snapshot.merkle_epoch.cmp(&stored_epoch) {
                        std::cmp::Ordering::Less => {
                            return Err(Error::MerkleRollback {
                                stored: stored_epoch,
                                received: snapshot.merkle_epoch,
                            });
                        }
                        std::cmp::Ordering::Equal => {
                            if root_hash.as_slice() != snapshot.merkle_root_hash
                                || root_bytes != snapshot.merkle_root_bytes
                            {
                                return Err(Error::MerkleFork {
                                    epoch: snapshot.merkle_epoch,
                                });
                            }
                            Acceptance::Unchanged
                        }
                        std::cmp::Ordering::Greater => Acceptance::Advanced,
                    }
                } else {
                    if !encoded_array_is_prefix(chain, snapshot.chain_bytes)? {
                        return Err(Error::UserFork {
                            seqno: stored_seqno.saturating_add(1),
                        });
                    }
                    if snapshot.merkle_epoch < stored_epoch {
                        return Err(Error::MerkleRollback {
                            stored: stored_epoch,
                            received: snapshot.merkle_epoch,
                        });
                    }
                    Acceptance::Advanced
                }
            }
        };

        if acceptance != Acceptance::Unchanged {
            transaction.execute(
                "INSERT INTO users (host_id, uid, chain_seqno, chain_tail_hash, chain_bytes, \
                 evidence_bytes, username, username_utf8, username_sequence, merkle_epoch, \
                 merkle_root_hash, merkle_root_bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(host_id, uid) DO UPDATE SET chain_seqno = excluded.chain_seqno, \
                 chain_tail_hash = excluded.chain_tail_hash, chain_bytes = excluded.chain_bytes, \
                 evidence_bytes = excluded.evidence_bytes, username = excluded.username, \
                 username_utf8 = excluded.username_utf8, \
                 username_sequence = excluded.username_sequence, \
                 merkle_epoch = excluded.merkle_epoch, merkle_root_hash = excluded.merkle_root_hash, \
                 merkle_root_bytes = excluded.merkle_root_bytes",
                params![
                    snapshot.host_id,
                    snapshot.uid,
                    chain_seqno,
                    snapshot.chain_tail_hash.as_slice(),
                    snapshot.chain_bytes,
                    snapshot.evidence_bytes,
                    snapshot.username,
                    snapshot.username_utf8,
                    sqlite_integer("username sequence", snapshot.username_sequence)?,
                    merkle_epoch,
                    snapshot.merkle_root_hash.as_slice(),
                    snapshot.merkle_root_bytes,
                ],
            )?;
            transaction.execute(
                "DELETE FROM user_devices WHERE host_id = ?1 AND uid = ?2",
                params![snapshot.host_id, snapshot.uid],
            )?;
            transaction.execute(
                "DELETE FROM user_shared_keys WHERE host_id = ?1 AND uid = ?2",
                params![snapshot.host_id, snapshot.uid],
            )?;
            for device in &devices {
                transaction.execute(
                    "INSERT INTO user_devices \
                     (host_id, uid, device_id, role_type, role_visibility, hepk_bytes, subkey_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        snapshot.host_id,
                        snapshot.uid,
                        device.device_id,
                        sqlite_integer("device role", device.role.protocol_value())?,
                        i64::from(device.role.visibility().unwrap_or(0)),
                        device.hepk_bytes,
                        device.subkey_id,
                    ],
                )?;
            }
            for shared_key in &shared_keys {
                transaction.execute(
                    "INSERT INTO user_shared_keys \
                     (host_id, uid, role_type, role_visibility, generation, verify_key, hepk_bytes) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        snapshot.host_id,
                        snapshot.uid,
                        sqlite_integer("PUK role", shared_key.role.protocol_value())?,
                        i64::from(shared_key.role.visibility().unwrap_or(0)),
                        sqlite_integer("PUK generation", shared_key.generation)?,
                        shared_key.verify_key,
                        shared_key.hepk_bytes,
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(acceptance)
    }

    /// Atomically pins a verified team chain and its public roster/PTK
    /// projection. PTK seeds and other private material are never stored.
    pub fn accept_verified_team(&mut self, snapshot: &VerifiedTeamSnapshot) -> Result<Acceptance> {
        self.accept_team_parts(snapshot.parts())
    }

    fn accept_team_parts(&mut self, snapshot: VerifiedTeamSnapshotParts<'_>) -> Result<Acceptance> {
        validate_team_snapshot(snapshot)?;
        let chain_seqno = sqlite_integer("team chain sequence", snapshot.chain_seqno)?;
        let merkle_epoch = sqlite_integer("team Merkle epoch", snapshot.merkle_epoch)?;
        let mut members = snapshot.members.to_vec();
        members.sort();
        let mut shared_keys = snapshot.shared_keys.to_vec();
        shared_keys.sort();
        if members.windows(2).any(|pair| {
            pair[0].party_id == pair[1].party_id
                && pair[0].scoped_host_id == pair[1].scoped_host_id
                && pair[0].source_role == pair[1].source_role
        }) || shared_keys
            .windows(2)
            .any(|pair| pair[0].role == pair[1].role)
        {
            return Err(Error::InvalidTeam(
                "member identities and current PTK roles must be unique",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let host_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [snapshot.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        let accepted_root = transaction
            .query_row(
                "SELECT root_hash FROM merkle_roots WHERE host_id = ?1 AND epoch = ?2",
                params![snapshot.host_id, merkle_epoch],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if accepted_root.as_deref() != Some(snapshot.merkle_root_hash.as_slice()) {
            return Err(Error::InvalidTeam(
                "team projection is not bound to an accepted Merkle root",
            ));
        }
        let stored = load_team_snapshot(&transaction, snapshot.host_id, snapshot.team_id)?;
        let acceptance = match &stored {
            None => Acceptance::Inserted,
            Some(stored) if snapshot.chain_seqno < stored.chain_seqno => {
                return Err(Error::TeamRollback {
                    stored: stored.chain_seqno,
                    received: snapshot.chain_seqno,
                });
            }
            Some(stored) if snapshot.chain_seqno == stored.chain_seqno => {
                if stored.chain_tail_hash != snapshot.chain_tail_hash
                    || stored.chain_bytes != snapshot.chain_bytes
                {
                    return Err(Error::TeamFork {
                        seqno: snapshot.chain_seqno,
                    });
                }
                if stored.team_name != snapshot.team_name
                    || stored.team_name_utf8 != snapshot.team_name_utf8
                    || stored.team_name_sequence != snapshot.team_name_sequence
                    || stored.members != members
                    || stored.shared_keys != shared_keys
                {
                    return Err(Error::TeamProjectionChanged {
                        seqno: snapshot.chain_seqno,
                    });
                }
                match snapshot.merkle_epoch.cmp(&stored.merkle_epoch) {
                    std::cmp::Ordering::Less => {
                        return Err(Error::MerkleRollback {
                            stored: stored.merkle_epoch,
                            received: snapshot.merkle_epoch,
                        });
                    }
                    std::cmp::Ordering::Equal => {
                        if stored.merkle_root_hash != snapshot.merkle_root_hash
                            || stored.merkle_root_bytes != snapshot.merkle_root_bytes
                        {
                            return Err(Error::MerkleFork {
                                epoch: snapshot.merkle_epoch,
                            });
                        }
                        Acceptance::Unchanged
                    }
                    std::cmp::Ordering::Greater => Acceptance::Advanced,
                }
            }
            Some(stored) => {
                if !encoded_array_is_prefix(&stored.chain_bytes, snapshot.chain_bytes)? {
                    return Err(Error::TeamFork {
                        seqno: stored.chain_seqno.saturating_add(1),
                    });
                }
                if snapshot.merkle_epoch < stored.merkle_epoch {
                    return Err(Error::MerkleRollback {
                        stored: stored.merkle_epoch,
                        received: snapshot.merkle_epoch,
                    });
                }
                Acceptance::Advanced
            }
        };
        if acceptance != Acceptance::Unchanged {
            transaction.execute(
                "INSERT INTO teams (host_id, team_id, chain_seqno, chain_tail_hash, chain_bytes, \
                 evidence_bytes, team_name, team_name_utf8, team_name_sequence, merkle_epoch, \
                 merkle_root_hash, merkle_root_bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(host_id, team_id) DO UPDATE SET chain_seqno = excluded.chain_seqno, \
                 chain_tail_hash = excluded.chain_tail_hash, chain_bytes = excluded.chain_bytes, \
                 evidence_bytes = excluded.evidence_bytes, team_name = excluded.team_name, \
                 team_name_utf8 = excluded.team_name_utf8, \
                 team_name_sequence = excluded.team_name_sequence, \
                 merkle_epoch = excluded.merkle_epoch, merkle_root_hash = excluded.merkle_root_hash, \
                 merkle_root_bytes = excluded.merkle_root_bytes",
                params![
                    snapshot.host_id,
                    snapshot.team_id,
                    chain_seqno,
                    snapshot.chain_tail_hash.as_slice(),
                    snapshot.chain_bytes,
                    snapshot.evidence_bytes,
                    snapshot.team_name,
                    snapshot.team_name_utf8,
                    sqlite_integer("team-name sequence", snapshot.team_name_sequence)?,
                    merkle_epoch,
                    snapshot.merkle_root_hash.as_slice(),
                    snapshot.merkle_root_bytes,
                ],
            )?;
            transaction.execute(
                "DELETE FROM team_members WHERE host_id = ?1 AND team_id = ?2",
                params![snapshot.host_id, snapshot.team_id],
            )?;
            transaction.execute(
                "DELETE FROM team_shared_keys WHERE host_id = ?1 AND team_id = ?2",
                params![snapshot.host_id, snapshot.team_id],
            )?;
            for member in &members {
                transaction.execute(
                    "INSERT INTO team_members (host_id, team_id, party_id, scoped_host_id, \
                     source_role_type, source_role_visibility, role_type, role_visibility, \
                     generation, verify_key, hepk_fingerprint, removal_key_commitment) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        snapshot.host_id,
                        snapshot.team_id,
                        member.party_id,
                        member.scoped_host_id.as_deref().unwrap_or_default(),
                        sqlite_integer(
                            "team member source role",
                            member.source_role.protocol_value()
                        )?,
                        i64::from(member.source_role.visibility().unwrap_or(0)),
                        sqlite_integer("team member role", member.role.protocol_value())?,
                        i64::from(member.role.visibility().unwrap_or(0)),
                        sqlite_integer("team member generation", member.generation)?,
                        member.verify_key,
                        member.hepk_fingerprint.as_slice(),
                        member
                            .removal_key_commitment
                            .as_ref()
                            .map_or(&[][..], |commitment| commitment.as_slice()),
                    ],
                )?;
            }
            for key in &shared_keys {
                transaction.execute(
                    "INSERT INTO team_shared_keys (host_id, team_id, role_type, role_visibility, \
                     generation, verify_key, hepk_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        snapshot.host_id,
                        snapshot.team_id,
                        sqlite_integer("PTK role", key.role.protocol_value())?,
                        i64::from(key.role.visibility().unwrap_or(0)),
                        sqlite_integer("PTK generation", key.generation)?,
                        key.verify_key,
                        key.hepk_bytes,
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(acceptance)
    }

    /// Advances only the already-verified Merkle head for a pinned host.
    pub fn accept_verified_merkle_root(
        &mut self,
        host_id: &[u8],
        root: &VerifiedMerkleRoot,
    ) -> Result<Acceptance> {
        let root = root.parts();
        let epoch = sqlite_integer("Merkle epoch", root.epoch)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let host_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        let advanced = accept_merkle_root(&transaction, host_id, root, epoch)?;
        transaction.commit()?;
        Ok(if advanced {
            Acceptance::Advanced
        } else {
            Acceptance::Unchanged
        })
    }

    /// Loads the accepted projection for a discovery name.
    pub fn host_for_lookup(&self, lookup_name: &str) -> Result<Option<StoredHostSnapshot>> {
        let host_id = self
            .connection
            .query_row(
                "SELECT host_id FROM host_lookups WHERE lookup_name = ?1",
                [lookup_name],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        host_id
            .map(|host_id| load_snapshot(&self.connection, lookup_name, &host_id))
            .transpose()
    }

    /// Loads an untrusted persisted user projection. Callers must pass its
    /// `parts()` through `foks_verify::restore_verified_user` before use.
    pub fn user_for_host(&self, host_id: &[u8], uid: &[u8]) -> Result<Option<StoredUserSnapshot>> {
        load_user_snapshot(&self.connection, host_id, uid)
    }

    /// Loads an untrusted persisted team projection. Re-authenticate it with
    /// `foks_verify::restore_verified_team` before use.
    pub fn team_for_host(
        &self,
        host_id: &[u8],
        team_id: &[u8],
    ) -> Result<Option<StoredTeamSnapshot>> {
        load_team_snapshot(&self.connection, host_id, team_id)
    }
}

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
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA)?;
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
        || !matches!(operation.scope_id.len(), 0 | 16 | 33)
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
    if job.host_id.len() != 33
        || !matches!(job.scope_id.len(), 0 | 16 | 33 | 34)
        || job.interval_micros == 0
    {
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
             team_name_utf8, team_name_sequence, merkle_epoch, merkle_root_hash, \
             merkle_root_bytes FROM teams WHERE host_id = ?1 AND team_id = ?2",
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
        team_name,
        team_name_utf8,
        team_name_sequence,
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
         role_visibility, generation, verify_key, hepk_fingerprint, \
         removal_key_commitment FROM team_members \
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
            "SELECT epoch, root_hash FROM merkle_heads WHERE host_id = ?1",
            [host_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    if let Some((stored_epoch, stored_hash)) = stored {
        let stored_epoch = stored_unsigned("Merkle epoch", stored_epoch)?;
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
                if stored_hash != root.root_hash
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
    let (evidence_kind, anchor_epoch, evidence_bytes) = encode_evidence(root.evidence)?;
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
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            ..
        } if *anchor_epoch >= root.epoch || historical_response.is_empty() => Err(
            Error::InvalidSnapshot("Merkle skip-path evidence is malformed"),
        ),
        MerkleRootEvidence::SkipPath { prior, .. } => validate_evidence_order(prior, root.epoch),
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
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            prior,
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
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            prior,
        } => Value::Array(vec![
            Value::Unsigned(1),
            Value::Unsigned(*anchor_epoch),
            Value::Binary(historical_response.clone()),
            evidence_value(prior),
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
        [Value::Unsigned(1), Value::Unsigned(anchor_epoch), Value::Binary(historical_response), prior] => {
            Ok(MerkleRootEvidence::SkipPath {
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
            .advance_mutation(&[6; 16], MutationState::Verified, 104)
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
                TeamMutationState::Submitted,
                302,
            )
            .unwrap();
        store
            .advance_team_mutation(
                &duplicate_transition.operation_id,
                TeamMutationState::Verified,
                303,
            )
            .unwrap();
        let stale_third = TeamMutationOperation {
            operation_id: [3; 16],
            state: TeamMutationState::Prepared,
            ..duplicate_transition
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
            .advance_team_mutation(&superseded.operation_id, TeamMutationState::Submitted, 401)
            .unwrap();
        store
            .advance_team_mutation(&superseded.operation_id, TeamMutationState::Superseded, 402)
            .unwrap();
        assert!(store
            .advance_team_mutation(&superseded.operation_id, TeamMutationState::Verified, 403,)
            .is_err());
        let replacement = TeamMutationOperation {
            operation_id: [1; 16],
            state: TeamMutationState::Prepared,
            ..superseded
        };
        store.record_team_mutation(&replacement).unwrap();
    }

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
            merkle_epoch: 11,
            merkle_root_hash: [7; 32],
            merkle_root_bytes: vec![9; 80],
            members: vec![VerifiedTeamMember {
                party_id: vec![1; 33],
                scoped_host_id: None,
                source_role: foks_proto::Role::OWNER,
                role: foks_proto::Role::OWNER,
                generation: 2,
                verify_key: vec![14; 33],
                hepk_fingerprint: [33; 32],
                removal_key_commitment: Some([36; 32]),
            }],
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
    fn projection_cannot_change_without_chain_advance() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut changed = snapshot();
        changed.services[0].endpoint_bytes = vec![12; 20];
        assert!(matches!(
            store.accept_host_parts(changed.parts()),
            Err(Error::ProjectionChanged { seqno: 4 })
        ));
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
            Err(Error::InvalidUser(_))
        ));

        let mut changed_device = user;
        changed_device.devices[0].hepk_bytes[0] ^= 1;
        assert!(matches!(
            store.accept_user_parts(changed_device.parts()),
            Err(Error::UserProjectionChanged { seqno: 1 })
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
        assert_eq!(loaded.members, team.members);
        assert_eq!(loaded.shared_keys, team.shared_keys);

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
