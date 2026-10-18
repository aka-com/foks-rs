//! Catalog loading, item reads, and agent operation builders shared by the
//! FOKS desktop command layer and the scripted `foks-desktop-backend`.

#![forbid(unsafe_code)]

mod chat;
pub mod ops;
pub use chat::{chat_request, chat_request_cancellable, validate_chat_reply};
mod deadline;
pub use deadline::{
    bind_transport_deadline, discard_transport_result, local_deadline_error, PendingOperation,
};

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::{fs, path::PathBuf};

use foks_agent_client::AgentClient;
use foks_agent_proto::{
    AccountStoreRef, AccountSummary, ErrorCode, ErrorFields, KnownStoreSummary, KvChunkResult,
    KvEntryMetadata, KvPage, KvPrecondition, KvReadResult, KvRole, KvStoreRef, KvUploadHeader,
    Operation, ProfileOverview, ResponseResult, ServerStatusSnapshot, TeamKind, TeamStoreRef,
    TeamSummary,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeroize::{Zeroize as _, Zeroizing};

// The size above which an edit is uploaded as a stream rather than inline.
// The agent now accepts a larger inline put than this, so this is a choice
// about where streaming starts, not a frame bound; it stays where it was.
pub const MAXIMUM_INLINE_KV_BYTES: usize = 128 * 1024;
// Byte payloads are base64 on the local protocol, so a payload's frame cost
// is `ceil(n/3)*4` and the protocol's own bound is what fits the frame. The
// agent refuses a longer chunk request as invalid, so asking for exactly its
// bound is the fewest round trips a download can take.
const KV_READ_CHUNK_BYTES: u32 = foks_agent_proto::MAXIMUM_KV_PAYLOAD_BYTES as u32;
const MAXIMUM_DESKTOP_REVEAL_BYTES: u64 = 16 * 1024 * 1024;

/// Typed desktop-side classification of local-agent failures.
///
/// Desktop error variants mapped from local agent and transport failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentError {
    Protocol {
        code: ErrorCode,
        message: String,
        fields: Box<ErrorFields>,
    },
    Transport(String),
    Local(LocalAgentCondition),
    Ambiguous(String),
    Cancelled,
    DeadlineExceeded,
    Ipc {
        code: IpcErrorCode,
        message: String,
        ambiguous: bool,
        connection_lost: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcErrorCode {
    Unsupported,
    UnsafeSocket,
    Io,
    UploadSource,
    Protocol,
    ResponseBinding,
    VersionMismatch,
    Cancelled,
    DeadlineExceeded,
    DeadlineClock,
    Ambiguous,
}

impl IpcErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::UnsafeSocket => "unsafe-socket",
            Self::Io => "io",
            Self::UploadSource => "upload-source",
            Self::Protocol => "protocol",
            Self::ResponseBinding => "response-binding",
            Self::VersionMismatch => "version-mismatch",
            Self::Cancelled => "cancelled",
            Self::DeadlineExceeded => "deadline-exceeded",
            Self::DeadlineClock => "deadline-clock",
            Self::Ambiguous => "ambiguous",
        }
    }

    pub const fn retryable(self) -> bool {
        matches!(self, Self::Cancelled | Self::DeadlineExceeded)
    }

    pub const fn security_failure(self) -> bool {
        matches!(
            self,
            Self::UnsafeSocket | Self::Protocol | Self::ResponseBinding | Self::VersionMismatch
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAgentCondition {
    Maintenance,
    RestartRequired,
    RecoveryRequired,
    RestorationFailed,
}

impl AgentError {
    /// Reports a transient condition, not permission to retry a mutation.
    /// Callers must reconcile an ambiguous mutation or use its explicit
    /// resume operation before issuing it again.
    pub fn transient(&self) -> bool {
        if let Self::Ipc {
            code,
            connection_lost,
            ..
        } = self
        {
            return !code.security_failure()
                && (*connection_lost || code.retryable() || *code == IpcErrorCode::Ambiguous);
        }
        matches!(
            self,
            Self::Protocol {
                code: ErrorCode::Busy
                    | ErrorCode::DeadlineExceeded
                    | ErrorCode::ProfileBusy
                    | ErrorCode::RateLimited,
                ..
            } | Self::Transport(_)
                | Self::Ambiguous(_)
                | Self::Cancelled
                | Self::DeadlineExceeded
        )
    }

    pub fn fatal(&self) -> bool {
        matches!(
            self,
            Self::Protocol {
                code: ErrorCode::VersionMismatch,
                ..
            }
        ) || matches!(self, Self::Ipc { code, .. } if code.security_failure())
    }

    pub fn ambiguous(&self) -> bool {
        matches!(
            self,
            Self::Protocol {
                code: ErrorCode::DeadlineExceeded,
                ..
            } | Self::Ambiguous(_)
                | Self::Ipc {
                    ambiguous: true,
                    ..
                }
                | Self::Ipc {
                    code: IpcErrorCode::Ambiguous,
                    ..
                }
        )
    }

    pub fn connection_lost(&self) -> bool {
        matches!(
            self,
            Self::Transport(_)
                | Self::Ipc {
                    connection_lost: true,
                    ..
                }
        )
    }

    pub fn user_message(&self) -> &str {
        match self {
            Self::Protocol { message, .. }
            | Self::Transport(message)
            | Self::Ambiguous(message)
            | Self::Ipc { message, .. } => message,
            Self::Local(condition) => match condition {
                LocalAgentCondition::Maintenance => "State maintenance is in progress.",
                LocalAgentCondition::RestartRequired => {
                    "The selected state root requires an application restart."
                }
                LocalAgentCondition::RecoveryRequired => {
                    "Client state recovery is required before the agent can restart."
                }
                LocalAgentCondition::RestorationFailed => {
                    "The local agent could not be restored after state maintenance."
                }
            },
            Self::Cancelled => "Request cancelled.",
            Self::DeadlineExceeded => "Request deadline exceeded.",
        }
    }
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol { code, message, .. } => {
                write!(formatter, "agent returned {code:?}: {message}")?;
                if *code == ErrorCode::DeadlineExceeded {
                    formatter.write_str(
                        "\n\nThe operation may still have completed. Refresh this screen \
                         (or run the matching resume action) before retrying it.",
                    )?;
                }
                Ok(())
            }
            Self::Transport(message) | Self::Ipc { message, .. } => formatter.write_str(message),
            Self::Local(condition) => formatter.write_str(match condition {
                LocalAgentCondition::Maintenance => "state maintenance is in progress",
                LocalAgentCondition::RestartRequired => "application restart is required",
                LocalAgentCondition::RecoveryRequired => "client state recovery is required",
                LocalAgentCondition::RestorationFailed => {
                    "the local agent could not be restored after state maintenance"
                }
            }),
            Self::Ambiguous(message) => write!(
                formatter,
                "{message}\n\nThe upload commit may have completed. The store will be refreshed before another mutation."
            ),
            Self::Cancelled => formatter.write_str("Request cancelled"),
            Self::DeadlineExceeded => formatter.write_str("Request deadline exceeded"),
        }
    }
}

impl std::error::Error for AgentError {}

pub trait AgentTransport: Send + Sync + 'static {
    fn call(&self, operation: Operation) -> Result<Value, AgentError>;
    /// A supplied operation context must reach the concrete IPC transport.
    /// Unsupported adapters fail closed instead of silently resetting its cap.
    fn call_with_deadline(
        &self,
        mut operation: Operation,
        _deadline: &foks_local_deadline::Deadline,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, AgentError> {
        operation.zeroize_plaintext();
        Err(deadline::unsupported())
    }

    fn put_kv_stream_with_deadline(
        &self,
        _header: KvUploadHeader,
        _reader: &mut dyn std::io::Read,
        _deadline: &foks_local_deadline::Deadline,
    ) -> Result<Value, AgentError> {
        Err(deadline::unsupported())
    }
    fn call_cancellable(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, AgentError> {
        let mutation = operation.is_mutation();
        let mut operation = PendingOperation::new(operation);
        if cancelled() {
            return Err(AgentError::Cancelled);
        }
        let mut result = self.call(operation.take());
        if cancelled() {
            if deadline::preserve_terminal_transport_error(&mut result) {
                return result;
            }
            discard_transport_result(&mut result);
            return Err(if mutation {
                AgentError::Ambiguous("Request cancelled after mutation started.".into())
            } else {
                AgentError::Cancelled
            });
        }
        result
    }

    fn put_kv_stream(
        &self,
        _header: KvUploadHeader,
        _reader: &mut dyn std::io::Read,
    ) -> Result<Value, AgentError> {
        Err(AgentError::Transport(
            "this agent transport does not support streaming uploads".to_owned(),
        ))
    }
}

impl AgentTransport for AgentClient {
    fn call_with_deadline(
        &self,
        operation: Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, AgentError> {
        let mut operation = PendingOperation::new(operation);
        let client = self
            .with_local_deadline(deadline.clone())
            .map_err(agent_client_error)?;
        <Self as AgentTransport>::call_cancellable(&client, operation.take(), cancelled)
    }

    fn put_kv_stream_with_deadline(
        &self,
        header: KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<Value, AgentError> {
        let client = self
            .with_local_deadline(deadline.clone())
            .map_err(agent_client_error)?;
        <Self as AgentTransport>::put_kv_stream(&client, header, reader)
    }

    fn call(&self, operation: Operation) -> Result<Value, AgentError> {
        <Self as AgentTransport>::call_cancellable(self, operation, &|| false)
    }

    fn call_cancellable(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, AgentError> {
        let response = AgentClient::call_cancellable(self, operation, cancelled)
            .map_err(agent_client_error)?;
        match response.result {
            ResponseResult::Success { value } => Ok(value),
            ResponseResult::Error {
                code,
                message,
                fields,
            } => Err(AgentError::Protocol {
                code,
                message,
                fields: fields.into(),
            }),
        }
    }

    fn put_kv_stream(
        &self,
        header: KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<Value, AgentError> {
        let response = self
            .put_kv_stream(header, reader)
            .map_err(agent_client_error)?;
        match response.result {
            ResponseResult::Success { value } => Ok(value),
            ResponseResult::Error {
                code,
                message,
                fields,
            } => Err(AgentError::Protocol {
                code,
                message,
                fields: fields.into(),
            }),
        }
    }
}

pub fn agent_client_error(error: impl std::borrow::Borrow<foks_agent_client::Error>) -> AgentError {
    use foks_agent_client::Error;
    let error = error.borrow();
    let connection_lost = error.is_connection_loss();
    let message = error.to_string();
    let ipc = |code| AgentError::Ipc {
        code,
        message: message.clone(),
        ambiguous: false,
        connection_lost,
    };
    match error {
        Error::Cancelled => AgentError::Cancelled,
        Error::DeadlineExceeded => AgentError::DeadlineExceeded,
        Error::LocalDeadline(_) => ipc(IpcErrorCode::DeadlineClock),
        Error::Ambiguous(cause) => {
            let cause = agent_client_error(cause.as_ref());
            let code = match &cause {
                AgentError::Ipc { code, .. } => *code,
                AgentError::Protocol {
                    code: ErrorCode::VersionMismatch,
                    ..
                } => IpcErrorCode::VersionMismatch,
                AgentError::Transport(_) => IpcErrorCode::Io,
                AgentError::Cancelled => IpcErrorCode::Cancelled,
                AgentError::DeadlineExceeded => IpcErrorCode::DeadlineExceeded,
                _ => IpcErrorCode::Ambiguous,
            };
            AgentError::Ipc {
                code,
                message,
                ambiguous: true,
                connection_lost,
            }
        }
        Error::Protocol(foks_agent_proto::Error::Version) => AgentError::Protocol {
            code: ErrorCode::VersionMismatch,
            message: "desktop and agent protocol versions do not match".to_owned(),
            fields: Box::default(),
        },
        // The agent refused the request before decoding it, so this is
        // definite even for a mutation.
        Error::Protocol(foks_agent_proto::Error::VersionMismatch { agent, supported }) => {
            let accepted = match supported {
                Some(range) if *range != foks_agent_proto::ProtocolRange::exactly(*agent) => {
                    format!(" (accepted: {range})")
                }
                _ => String::new(),
            };
            AgentError::Protocol {
                code: ErrorCode::VersionMismatch,
                message: format!(
                    "desktop protocol version {} does not match agent protocol version {agent}{accepted}",
                    foks_agent_proto::PROTOCOL_VERSION
                ),
                fields: Box::default(),
            }
        }
        Error::Io(_) if connection_lost => AgentError::Transport(message),
        Error::Io(_) => ipc(IpcErrorCode::Io),
        Error::UploadSource(_) => ipc(IpcErrorCode::UploadSource),
        Error::UnsafeSocket => ipc(IpcErrorCode::UnsafeSocket),
        Error::Protocol(_) => ipc(IpcErrorCode::Protocol),
        Error::ResponseBinding => ipc(IpcErrorCode::ResponseBinding),
        Error::Unsupported => ipc(IpcErrorCode::Unsupported),
    }
}

#[cfg(test)]
mod ipc_error_tests {
    use super::*;

    #[test]
    fn protocol_errors_remain_small_and_preserve_fields_and_classification() {
        assert!(std::mem::size_of::<AgentError>() < 128);
        let fields = ErrorFields {
            capability: Some("kv".into()),
            profile: Some("work.example".into()),
            state_dir: Some("/state/work".into()),
            reason: Some("schema mismatch".into()),
            found_schema: Some(3),
            supported_schema: Some(2),
            minimum_protocol: Some(35),
            maximum_protocol: Some(36),
        };
        for (code, transient, ambiguous, fatal) in [
            (ErrorCode::DeadlineExceeded, true, true, false),
            (ErrorCode::VersionMismatch, false, false, true),
            (ErrorCode::CapabilityDenied, false, false, false),
        ] {
            let error = response_value(ResponseResult::Error {
                code,
                message: "original message".into(),
                fields: fields.clone(),
            })
            .unwrap_err();
            assert_eq!(error.transient(), transient);
            assert_eq!(error.ambiguous(), ambiguous);
            assert_eq!(error.fatal(), fatal);
            assert!(!error.connection_lost());
            assert_eq!(error.user_message(), "original message");
            let AgentError::Protocol {
                code: actual_code,
                fields: actual_fields,
                ..
            } = error
            else {
                panic!("expected a protocol error");
            };
            assert_eq!(actual_code, code);
            assert_eq!(*actual_fields, fields);
        }
    }

    #[test]
    fn client_cancellation_and_local_deadline_are_not_agent_loss() {
        for error in [
            foks_agent_client::Error::Cancelled,
            foks_agent_client::Error::DeadlineExceeded,
        ] {
            let mapped = agent_client_error(error);
            assert!(!mapped.connection_lost());
            assert!(!mapped.fatal());
            assert!(!mapped.ambiguous());
            assert!(matches!(
                mapped,
                AgentError::Cancelled | AgentError::DeadlineExceeded
            ));
        }
    }

    #[test]
    fn malformed_catalog_payload_is_not_a_connection_loss() {
        let error =
            decode_agent_value::<Vec<ProfileSummary>>(serde_json::json!({"unexpected":true}))
                .err()
                .unwrap();
        assert!(error.fatal());
        assert!(!error.connection_lost());
        assert!(!error.ambiguous());
    }

    #[test]
    fn default_adapter_does_not_hide_security_failures_behind_cancellation() {
        struct Failed(std::sync::atomic::AtomicBool);
        impl AgentTransport for Failed {
            fn call(&self, _: Operation) -> Result<Value, AgentError> {
                self.0.store(true, std::sync::atomic::Ordering::Release);
                Err(agent_client_error(foks_agent_client::Error::UnsafeSocket))
            }
        }
        let transport = Failed(std::sync::atomic::AtomicBool::new(false));
        let error = transport
            .call_cancellable(Operation::Ping, &|| {
                transport.0.load(std::sync::atomic::Ordering::Acquire)
            })
            .unwrap_err();
        assert!(error.fatal());
        assert!(!error.connection_lost());
        assert!(matches!(
            error,
            AgentError::Ipc {
                code: IpcErrorCode::UnsafeSocket,
                ..
            }
        ));
    }
}

fn response_value(result: ResponseResult) -> Result<Value, AgentError> {
    match result {
        ResponseResult::Success { value } => Ok(value),
        ResponseResult::Error {
            code,
            message,
            fields,
        } => Err(AgentError::Protocol {
            code,
            message,
            fields: fields.into(),
        }),
    }
}

/// Strongly typed response models deserialized from background service payloads.
#[derive(serde::Deserialize)]
struct ProfileSummary {
    name: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum CatalogStoreRef {
    Account(AccountStoreRef),
    Team(TeamStoreRef),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogStoreSummary {
    Account {
        store: AccountStoreRef,
    },
    Team {
        store: TeamStoreRef,
        kind: String,
        name: Option<String>,
        active: bool,
        creation_phase: Option<String>,
        /// The team chain sequence the agent has pinned locally, when it
        /// reported one. A reader uses it only to notice that a team has not
        /// changed; `None` means unknown, never unchanged.
        chain_seqno: Option<u64>,
    },
}

impl CatalogStoreSummary {
    pub fn store_ref(&self) -> CatalogStoreRef {
        match self {
            Self::Account { store } => CatalogStoreRef::Account(store.clone()),
            Self::Team { store, .. } => CatalogStoreRef::Team(store.clone()),
        }
    }

    pub fn profile(&self) -> &str {
        match self {
            Self::Account { store } => &store.profile,
            Self::Team { store, .. } => &store.profile,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Account { store } => &store.account_alias,
            Self::Team { store, name, .. } => name.as_deref().unwrap_or(&store.team_alias),
        }
    }
}

impl CatalogStoreRef {
    pub fn profile(&self) -> &str {
        match self {
            Self::Account(store) => &store.profile,
            Self::Team(store) => &store.profile,
        }
    }
}

impl CatalogSnapshot {
    pub fn profile_blocked(&self, profile: &str) -> bool {
        self.blocked_profiles
            .iter()
            .any(|blocked| blocked == profile)
    }

    pub fn store_blocked(&self, store: &CatalogStoreRef) -> bool {
        self.profile_blocked(store.profile())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogItem {
    pub store: CatalogStoreRef,
    pub metadata: KvEntryMetadata,
}

#[derive(Debug)]
pub enum KvItemValue {
    File(Zeroizing<Vec<u8>>),
    Symlink(Zeroizing<String>),
    Directory,
}

#[derive(Debug)]
pub struct KvItemRead {
    pub store: CatalogStoreRef,
    pub path: String,
    pub version: u64,
    pub read_role: KvRole,
    pub write_role: KvRole,
    pub value: KvItemValue,
}

pub enum KvAccountMutation {
    Inline(Operation),
    Stream {
        header: KvUploadHeader,
        content: Zeroizing<Vec<u8>>,
    },
}

impl std::fmt::Debug for KvAccountMutation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inline(operation) => formatter.debug_tuple("Inline").field(operation).finish(),
            Self::Stream { header, content } => formatter
                .debug_struct("Stream")
                .field("header", header)
                .field(
                    "content",
                    &format_args!("<redacted; {} bytes>", content.len()),
                )
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogFailureScope {
    Profile { profile: String, source: String },
    Store(CatalogStoreRef),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogFailure {
    pub scope: CatalogFailureScope,
    pub error: AgentError,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogInventoryState {
    pub profile: String,
    pub accounts_complete: bool,
    pub teams_complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogStoreReadState {
    NotLoaded,
    Complete,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogStoreRead {
    pub store: CatalogStoreRef,
    pub state: CatalogStoreReadState,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogSnapshot {
    pub profiles: Vec<String>,
    pub stores: Vec<CatalogStoreSummary>,
    pub known_stores: Vec<CatalogStoreSummary>,
    pub inventory: Vec<CatalogInventoryState>,
    pub profile_overviews: Vec<ProfileOverview>,
    pub items: Vec<CatalogItem>,
    pub store_reads: Vec<CatalogStoreRead>,
    pub full_item_reads: Option<Vec<String>>,
    pub failures: Vec<CatalogFailure>,
    pub blocked_profiles: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CatalogLoadToken(Arc<AtomicBool>);

impl CatalogLoadToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn check(&self) -> Result<(), AgentError> {
        if self.is_cancelled() {
            Err(AgentError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Loads one live item catalog. Calls are sequential inside each FOKS profile
/// because native checkpoint/session access is exclusive; up to four distinct
/// profiles are processed concurrently.
pub fn load_catalog(transport: Arc<dyn AgentTransport>) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_with_token(transport, true, false, CatalogLoadToken::default())
}

/// Loads account and team discovery without walking any KV tree.
pub fn load_stores(transport: Arc<dyn AgentTransport>) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_with_token(transport, false, false, CatalogLoadToken::default())
}

pub fn load_catalog_cancellable(
    transport: Arc<dyn AgentTransport>,
    token: CatalogLoadToken,
    fresh: bool,
) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_with_token(transport, true, fresh, token)
}

/// The whole catalog, for a caller that has already listed the configured
/// profiles. The desktop backend decodes that listing into its own richer
/// summary for the metadata it attaches to the result, so taking it here keeps
/// one `ListProfiles` per catalog read instead of two.
pub fn load_catalog_cancellable_with_profiles(
    transport: Arc<dyn AgentTransport>,
    profiles: Vec<String>,
    token: CatalogLoadToken,
    fresh: bool,
) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_profiles(transport, profiles, true, fresh, token, &|_| {})
}

pub fn load_profile_catalog_cancellable(
    transport: Arc<dyn AgentTransport>,
    profile: String,
    token: CatalogLoadToken,
    fresh: bool,
) -> Result<CatalogSnapshot, AgentError> {
    token.check()?;
    let profiles: Vec<ProfileSummary> = decode_agent_value(
        transport.call_cancellable(Operation::ListProfiles, &|| token.is_cancelled())?,
    )?;
    let profiles = profiles
        .into_iter()
        .map(|profile| profile.name)
        .collect::<Vec<_>>();
    load_profile_catalog_with_profiles(transport, profile, &profiles, token, fresh)
}

/// One profile's catalog, for a caller that has already listed the configured
/// profiles. The desktop backend decodes that listing into its own richer
/// summary for the same publish, so taking it here keeps one `ListProfiles`
/// per catalog read instead of two.
pub fn load_profile_catalog_with_profiles(
    transport: Arc<dyn AgentTransport>,
    profile: String,
    profiles: &[String],
    token: CatalogLoadToken,
    fresh: bool,
) -> Result<CatalogSnapshot, AgentError> {
    token.check()?;
    if profiles
        .iter()
        .filter(|candidate| **candidate == profile)
        .count()
        != 1
    {
        return Err(AgentError::Transport(
            "profile is missing or duplicated in the configured profiles".to_owned(),
        ));
    }
    let mut snapshot =
        load_profile_catalog(transport.as_ref(), profile.clone(), true, fresh, token)?;
    snapshot.profiles.push(profile);
    Ok(snapshot)
}

pub fn load_stores_cancellable(
    transport: Arc<dyn AgentTransport>,
    token: CatalogLoadToken,
) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_with_token(transport, false, false, token)
}

fn load_catalog_with_token(
    transport: Arc<dyn AgentTransport>,
    include_items: bool,
    fresh: bool,
    token: CatalogLoadToken,
) -> Result<CatalogSnapshot, AgentError> {
    token.check()?;
    let profiles: Vec<ProfileSummary> =
        decode_agent_value(transport.call(Operation::ListProfiles)?)?;
    token.check()?;
    let profiles = profiles
        .into_iter()
        .map(|profile| profile.name)
        .collect::<Vec<_>>();
    load_catalog_profiles(transport, profiles, include_items, fresh, token, &|_| {})
}

pub fn load_catalog_progressive_cancellable(
    transport: Arc<dyn AgentTransport>,
    token: CatalogLoadToken,
    fresh: bool,
    publish: impl Fn(&CatalogSnapshot) + Sync,
) -> Result<CatalogSnapshot, AgentError> {
    token.check()?;
    let profiles: Vec<ProfileSummary> = decode_agent_value(
        transport.call_cancellable(Operation::ListProfiles, &|| token.check().is_err())?,
    )?;
    load_catalog_progressive_with_profiles(
        transport,
        profiles.into_iter().map(|profile| profile.name).collect(),
        token,
        fresh,
        publish,
    )
}

/// As [`load_catalog_progressive_cancellable`], for a caller that has already
/// listed the configured profiles.
pub fn load_catalog_progressive_with_profiles(
    transport: Arc<dyn AgentTransport>,
    profiles: Vec<String>,
    token: CatalogLoadToken,
    fresh: bool,
    publish: impl Fn(&CatalogSnapshot) + Sync,
) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_profiles(transport, profiles, true, fresh, token, &publish)
}

fn load_catalog_profiles(
    transport: Arc<dyn AgentTransport>,
    profiles: Vec<String>,
    include_items: bool,
    fresh: bool,
    token: CatalogLoadToken,
    publish: &(impl Fn(&CatalogSnapshot) + Sync),
) -> Result<CatalogSnapshot, AgentError> {
    let snapshot = Mutex::new(CatalogSnapshot {
        profiles: profiles.clone(),
        ..CatalogSnapshot::default()
    });
    let update = |profile: &str, loaded: &CatalogSnapshot| {
        let mut snapshot = snapshot.lock().expect("catalog progress poisoned");
        if token.check().is_err() {
            return;
        }
        snapshot.stores.retain(|store| store.profile() != profile);
        snapshot
            .known_stores
            .retain(|store| store.profile() != profile);
        snapshot.inventory.retain(|entry| entry.profile != profile);
        snapshot
            .profile_overviews
            .retain(|entry| entry.profile != profile);
        snapshot
            .items
            .retain(|item| item.store.profile() != profile);
        snapshot.failures.retain(|failure| match &failure.scope {
            CatalogFailureScope::Profile {
                profile: candidate, ..
            } => candidate != profile,
            CatalogFailureScope::Store(store) => store.profile() != profile,
        });
        snapshot
            .blocked_profiles
            .retain(|candidate| candidate != profile);
        snapshot.stores.extend(loaded.stores.clone());
        snapshot.known_stores.extend(loaded.known_stores.clone());
        snapshot.inventory.extend(loaded.inventory.clone());
        snapshot
            .profile_overviews
            .extend(loaded.profile_overviews.clone());
        snapshot.items.extend(loaded.items.clone());
        snapshot
            .store_reads
            .retain(|read| read.store.profile() != profile);
        snapshot.store_reads.extend(loaded.store_reads.clone());
        if include_items && !loaded.inventory.is_empty() {
            snapshot.full_item_reads.get_or_insert_with(Vec::new);
        }
        if let Some(full_item_reads) = &mut snapshot.full_item_reads {
            full_item_reads.retain(|candidate| candidate != profile);
            full_item_reads.extend(loaded.full_item_reads.iter().flatten().cloned());
        }
        snapshot.failures.extend(loaded.failures.clone());
        snapshot
            .blocked_profiles
            .extend(loaded.blocked_profiles.clone());
        sort_catalog(&mut snapshot);
        publish(&snapshot);
    };
    let loaded = run_bounded(profiles, |profile| {
        token.check()?;
        let loaded = load_profile_catalog_progress(
            transport.as_ref(),
            profile.clone(),
            include_items,
            fresh,
            token.clone(),
            &|snapshot| update(&profile, snapshot),
        )?;
        token.check()?;
        update(&profile, &loaded);
        Ok::<_, AgentError>(())
    });
    for loaded in loaded {
        loaded?;
    }
    token.check()?;
    let mut snapshot = snapshot.into_inner().expect("catalog progress poisoned");
    if include_items {
        snapshot.full_item_reads.get_or_insert_with(Vec::new);
    }
    Ok(snapshot)
}

fn sort_catalog(snapshot: &mut CatalogSnapshot) {
    snapshot.stores.sort_by(|left, right| {
        left.profile()
            .cmp(right.profile())
            .then(left.label().cmp(right.label()))
    });
    snapshot.known_stores.sort_by(|left, right| {
        left.profile()
            .cmp(right.profile())
            .then(left.label().cmp(right.label()))
    });
    snapshot
        .inventory
        .sort_by(|left, right| left.profile.cmp(&right.profile));
    snapshot.items.sort_by(|left, right| {
        left.store
            .profile()
            .cmp(right.store.profile())
            .then(left.metadata.path.cmp(&right.metadata.path))
    });
    snapshot.blocked_profiles.sort();
    snapshot.blocked_profiles.dedup();
}

fn load_profile_catalog(
    transport: &dyn AgentTransport,
    profile: String,
    include_items: bool,
    fresh: bool,
    token: CatalogLoadToken,
) -> Result<CatalogSnapshot, AgentError> {
    load_profile_catalog_progress(transport, profile, include_items, fresh, token, &|_| {})
}

fn load_profile_catalog_progress(
    transport: &dyn AgentTransport,
    profile: String,
    include_items: bool,
    fresh: bool,
    token: CatalogLoadToken,
    publish_local: &dyn Fn(&CatalogSnapshot),
) -> Result<CatalogSnapshot, AgentError> {
    token.check()?;
    let mut snapshot = CatalogSnapshot::default();
    match transport.call_cancellable(
        Operation::ListKnownStores {
            profile: profile.clone(),
        },
        &|| token.check().is_err(),
    ) {
        Ok(value) => match decode_agent_value::<Vec<KnownStoreSummary>>(value) {
            Ok(stores) => {
                snapshot.known_stores = stores
                    .into_iter()
                    .map(|store| known_catalog_store(&profile, store))
                    .collect();
            }
            Err(error) => snapshot.failures.push(CatalogFailure {
                scope: CatalogFailureScope::Profile {
                    profile: profile.clone(),
                    source: "known store index".to_owned(),
                },
                error,
            }),
        },
        Err(error) => snapshot.failures.push(CatalogFailure {
            scope: CatalogFailureScope::Profile {
                profile: profile.clone(),
                source: "known store index".to_owned(),
            },
            error,
        }),
    }
    snapshot.store_reads = snapshot
        .known_stores
        .iter()
        .map(|store| CatalogStoreRead {
            store: store.store_ref(),
            state: CatalogStoreReadState::NotLoaded,
        })
        .collect();
    let mut accounts_complete = false;
    let mut teams_complete = false;
    token.check()?;
    publish_local(&snapshot);
    let overview = transport.call_cancellable(
        Operation::ListProfileOverview {
            profile: profile.clone(),
        },
        &|| token.check().is_err(),
    );
    token.check()?;
    match overview {
        Ok(value) => match decode_agent_value::<ProfileOverview>(value) {
            Ok(overview) if overview.profile == profile => {
                match response_value(overview.accounts.clone())
                    .and_then(decode_agent_value::<Vec<AccountSummary>>)
                {
                    Ok(accounts) if accounts.iter().all(|account| account.profile == profile) => {
                        accounts_complete = true;
                        snapshot.stores.extend(accounts.into_iter().map(|account| {
                            CatalogStoreSummary::Account {
                                store: AccountStoreRef {
                                    profile: profile.clone(),
                                    account_alias: account.alias,
                                },
                            }
                        }));
                    }
                    Ok(_) => snapshot.failures.push(CatalogFailure {
                        scope: CatalogFailureScope::Profile {
                            profile: profile.clone(),
                            source: "account stores".to_owned(),
                        },
                        error: AgentError::Transport(
                            "agent returned accounts for a different profile".to_owned(),
                        ),
                    }),
                    Err(error) => {
                        if error_blocks_profile_catalog(&error) {
                            snapshot.blocked_profiles.push(profile.clone());
                        }
                        snapshot.failures.push(CatalogFailure {
                            scope: CatalogFailureScope::Profile {
                                profile: profile.clone(),
                                source: "account stores".to_owned(),
                            },
                            error,
                        });
                    }
                }
                match response_value(overview.teams.clone())
                    .and_then(decode_agent_value::<Vec<TeamSummary>>)
                {
                    Ok(teams) => {
                        teams_complete = true;
                        snapshot.stores.extend(teams.into_iter().map(|team| {
                            CatalogStoreSummary::Team {
                                store: TeamStoreRef {
                                    profile: profile.clone(),
                                    account_alias: team.account_alias,
                                    team_alias: team.alias,
                                    team_id: team.team_id_hex,
                                },
                                kind: team.kind,
                                name: team.name,
                                active: team.active,
                                creation_phase: team.creation_phase,
                                chain_seqno: team.chain_seqno,
                            }
                        }));
                    }
                    Err(error) => {
                        if error_blocks_profile_catalog(&error) {
                            snapshot.blocked_profiles.push(profile.clone());
                        }
                        snapshot.failures.push(CatalogFailure {
                            scope: CatalogFailureScope::Profile {
                                profile: profile.clone(),
                                source: "team stores".to_owned(),
                            },
                            error,
                        });
                    }
                }
                if let Ok(status) = response_value(overview.server_status.clone())
                    .and_then(decode_agent_value::<ServerStatusSnapshot>)
                {
                    if status.profile != profile {
                        snapshot.failures.push(CatalogFailure {
                            scope: CatalogFailureScope::Profile {
                                profile: profile.clone(),
                                source: "server status".to_owned(),
                            },
                            error: AgentError::Transport(
                                "agent returned server status for a different profile".to_owned(),
                            ),
                        });
                    }
                }
                snapshot.profile_overviews.push(overview);
            }
            Ok(_) => snapshot.failures.push(CatalogFailure {
                scope: CatalogFailureScope::Profile {
                    profile: profile.clone(),
                    source: "profile overview".to_owned(),
                },
                error: AgentError::Transport(
                    "agent returned an overview for a different profile".to_owned(),
                ),
            }),
            Err(error) => snapshot.failures.push(CatalogFailure {
                scope: CatalogFailureScope::Profile {
                    profile: profile.clone(),
                    source: "profile overview".to_owned(),
                },
                error,
            }),
        },
        Err(error) => {
            if error_blocks_profile_catalog(&error) {
                snapshot.blocked_profiles.push(profile.clone());
            }
            snapshot.failures.push(CatalogFailure {
                scope: CatalogFailureScope::Profile {
                    profile: profile.clone(),
                    source: "profile overview".to_owned(),
                },
                error,
            });
        }
    }
    if accounts_complete {
        snapshot
            .known_stores
            .retain(|store| !matches!(store, CatalogStoreSummary::Account { .. }));
        snapshot.known_stores.extend(
            snapshot
                .stores
                .iter()
                .filter(|store| matches!(store, CatalogStoreSummary::Account { .. }))
                .cloned(),
        );
    }
    if teams_complete {
        snapshot
            .known_stores
            .retain(|store| !matches!(store, CatalogStoreSummary::Team { .. }));
        snapshot.known_stores.extend(
            snapshot
                .stores
                .iter()
                .filter(|store| matches!(store, CatalogStoreSummary::Team { .. }))
                .cloned(),
        );
    }
    snapshot.inventory.push(CatalogInventoryState {
        profile: profile.clone(),
        accounts_complete,
        teams_complete,
    });
    snapshot
        .known_stores
        .sort_by(|left, right| left.label().cmp(right.label()));
    snapshot.store_reads = snapshot
        .known_stores
        .iter()
        .map(|store| CatalogStoreRead {
            store: store.store_ref(),
            state: CatalogStoreReadState::NotLoaded,
        })
        .collect();
    if include_items {
        publish_local(&snapshot);
    }
    if !include_items || snapshot.blocked_profiles.contains(&profile) {
        return Ok(snapshot);
    }
    snapshot
        .stores
        .sort_by(|left, right| left.label().cmp(right.label()));
    let stores = snapshot
        .stores
        .iter()
        .filter(|store| !matches!(store, CatalogStoreSummary::Team { active: false, .. }))
        .map(CatalogStoreSummary::store_ref)
        .collect::<Vec<_>>();
    for store in stores {
        token.check()?;
        let result = load_store_pages(transport, &store, fresh, &token);
        if let Some(read) = snapshot
            .store_reads
            .iter_mut()
            .find(|read| read.store == store)
        {
            read.state = if result.is_ok() {
                CatalogStoreReadState::Complete
            } else {
                CatalogStoreReadState::Failed
            };
        }
        match result {
            Ok(entries) => snapshot
                .items
                .extend(entries.into_iter().map(|metadata| CatalogItem {
                    store: store.clone(),
                    metadata,
                })),
            Err(error) => {
                let blocks_profile = error_blocks_profile_catalog(&error);
                let denies_kv = matches!(&error, AgentError::Protocol { code: ErrorCode::CapabilityDenied, fields, .. } if fields.capability.as_deref() == Some("kv"));
                snapshot.failures.push(CatalogFailure {
                    scope: if blocks_profile || denies_kv {
                        CatalogFailureScope::Profile {
                            profile: profile.clone(),
                            source: "KV catalog".to_owned(),
                        }
                    } else {
                        CatalogFailureScope::Store(store)
                    },
                    error,
                });
                if blocks_profile || denies_kv {
                    snapshot.items.clear();
                    for read in &mut snapshot.store_reads {
                        read.state = CatalogStoreReadState::Failed;
                    }
                    if blocks_profile {
                        snapshot.blocked_profiles.push(profile.clone());
                    }
                    break;
                }
            }
        }
    }
    token.check()?;
    if accounts_complete && teams_complete && snapshot.failures.is_empty() {
        snapshot.full_item_reads = Some(vec![profile]);
    }
    Ok(snapshot)
}

fn error_blocks_profile_catalog(error: &AgentError) -> bool {
    matches!(
        error,
        AgentError::Protocol {
            code: ErrorCode::RollbackDetected | ErrorCode::CheckpointResetRequired,
            ..
        }
    )
}

/// Reads the exact catalog version selected by the user. Large files are
/// assembled from version-bound chunks to ensure concurrent edits cannot
/// result in spliced or inconsistent content.
pub fn read_catalog_item(
    transport: &dyn AgentTransport,
    item: &CatalogItem,
) -> Result<KvItemRead, AgentError> {
    let store = kv_store_ref(&item.store);
    let value = transport.call(Operation::ReadKv {
        store: store.clone(),
        path: item.metadata.path.clone(),
        dirent_id: item.metadata.dirent_id,
        version: item.metadata.version,
    })?;
    let mut read: KvReadResult = decode_agent_value(value)?;
    if read.store != store
        || read.path != item.metadata.path
        || read.dirent_id != item.metadata.dirent_id
        || read.version != item.metadata.version
        || read.node_type != item.metadata.node_type
        || read.read_role != item.metadata.read_role
        || read.write_role != item.metadata.write_role
    {
        zeroize_kv_read_payload(&mut read);
        return Err(AgentError::Transport(
            "agent returned a KV value for a different catalog selection".to_owned(),
        ));
    }
    let value = match read.node_type.as_str() {
        "small-file" => {
            if read.symlink_target.is_some() {
                zeroize_kv_read_payload(&mut read);
                return Err(AgentError::Transport(
                    "Agent returned unexpected symlink target in small-file response".to_owned(),
                ));
            }
            KvItemValue::File(Zeroizing::new(read.content.take().ok_or_else(|| {
                AgentError::Transport("missing content in small-file response".to_owned())
            })?))
        }
        "file" => {
            if read.content.is_some() || read.symlink_target.is_some() {
                zeroize_kv_read_payload(&mut read);
                return Err(AgentError::Transport(
                    "agent mixed inline content into a large-file response".to_owned(),
                ));
            }
            // A large file reports no size: the agent would have to download
            // and decrypt the whole file to measure one, doubling the bytes
            // this loop is about to move. The loop ends on the end-of-file
            // flag the chunk response carries, which this side already
            // validates against what it received, and the reveal cap becomes
            // a cap on what has accumulated rather than a pre-check.
            let mut content = Zeroizing::new(Vec::new());
            let mut offset = 0u64;
            loop {
                let length = KV_READ_CHUNK_BYTES;
                let mut chunk: KvChunkResult =
                    decode_agent_value(transport.call(Operation::ReadKvChunk {
                        store: store.clone(),
                        path: item.metadata.path.clone(),
                        dirent_id: item.metadata.dirent_id,
                        version: item.metadata.version,
                        offset,
                        length,
                    })?)?;
                let invalid = chunk.store != store
                    || chunk.path != item.metadata.path
                    || chunk.dirent_id != item.metadata.dirent_id
                    || chunk.version != item.metadata.version
                    || chunk.offset != offset
                    || chunk.content.is_empty()
                    || chunk.content.len() > length as usize;
                if invalid {
                    chunk.content.zeroize();
                    return Err(AgentError::Transport(
                        "received invalid or mismatched KV chunk".to_owned(),
                    ));
                }
                offset = offset
                    .checked_add(chunk.content.len() as u64)
                    .ok_or_else(|| {
                        AgentError::Transport("chunk offset overflow while reading file".to_owned())
                    })?;
                if offset > MAXIMUM_DESKTOP_REVEAL_BYTES {
                    chunk.content.zeroize();
                    return Err(AgentError::Transport(format!(
                        "File exceeds maximum in-memory preview limit of {MAXIMUM_DESKTOP_REVEAL_BYTES} bytes"
                    )));
                }
                content.extend_from_slice(&chunk.content);
                let eof = chunk.eof;
                chunk.content.zeroize();
                if eof {
                    break;
                }
            }
            KvItemValue::File(content)
        }
        "symlink" => {
            if read.content.is_some() {
                zeroize_kv_read_payload(&mut read);
                return Err(AgentError::Transport(
                    "unexpected file content in symlink response".to_owned(),
                ));
            }
            KvItemValue::Symlink(Zeroizing::new(read.symlink_target.take().ok_or_else(
                || AgentError::Transport("missing symlink target in response".to_owned()),
            )?))
        }
        "directory" => {
            if read.content.is_some() || read.symlink_target.is_some() {
                zeroize_kv_read_payload(&mut read);
                return Err(AgentError::Transport(
                    "unexpected content payload returned for directory".to_owned(),
                ));
            }
            KvItemValue::Directory
        }
        _ => {
            zeroize_kv_read_payload(&mut read);
            return Err(AgentError::Transport(
                "unsupported KV node type in response".to_owned(),
            ));
        }
    };
    Ok(KvItemRead {
        store: item.store.clone(),
        path: read.path,
        version: read.version,
        read_role: read.read_role,
        write_role: read.write_role,
        value,
    })
}

fn zeroize_kv_read_payload(read: &mut KvReadResult) {
    if let Some(content) = &mut read.content {
        content.zeroize();
    }
    if let Some(target) = &mut read.symlink_target {
        target.zeroize();
    }
}

/// Builds the create mutation for one text item.
///
/// Creates set `mkdir_p` so parent directories are created automatically
/// when writing a new path. Edits and replacements address existing paths
/// and do not need parent directory creation.
pub fn create_kv_file_mutation(
    store: &CatalogStoreRef,
    path: &str,
    content: Vec<u8>,
) -> Result<KvAccountMutation, &'static str> {
    file_mutation(
        kv_store_ref(store),
        required_path(path)?,
        content,
        KvRole::Owner,
        KvRole::Owner,
        KvPrecondition::Create,
        true,
    )
}

pub fn edit_kv_file_mutation(
    item: &CatalogItem,
    content: Vec<u8>,
) -> Result<KvAccountMutation, &'static str> {
    if !matches!(item.metadata.node_type.as_str(), "file" | "small-file") {
        return Err("the selected item is not a file");
    }
    file_mutation(
        kv_store_ref(&item.store),
        item.metadata.path.clone(),
        content,
        item.metadata.read_role,
        item.metadata.write_role,
        KvPrecondition::ExactEntry {
            dirent_id: item.metadata.dirent_id,
            version: item.metadata.version,
        },
        false,
    )
}

/// Builds the header for a native file upload without first assembling the
/// file in memory. The caller must stream exactly `total_length` bytes from an
/// already-open source handle.
pub fn create_kv_file_upload(
    store: &CatalogStoreRef,
    path: &str,
    total_length: u64,
) -> Result<KvUploadHeader, &'static str> {
    file_upload_header(
        kv_store_ref(store),
        required_path(path)?,
        total_length,
        KvRole::Owner,
        KvRole::Owner,
        KvPrecondition::Create,
        true,
    )
}

/// Builds an exact-version replacement header from authenticated catalog
/// metadata. The caller must stream from an already-open source handle.
pub fn edit_kv_file_upload(
    item: &CatalogItem,
    total_length: u64,
) -> Result<KvUploadHeader, &'static str> {
    if !matches!(item.metadata.node_type.as_str(), "file" | "small-file") {
        return Err("the selected item is not a file");
    }
    file_upload_header(
        kv_store_ref(&item.store),
        item.metadata.path.clone(),
        total_length,
        item.metadata.read_role,
        item.metadata.write_role,
        KvPrecondition::ExactEntry {
            dirent_id: item.metadata.dirent_id,
            version: item.metadata.version,
        },
        false,
    )
}

pub fn create_kv_symlink_operation(
    store: &CatalogStoreRef,
    path: &str,
    target: &str,
) -> Result<Operation, &'static str> {
    Ok(Operation::PutKvSymlink {
        store: kv_store_ref(store),
        path: required_path(path)?,
        target: required_verbatim_text(target, "symlink target is required")?,
        read_role: KvRole::Owner,
        write_role: KvRole::Owner,
        precondition: KvPrecondition::Create,
        mkdir_p: true,
    })
}

pub fn edit_kv_symlink_operation(
    item: &CatalogItem,
    _target: &str,
) -> Result<Operation, &'static str> {
    if item.metadata.node_type != "symlink" {
        return Err("the selected item is not a symlink");
    }
    Err("symlink targets cannot be edited directly; delete and recreate the symlink")
}

pub fn create_kv_directory_operation(
    store: &CatalogStoreRef,
    path: &str,
) -> Result<Operation, &'static str> {
    Ok(Operation::MkdirKv {
        store: kv_store_ref(store),
        path: required_path(path)?,
        read_role: KvRole::Owner,
        write_role: KvRole::Owner,
        precondition: KvPrecondition::Create,
        mkdir_p: true,
    })
}

pub fn move_kv_operation(item: &CatalogItem, destination: &str) -> Result<Operation, &'static str> {
    let destination = required_path(destination)?;
    if destination == item.metadata.path {
        return Err("source and destination are identical");
    }
    Ok(Operation::MoveKv {
        dirent_id: item.metadata.dirent_id,
        store: kv_store_ref(&item.store),
        path: item.metadata.path.clone(),
        destination,
        version: item.metadata.version,
    })
}

pub fn remove_kv_operation(item: &CatalogItem, recursive: bool) -> Result<Operation, &'static str> {
    Ok(Operation::RemoveKv {
        expected_dirent_id: Some(item.metadata.dirent_id),
        store: kv_store_ref(&item.store),
        path: item.metadata.path.clone(),
        recursive,
        precondition: KvPrecondition::ExactVersion {
            version: item.metadata.version,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn file_mutation(
    store: KvStoreRef,
    path: String,
    content: Vec<u8>,
    read_role: KvRole,
    write_role: KvRole,
    precondition: KvPrecondition,
    mkdir_p: bool,
) -> Result<KvAccountMutation, &'static str> {
    if content.len() <= MAXIMUM_INLINE_KV_BYTES {
        Ok(KvAccountMutation::Inline(Operation::PutKv {
            store,
            path,
            content,
            read_role,
            write_role,
            precondition,
            mkdir_p,
        }))
    } else {
        let total_length = u64::try_from(content.len())
            .map_err(|_| "content size exceeds maximum upload limit")?;
        Ok(KvAccountMutation::Stream {
            header: KvUploadHeader {
                upload_id: new_upload_id(total_length)?,
                adapter: None,
                store,
                path,
                total_length,
                read_role,
                write_role,
                precondition,
                mkdir_p,
            },
            content: Zeroizing::new(content),
        })
    }
}

fn file_upload_header(
    store: KvStoreRef,
    path: String,
    total_length: u64,
    read_role: KvRole,
    write_role: KvRole,
    precondition: KvPrecondition,
    mkdir_p: bool,
) -> Result<KvUploadHeader, &'static str> {
    Ok(KvUploadHeader {
        upload_id: new_upload_id(total_length)?,
        adapter: None,
        store,
        path,
        total_length,
        read_role,
        write_role,
        precondition,
        mkdir_p,
    })
}

fn kv_store_ref(store: &CatalogStoreRef) -> KvStoreRef {
    match store {
        CatalogStoreRef::Account(store) => KvStoreRef::Account(store.clone()),
        CatalogStoreRef::Team(store) => KvStoreRef::Team(store.clone()),
    }
}

fn required_path(path: &str) -> Result<String, &'static str> {
    let path = required_verbatim_text(path, "enter an absolute item path")?;
    if !path.starts_with('/') || path == "/" {
        return Err("enter an absolute path below the store root");
    }
    Ok(path)
}

fn known_catalog_store(profile: &str, store: KnownStoreSummary) -> CatalogStoreSummary {
    match store {
        KnownStoreSummary::Account { account_alias, .. } => CatalogStoreSummary::Account {
            store: AccountStoreRef {
                profile: profile.to_owned(),
                account_alias,
            },
        },
        KnownStoreSummary::Team {
            account_alias,
            team_alias,
            team_id_hex,
            team_kind,
            name,
            active,
            ..
        } => CatalogStoreSummary::Team {
            store: TeamStoreRef {
                profile: profile.to_owned(),
                account_alias,
                team_alias,
                team_id: team_id_hex,
            },
            kind: match team_kind {
                TeamKind::Named => "named",
                TeamKind::AdHoc => "ad-hoc",
            }
            .to_owned(),
            name,
            active,
            creation_phase: None,
            // A remembered store is a fallback for a profile whose overview
            // could not be read, so it carries no pinned chain sequence.
            chain_seqno: None,
        },
    }
}

fn load_store_pages(
    transport: &dyn AgentTransport,
    store: &CatalogStoreRef,
    fresh: bool,
    token: &CatalogLoadToken,
) -> Result<Vec<KvEntryMetadata>, AgentError> {
    match load_store_pages_once(transport, store, fresh, token) {
        Err(error) if catalog_cursor_snapshot_changed(&error) => {
            token.check()?;
            load_store_pages_once(transport, store, fresh, token)
        }
        result => result,
    }
}

fn load_store_pages_once(
    transport: &dyn AgentTransport,
    store: &CatalogStoreRef,
    fresh: bool,
    token: &CatalogLoadToken,
) -> Result<Vec<KvEntryMetadata>, AgentError> {
    // The agent truncates a page by encoded bytes against its frame budget
    // and returns a cursor for exactly what it kept, so the byte budget is
    // the real bound and this is only an upper limit on rows per round trip.
    //
    // It is the agent's own maximum. A desktop asking for more than an agent
    // allows is refused as an invalid request, so the value may only be the
    // maximum of the agents this desktop can actually reach. The protocol
    // version decides that set: an agent built before this maximum rose also
    // predates the base64 wire change, so it rejects the frame at its version
    // check before it ever parses this field, and the desktop answers that by
    // replacing it. No agent that would refuse this value can serve a request
    // carrying it.
    const PAGE_LIMIT: u32 = 4096;
    const MAXIMUM_PAGES: usize = 4096;
    let mut cursor = None;
    let mut snapshot_version = None;
    let mut entries = Vec::new();
    for _ in 0..MAXIMUM_PAGES {
        token.check()?;
        // Freshness applies to the first page only: every later page must
        // come from the snapshot its cursor names, and a listing made for the
        // later request would be a different one.
        let fresh = fresh && cursor.is_none();
        let operation = match store {
            CatalogStoreRef::Account(store) => Operation::ListKv {
                store: store.clone(),
                cursor: cursor.clone(),
                limit: PAGE_LIMIT,
                fresh,
            },
            CatalogStoreRef::Team(store) => Operation::ListTeamKv {
                store: store.clone(),
                cursor: cursor.clone(),
                limit: PAGE_LIMIT,
                fresh,
            },
        };
        let page: KvPage =
            decode_agent_value(transport.call_cancellable(operation, &|| token.check().is_err())?)?;
        token.check()?;
        if snapshot_version
            .replace(page.snapshot_version)
            .is_some_and(|version| version != page.snapshot_version)
        {
            return Err(invalid_agent_response(
                "catalog snapshot changed between pages",
            ));
        }
        if page.entries.is_empty() && page.next_cursor.is_some() {
            return Err(invalid_agent_response(
                "pagination stalled on empty page with non-empty cursor",
            ));
        }
        entries.extend(page.entries);
        let Some(next) = page.next_cursor else {
            return Ok(entries);
        };
        cursor = Some(next);
    }
    Err(invalid_agent_response(
        "catalog pagination exceeded maximum page count",
    ))
}

fn catalog_cursor_snapshot_changed(error: &AgentError) -> bool {
    matches!(
        error,
        AgentError::Protocol {
            code: ErrorCode::CatalogSnapshotChanged,
            ..
        }
    )
}

fn decode_agent_value<T: DeserializeOwned>(value: Value) -> Result<T, AgentError> {
    serde_json::from_value(value)
        .map_err(|error| invalid_agent_response(&format!("invalid agent response: {error}")))
}

fn invalid_agent_response(message: &str) -> AgentError {
    AgentError::Ipc {
        code: IpcErrorCode::Protocol,
        message: message.to_owned(),
        ambiguous: false,
        connection_lost: false,
    }
}

fn run_bounded<T, R>(jobs: Vec<T>, work: impl Fn(T) -> R + Sync) -> Vec<R>
where
    T: Send,
    R: Send,
{
    const MAXIMUM_WORKERS: usize = 4;
    if jobs.is_empty() {
        return Vec::new();
    }
    let count = jobs.len();
    let queue = Mutex::new(jobs.into_iter().enumerate().collect::<VecDeque<_>>());
    let results = Mutex::new((0..count).map(|_| None).collect::<Vec<Option<R>>>());
    std::thread::scope(|scope| {
        for _ in 0..count.min(MAXIMUM_WORKERS) {
            scope.spawn(|| loop {
                let Some((index, job)) = queue.lock().expect("catalog queue poisoned").pop_front()
                else {
                    break;
                };
                results.lock().expect("catalog results poisoned")[index] = Some(work(job));
            });
        }
    });
    results
        .into_inner()
        .expect("catalog results poisoned")
        .into_iter()
        .map(|result| result.expect("catalog worker dropped a result"))
        .collect()
}

fn required_verbatim_text(value: &str, error: &'static str) -> Result<String, &'static str> {
    if value.trim().is_empty() {
        Err(error)
    } else {
        Ok(value.to_owned())
    }
}

/// Installs a local-only crash marker hook. Reports contain no panic payload,
/// paths, protocol values, or automatic upload mechanism.
pub fn install_crash_reporter(directory: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = write_crash_marker(&directory);
        previous(info);
    }));
}

fn write_crash_marker(directory: &std::path::Path) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
        if !directory.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(directory)?;
        }
        let metadata = fs::symlink_metadata(directory)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "crash directory is not private",
            ));
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = directory.join(format!("crash-{timestamp}-{}.txt", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        writeln!(file, "foks-desktop-version={}", env!("CARGO_PKG_VERSION"))?;
        writeln!(file, "timestamp={timestamp}")?;
        writeln!(file, "platform={}", std::env::consts::OS)?;
        file.sync_all()?;
        Ok(path)
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "crash markers are unsupported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn profile_overview(profile: &str, accounts: ResponseResult, teams: ResponseResult) -> Value {
        serde_json::to_value(ProfileOverview {
            profile: profile.to_owned(),
            accounts,
            teams,
            server_status: ResponseResult::Success {
                value: serde_json::json!({
                    "profile": profile,
                    "configured_probe": "localhost:4430",
                    "host": null,
                    "compatibility": {"status":"not-required"},
                    "chat_supported": null
                }),
            },
        })
        .unwrap()
    }

    fn success(value: Value) -> ResponseResult {
        ResponseResult::Success { value }
    }

    struct CatalogTransport;

    impl AgentTransport for CatalogTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            match operation {
                Operation::ListProfiles => Ok(serde_json::json!([{"name": "local"}])),
                Operation::ListKnownStores { .. } => Ok(serde_json::json!([])),
                Operation::ListProfileOverview { profile } => {
                    assert_eq!(profile, "local");
                    Ok(profile_overview(
                        &profile,
                        success(serde_json::json!([{
                            "profile": "local",
                            "alias": "personal",
                            "username": "alice"
                        }])),
                        success(serde_json::json!([{
                            "alias": "eng",
                            "account_alias": "personal",
                            "team_id_hex": "aa",
                            "kind": "named",
                            "name": "engineering",
                            "active": true
                        }])),
                    ))
                }
                Operation::ListKv { cursor, .. } => {
                    let (path, next_cursor) = if cursor.is_none() {
                        ("/first", Some("next"))
                    } else {
                        ("/second", None)
                    };
                    Ok(serde_json::to_value(KvPage {
                        snapshot_version: 7,
                        entries: vec![KvEntryMetadata {
                            dirent_id: [7; 16],
                            path: path.to_owned(),
                            node_type: "small-file".to_owned(),
                            version: 1,
                            size: None,
                            read_role: foks_agent_proto::KvRole::Owner,
                            write_role: foks_agent_proto::KvRole::Owner,
                            shape: None,
                        }],
                        next_cursor: next_cursor.map(str::to_owned),
                    })
                    .unwrap())
                }
                Operation::ListTeamKv { .. } => Err(AgentError::Protocol {
                    code: ErrorCode::CapabilityDenied,
                    message: "team KV is unavailable".to_owned(),
                    fields: ErrorFields {
                        capability: Some("kv".to_owned()),
                        ..ErrorFields::default()
                    }
                    .into(),
                }),
                operation => panic!("unexpected catalog operation: {operation:?}"),
            }
        }
    }

    #[test]
    fn progressive_catalog_publishes_local_and_healthy_before_slow_profile() {
        struct ProgressiveTransport(
            std::sync::mpsc::Sender<()>,
            Mutex<std::sync::mpsc::Receiver<()>>,
        );
        impl AgentTransport for ProgressiveTransport {
            fn call(&self, operation: Operation) -> Result<Value, AgentError> {
                match operation {
                    Operation::ListProfiles => {
                        Ok(serde_json::json!([{"name":"slow"}, {"name":"healthy"}]))
                    }
                    Operation::ListKnownStores { .. } => Ok(serde_json::json!([])),
                    Operation::ListProfileOverview { profile } => Ok(profile_overview(
                        &profile,
                        success(
                            serde_json::json!([{"profile":profile,"alias":"personal","username":"alice"}]),
                        ),
                        success(serde_json::json!([])),
                    )),
                    Operation::ListKv { store, .. } => {
                        if store.profile == "slow" {
                            self.0.send(()).unwrap();
                            self.1.lock().unwrap().recv().unwrap();
                            return Err(AgentError::Transport("offline".into()));
                        }
                        Ok(
                            serde_json::json!({"snapshot_version":1,"entries":[],"next_cursor":null}),
                        )
                    }
                    other => panic!("unexpected operation: {other:?}"),
                }
            }
        }
        for cancel in [false, true] {
            let (entered, blocked) = std::sync::mpsc::channel();
            let (release, gate) = std::sync::mpsc::channel();
            let (publish, progress) = std::sync::mpsc::channel();
            let token = CatalogLoadToken::default();
            let worker_token = token.clone();
            let worker = std::thread::spawn(move || {
                load_catalog_progressive_cancellable(
                    Arc::new(ProgressiveTransport(entered, Mutex::new(gate))),
                    worker_token,
                    false,
                    |snapshot| {
                        publish.send(snapshot.clone()).unwrap();
                    },
                )
            });
            blocked
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            let mut healthy = None;
            let mut local = false;
            while let Ok(snapshot) = progress.recv_timeout(std::time::Duration::from_secs(2)) {
                if snapshot
                    .store_reads
                    .iter()
                    .any(|read| read.state == CatalogStoreReadState::NotLoaded)
                {
                    local = true;
                    assert!(!snapshot.known_stores.is_empty());
                }
                if snapshot
                    .full_item_reads
                    .as_ref()
                    .is_some_and(|profiles| profiles.contains(&"healthy".into()))
                {
                    healthy = Some(snapshot);
                    break;
                }
            }
            if cancel {
                token.cancel();
            }
            release.send(()).unwrap();
            let result = worker.join().unwrap();
            if cancel {
                assert_eq!(result, Err(AgentError::Cancelled));
            } else {
                let result = result.unwrap();
                assert_eq!(result.full_item_reads, Some(vec!["healthy".into()]));
                assert_eq!(result.failures.len(), 1);
                assert!(result
                    .store_reads
                    .iter()
                    .any(|read| read.store.profile() == "healthy"
                        && read.state == CatalogStoreReadState::Complete));
                assert!(result
                    .store_reads
                    .iter()
                    .any(|read| read.store.profile() == "slow"
                        && read.state == CatalogStoreReadState::Failed));
                assert!(
                    matches!(&result.failures[0].scope, CatalogFailureScope::Store(store) if store.profile() == "slow")
                );
            }
            assert!(local);
            let healthy = healthy.expect("healthy profile must publish while slow KV is blocked");
            assert_eq!(healthy.profiles, ["slow", "healthy"]);
            assert!(!healthy
                .store_reads
                .iter()
                .any(|read| read.store.profile() == "slow"
                    && read.state == CatalogStoreReadState::Complete));
            if cancel {
                assert!(progress.try_iter().all(|snapshot| !snapshot
                    .store_reads
                    .iter()
                    .any(|read| read.store.profile() == "slow"
                        && read.state != CatalogStoreReadState::NotLoaded)));
            }
        }
    }

    #[test]
    fn profile_catalog_loading_never_contacts_unrelated_profiles() {
        struct IsolatedCatalogTransport(Mutex<Vec<Operation>>);
        impl AgentTransport for IsolatedCatalogTransport {
            fn call(&self, operation: Operation) -> Result<Value, AgentError> {
                self.0.lock().unwrap().push(operation.clone());
                if matches!(operation, Operation::ListProfiles) {
                    return Ok(serde_json::json!([{"name":"local"}, {"name":"unavailable"}]));
                }
                match &operation {
                    Operation::ListKnownStores { profile }
                    | Operation::ListProfileOverview { profile } => assert_eq!(profile, "local"),
                    Operation::ListKv { store, .. } => assert_eq!(store.profile, "local"),
                    Operation::ListTeamKv { store, .. } => assert_eq!(store.profile, "local"),
                    _ => panic!("unexpected catalog operation"),
                }
                CatalogTransport.call(operation)
            }
        }
        let transport = Arc::new(IsolatedCatalogTransport(Mutex::default()));
        let catalog = load_profile_catalog_cancellable(
            transport.clone(),
            "local".into(),
            CatalogLoadToken::default(),
            false,
        )
        .unwrap();
        assert_eq!(catalog.profiles, ["local"]);
        assert_eq!(catalog.stores.len(), 2);
        assert!(catalog.blocked_profiles.is_empty());
        transport.0.lock().unwrap().clear();
        assert!(load_profile_catalog_cancellable(
            transport.clone(),
            "missing".into(),
            CatalogLoadToken::default(),
            false,
        )
        .is_err());
        assert!(matches!(
            transport.0.lock().unwrap().as_slice(),
            [Operation::ListProfiles]
        ));
        transport.0.lock().unwrap().clear();
        let token = CatalogLoadToken::default();
        token.cancel();
        assert!(matches!(
            load_profile_catalog_cancellable(transport.clone(), "local".into(), token, false),
            Err(AgentError::Cancelled)
        ));
        assert!(transport.0.lock().unwrap().is_empty());

        // A caller that has already listed the configured profiles hands them
        // over instead of making the agent list them again.
        transport.0.lock().unwrap().clear();
        let profiles = vec!["local".to_owned(), "unavailable".to_owned()];
        let catalog = load_profile_catalog_with_profiles(
            transport.clone(),
            "local".into(),
            &profiles,
            CatalogLoadToken::default(),
            false,
        )
        .unwrap();
        assert_eq!(catalog.profiles, ["local"]);
        assert!(!transport
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Operation::ListProfiles)));
        assert!(load_profile_catalog_with_profiles(
            transport.clone(),
            "missing".into(),
            &profiles,
            CatalogLoadToken::default(),
            false,
        )
        .is_err());

        transport.0.lock().unwrap().clear();
        let progressive = load_catalog_progressive_with_profiles(
            transport.clone(),
            vec!["local".to_owned()],
            CatalogLoadToken::default(),
            false,
            |_| {},
        )
        .unwrap();
        assert_eq!(progressive.profiles, ["local"]);
        assert!(!transport
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Operation::ListProfiles)));
    }

    #[test]
    fn security_failures_still_block_profiles_independently_of_capabilities() {
        for code in [
            ErrorCode::RollbackDetected,
            ErrorCode::CheckpointResetRequired,
        ] {
            assert!(error_blocks_profile_catalog(&AgentError::Protocol {
                code,
                message: "changed wording".into(),
                fields: Box::default()
            }));
        }
        for capability in ["kv", "chat", "teams"] {
            assert!(!error_blocks_profile_catalog(&AgentError::Protocol {
                code: ErrorCode::CapabilityDenied,
                message: "changed wording".into(),
                fields: ErrorFields {
                    capability: Some(capability.into()),
                    ..ErrorFields::default()
                }
                .into(),
            }));
        }
    }

    #[test]
    fn kv_capability_failure_restricts_vaults_without_blocking_profile_trust() {
        let catalog = load_catalog(Arc::new(CatalogTransport)).unwrap();
        assert_eq!(catalog.full_item_reads, Some(vec![]));
        assert_eq!(catalog.stores.len(), 2);
        assert!(catalog.items.is_empty());
        assert!(catalog.blocked_profiles.is_empty());
        assert_eq!(catalog.failures.len(), 1);
        assert!(matches!(
            catalog.failures[0].scope,
            CatalogFailureScope::Profile { ref profile, .. } if profile == "local"
        ));
    }

    struct PartialCatalogTransport;

    impl AgentTransport for PartialCatalogTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            match operation {
                Operation::ListProfiles => Ok(serde_json::json!([{"name": "local"}])),
                Operation::ListKnownStores { .. } => Ok(serde_json::json!([{
                    "store_kind": "account",
                    "account_alias": "old-account",
                    "last_seen_at": 10
                }, {
                    "store_kind": "team",
                    "account_alias": "personal",
                    "team_alias": "household",
                    "team_id_hex": "03aa",
                    "team_kind": "named",
                    "name": "Household",
                    "active": true,
                    "last_seen_at": 11
                }])),
                Operation::ListProfileOverview { profile } => Ok(profile_overview(
                    &profile,
                    success(serde_json::json!([])),
                    ResponseResult::Error {
                        code: ErrorCode::OperationFailed,
                        message: "offline".to_owned(),
                        fields: ErrorFields::default(),
                    },
                )),
                operation => panic!("unexpected partial catalog operation: {operation:?}"),
            }
        }
    }

    #[test]
    fn partial_catalog_replaces_only_successful_known_store_sources() {
        let catalog = load_catalog(Arc::new(PartialCatalogTransport)).unwrap();
        assert_eq!(catalog.full_item_reads, Some(vec![]));
        assert!(catalog.stores.is_empty());
        assert!(matches!(
            catalog.known_stores.as_slice(),
            [CatalogStoreSummary::Team { store, .. }] if store.team_alias == "household"
        ));
        assert_eq!(
            catalog.inventory,
            [CatalogInventoryState {
                profile: "local".to_owned(),
                accounts_complete: true,
                teams_complete: false,
            }]
        );
        assert_eq!(catalog.failures.len(), 1);
    }

    struct AccountCatalogTransport;

    impl AgentTransport for AccountCatalogTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            match operation {
                Operation::ListProfiles => Ok(serde_json::json!([{"name": "local"}])),
                Operation::ListKnownStores { .. } => Ok(serde_json::json!([])),
                Operation::ListProfileOverview { profile } => Ok(profile_overview(
                    &profile,
                    success(serde_json::json!([{
                        "profile": "local",
                        "alias": "personal",
                        "username": "alice"
                    }])),
                    success(serde_json::json!([])),
                )),
                Operation::ListKv { cursor, .. } => {
                    let (path, next_cursor) = if cursor.is_none() {
                        ("/first", Some("next"))
                    } else {
                        ("/second", None)
                    };
                    Ok(serde_json::to_value(KvPage {
                        snapshot_version: 7,
                        entries: vec![KvEntryMetadata {
                            dirent_id: [7; 16],
                            path: path.to_owned(),
                            node_type: "small-file".to_owned(),
                            version: 1,
                            size: None,
                            read_role: KvRole::Owner,
                            write_role: KvRole::Owner,
                            shape: None,
                        }],
                        next_cursor: next_cursor.map(str::to_owned),
                    })
                    .unwrap())
                }
                operation => panic!("unexpected account catalog operation: {operation:?}"),
            }
        }
    }

    #[test]
    fn unified_catalog_pages_account_items_on_one_profile_queue() {
        let catalog = load_catalog(Arc::new(AccountCatalogTransport)).unwrap();
        assert_eq!(catalog.full_item_reads, Some(vec!["local".into()]));
        assert_eq!(catalog.items.len(), 2);
        assert_eq!(catalog.items[0].metadata.path, "/first");
        assert_eq!(catalog.items[1].metadata.path, "/second");
        assert!(catalog.failures.is_empty());
    }

    struct LimitRecordingTransport {
        limits: Mutex<Vec<u32>>,
    }

    impl AgentTransport for LimitRecordingTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            let Operation::ListKv { limit, .. } = operation else {
                panic!("unexpected operation")
            };
            self.limits.lock().unwrap().push(limit);
            Ok(serde_json::to_value(KvPage {
                snapshot_version: 3,
                entries: Vec::new(),
                next_cursor: None,
            })
            .unwrap())
        }
    }

    /// The agent bounds a page by encoded bytes and rejects a row limit above
    /// its own maximum, so this asks for as many rows as it will serve. A
    /// thousand-item store is then one round trip rather than five.
    #[test]
    fn a_catalog_page_asks_for_far_more_than_one_screen_of_rows() {
        // A page an agent refuses would fail every store load until that
        // agent exits, so this pins the value rather than the intent.
        let transport = LimitRecordingTransport {
            limits: Mutex::new(Vec::new()),
        };
        load_store_pages(
            &transport,
            &CatalogStoreRef::Account(AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            false,
            &CatalogLoadToken::default(),
        )
        .unwrap();
        // The agent's own maximum, which the version-mismatch takeover makes
        // safe to ask for: every agent this desktop can reach accepts it.
        assert_eq!(*transport.limits.lock().unwrap(), vec![4096]);
    }

    struct PagedCatalogTransport {
        calls: Mutex<Vec<(Option<String>, bool)>>,
    }

    impl AgentTransport for PagedCatalogTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            let Operation::ListKv { cursor, fresh, .. } = operation else {
                panic!("unexpected operation")
            };
            let mut calls = self.calls.lock().unwrap();
            calls.push((cursor.clone(), fresh));
            let page = calls.len();
            Ok(serde_json::to_value(KvPage {
                snapshot_version: 7,
                entries: vec![KvEntryMetadata {
                    dirent_id: [7; 16],
                    path: format!("/{page}"),
                    node_type: "small-file".to_owned(),
                    version: 1,
                    size: None,
                    read_role: KvRole::Owner,
                    write_role: KvRole::Owner,
                    shape: None,
                }],
                next_cursor: (page < 3).then(|| format!("cursor-{page}")),
            })
            .unwrap())
        }
    }

    #[test]
    fn a_fresh_store_walk_asks_for_the_first_page_only() {
        for fresh in [false, true] {
            let transport = PagedCatalogTransport {
                calls: Mutex::new(Vec::new()),
            };
            let entries = load_store_pages(
                &transport,
                &CatalogStoreRef::Account(AccountStoreRef {
                    profile: "local".to_owned(),
                    account_alias: "personal".to_owned(),
                }),
                fresh,
                &CatalogLoadToken::default(),
            )
            .unwrap();
            assert_eq!(entries.len(), 3);
            assert_eq!(
                *transport.calls.lock().unwrap(),
                vec![
                    (None, fresh),
                    (Some("cursor-1".to_owned()), false),
                    (Some("cursor-2".to_owned()), false),
                ]
            );
        }
    }

    struct ChangedCatalogTransport {
        cursors: Mutex<Vec<Option<String>>>,
    }

    impl AgentTransport for ChangedCatalogTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            let Operation::ListKv { cursor, .. } = operation else {
                panic!("unexpected operation")
            };
            let mut cursors = self.cursors.lock().unwrap();
            cursors.push(cursor.clone());
            let call = cursors.len();
            match call {
                1 => Ok(serde_json::to_value(KvPage {
                    snapshot_version: 7,
                    entries: vec![KvEntryMetadata {
                        dirent_id: [7; 16],
                        path: "/stale".to_owned(),
                        node_type: "small-file".to_owned(),
                        version: 1,
                        size: None,
                        read_role: KvRole::Owner,
                        write_role: KvRole::Owner,
                        shape: None,
                    }],
                    next_cursor: Some("stale-cursor".to_owned()),
                })
                .unwrap()),
                2 => Err(AgentError::Protocol {
                    code: ErrorCode::CatalogSnapshotChanged,
                    message: "wording is deliberately irrelevant".to_owned(),
                    fields: Box::default(),
                }),
                3 => Ok(serde_json::to_value(KvPage {
                    snapshot_version: 8,
                    entries: vec![KvEntryMetadata {
                        dirent_id: [7; 16],
                        path: "/fresh".to_owned(),
                        node_type: "small-file".to_owned(),
                        version: 1,
                        size: None,
                        read_role: KvRole::Owner,
                        write_role: KvRole::Owner,
                        shape: None,
                    }],
                    next_cursor: Some("fresh-cursor".to_owned()),
                })
                .unwrap()),
                4 => Ok(serde_json::to_value(KvPage {
                    snapshot_version: 8,
                    entries: vec![KvEntryMetadata {
                        dirent_id: [7; 16],
                        path: "/second".to_owned(),
                        node_type: "small-file".to_owned(),
                        version: 1,
                        size: None,
                        read_role: KvRole::Owner,
                        write_role: KvRole::Owner,
                        shape: None,
                    }],
                    next_cursor: None,
                })
                .unwrap()),
                _ => panic!("unexpected page call"),
            }
        }
    }

    #[test]
    fn catalog_restarts_once_when_a_cursor_snapshot_changes() {
        let transport = ChangedCatalogTransport {
            cursors: Mutex::new(Vec::new()),
        };
        let entries = load_store_pages(
            &transport,
            &CatalogStoreRef::Account(AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            false,
            &CatalogLoadToken::default(),
        )
        .unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            ["/fresh", "/second"]
        );
        assert_eq!(
            transport.cursors.into_inner().unwrap(),
            [
                None,
                Some("stale-cursor".to_owned()),
                None,
                Some("fresh-cursor".to_owned())
            ]
        );
    }

    #[test]
    fn empty_full_reads_have_explicit_root_and_profile_evidence() {
        struct EmptyCatalogTransport(bool);
        impl AgentTransport for EmptyCatalogTransport {
            fn call(&self, operation: Operation) -> Result<Value, AgentError> {
                match operation {
                    Operation::ListProfiles => Ok(if self.0 {
                        serde_json::json!([{"name":"local"}])
                    } else {
                        serde_json::json!([])
                    }),
                    Operation::ListKnownStores { .. } => Ok(serde_json::json!([])),
                    Operation::ListProfileOverview { profile } => Ok(profile_overview(
                        &profile,
                        success(serde_json::json!([])),
                        success(serde_json::json!([])),
                    )),
                    operation => panic!("unexpected empty catalog operation: {operation:?}"),
                }
            }
        }
        assert_eq!(CatalogSnapshot::default().full_item_reads, None);
        assert_eq!(
            load_stores(Arc::new(EmptyCatalogTransport(false)))
                .unwrap()
                .full_item_reads,
            None
        );
        assert_eq!(
            load_catalog(Arc::new(EmptyCatalogTransport(false)))
                .unwrap()
                .full_item_reads,
            Some(vec![])
        );
        assert_eq!(
            load_stores(Arc::new(EmptyCatalogTransport(true)))
                .unwrap()
                .full_item_reads,
            None
        );
        let profile = load_profile_catalog_cancellable(
            Arc::new(EmptyCatalogTransport(true)),
            "local".into(),
            CatalogLoadToken::default(),
            false,
        )
        .unwrap();
        assert_eq!(profile.full_item_reads, Some(vec!["local".into()]));
        assert!(profile.items.is_empty());
        assert!(profile.failures.is_empty());
    }

    #[test]
    fn stores_discovery_never_walks_a_kv_tree() {
        let catalog = load_stores(Arc::new(AccountCatalogTransport)).unwrap();
        assert_eq!(catalog.full_item_reads, None);
        assert_eq!(catalog.stores.len(), 1);
        assert!(catalog.items.is_empty());
        assert!(catalog.failures.is_empty());
    }

    struct LockAwareCatalogTransport {
        active_profiles: Mutex<std::collections::HashSet<String>>,
        active: std::sync::atomic::AtomicUsize,
        maximum: std::sync::atomic::AtomicUsize,
    }

    impl AgentTransport for LockAwareCatalogTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            let (profile, overview) = match operation {
                Operation::ListProfiles => {
                    return Ok(serde_json::json!([{"name": "one"}, {"name": "two"}]))
                }
                Operation::ListKnownStores { profile } => (profile, false),
                Operation::ListProfileOverview { profile } => (profile, true),
                operation => panic!("unexpected lock-aware operation: {operation:?}"),
            };
            {
                let mut active = self.active_profiles.lock().unwrap();
                if !active.insert(profile.clone()) {
                    return Err(AgentError::Protocol {
                        code: ErrorCode::ProfileBusy,
                        message: "same-profile overlap".to_owned(),
                        fields: Box::default(),
                    });
                }
            }
            let count = self
                .active
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.maximum
                .fetch_max(count, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.active
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            self.active_profiles.lock().unwrap().remove(&profile);
            Ok(if overview {
                profile_overview(
                    &profile,
                    success(serde_json::json!([])),
                    success(serde_json::json!([])),
                )
            } else {
                serde_json::json!([])
            })
        }
    }

    #[test]
    fn catalog_serializes_each_profile_while_parallelizing_distinct_profiles() {
        let transport = Arc::new(LockAwareCatalogTransport {
            active_profiles: Mutex::new(std::collections::HashSet::new()),
            active: std::sync::atomic::AtomicUsize::new(0),
            maximum: std::sync::atomic::AtomicUsize::new(0),
        });
        let catalog = load_catalog(transport.clone()).unwrap();
        assert!(catalog.failures.is_empty());
        assert!(transport.maximum.load(std::sync::atomic::Ordering::SeqCst) > 1);
    }

    struct CancellingCatalogTransport(CatalogLoadToken);

    impl AgentTransport for CancellingCatalogTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            match operation {
                Operation::ListProfiles => Ok(serde_json::json!([{"name": "local"}])),
                Operation::ListKnownStores { .. } => Ok(serde_json::json!([])),
                Operation::ListProfileOverview { profile } => {
                    self.0.cancel();
                    Ok(profile_overview(
                        &profile,
                        success(serde_json::json!([])),
                        success(serde_json::json!([])),
                    ))
                }
                operation => panic!("canceled catalog launched another call: {operation:?}"),
            }
        }
    }

    #[test]
    fn canceled_catalog_stops_before_its_next_profile_call() {
        let token = CatalogLoadToken::default();
        let result = load_catalog_cancellable(
            Arc::new(CancellingCatalogTransport(token.clone())),
            token,
            false,
        );
        assert_eq!(result.unwrap_err(), AgentError::Cancelled);
    }

    struct LargeReadTransport {
        store: KvStoreRef,
        total: u64,
        operations: Mutex<Vec<Operation>>,
    }

    impl AgentTransport for LargeReadTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            self.operations.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ReadKv {
                    store,
                    path,
                    dirent_id,
                    version,
                } => {
                    assert_eq!(store, self.store);
                    assert_eq!(path, "/large");
                    assert_eq!(version, 9);
                    Ok(serde_json::to_value(KvReadResult {
                        store,
                        path,
                        dirent_id,
                        version,
                        node_type: "file".to_owned(),
                        // As the agent answers now: a large file has no size
                        // it can report without downloading itself.
                        size: None,
                        read_role: KvRole::Member { visibility: 2 },
                        write_role: KvRole::Admin,
                        content: None,
                        symlink_target: None,
                    })
                    .unwrap())
                }
                Operation::ReadKvChunk {
                    store,
                    path,
                    dirent_id,
                    version,
                    offset,
                    length,
                } => {
                    assert_eq!(store, self.store);
                    assert_eq!(path, "/large");
                    assert_eq!(version, 9);
                    let count = (self.total - offset).min(u64::from(length)) as usize;
                    Ok(serde_json::to_value(KvChunkResult {
                        store,
                        path,
                        dirent_id,
                        version,
                        offset,
                        content: (0..count)
                            .map(|index| ((offset as usize + index) % 251) as u8)
                            .collect(),
                        eof: offset + count as u64 == self.total,
                    })
                    .unwrap())
                }
                operation => panic!("unexpected item read: {operation:?}"),
            }
        }
    }

    #[test]
    fn large_item_reads_are_chunked_and_bound_to_the_catalog_version() {
        let account = AccountStoreRef {
            profile: "local".to_owned(),
            account_alias: "personal".to_owned(),
        };
        let item = CatalogItem {
            store: CatalogStoreRef::Account(account.clone()),
            metadata: KvEntryMetadata {
                dirent_id: [7; 16],
                path: "/large".to_owned(),
                node_type: "file".to_owned(),
                version: 9,
                size: None,
                read_role: KvRole::Member { visibility: 2 },
                write_role: KvRole::Admin,
                shape: None,
            },
        };
        let transport = LargeReadTransport {
            store: KvStoreRef::Account(account),
            total: u64::from(KV_READ_CHUNK_BYTES) + 17,
            operations: Mutex::new(Vec::new()),
        };
        let read = read_catalog_item(&transport, &item).unwrap();
        let KvItemValue::File(content) = read.value else {
            panic!("large file did not return file content")
        };
        assert_eq!(content.len(), KV_READ_CHUNK_BYTES as usize + 17);
        assert_eq!(content[0], 0);
        assert_eq!(
            content[KV_READ_CHUNK_BYTES as usize],
            (KV_READ_CHUNK_BYTES as usize % 251) as u8
        );
        assert_eq!(transport.operations.lock().unwrap().len(), 3);
    }

    fn large_file_item() -> CatalogItem {
        CatalogItem {
            store: CatalogStoreRef::Account(AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            metadata: KvEntryMetadata {
                dirent_id: [7; 16],
                path: "/large".to_owned(),
                node_type: "file".to_owned(),
                version: 9,
                size: None,
                read_role: KvRole::Member { visibility: 2 },
                write_role: KvRole::Admin,
                shape: None,
            },
        }
    }

    fn large_read_transport(total: u64) -> LargeReadTransport {
        LargeReadTransport {
            store: KvStoreRef::Account(AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            total,
            operations: Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn selected_read_rejects_recreated_identity_at_initial_read_and_later_chunk() {
        struct Replaced {
            inner: LargeReadTransport,
            after_chunks: bool,
        }
        impl AgentTransport for Replaced {
            fn call(&self, operation: Operation) -> Result<Value, AgentError> {
                let corrupt = if self.after_chunks {
                    matches!(&operation, Operation::ReadKvChunk { offset, .. } if *offset > 0)
                } else {
                    matches!(&operation, Operation::ReadKv { .. })
                };
                let mut value = self.inner.call(operation)?;
                if corrupt {
                    value["dirent_id"] = serde_json::to_value([8; 16]).unwrap();
                }
                Ok(value)
            }
        }
        for after_chunks in [false, true] {
            let transport = Replaced {
                inner: large_read_transport(2 * u64::from(KV_READ_CHUNK_BYTES)),
                after_chunks,
            };
            assert!(read_catalog_item(&transport, &large_file_item()).is_err());
        }
    }

    /// With no size reported, the end-of-file flag is the only thing that
    /// ends the loop. The file here is an exact multiple of the chunk size,
    /// so the final chunk is full and is distinguishable from a non-final one
    /// by nothing but that flag.
    #[test]
    fn a_large_read_ends_on_the_end_of_file_flag_when_no_size_is_reported() {
        let total = 2 * u64::from(KV_READ_CHUNK_BYTES);
        let transport = large_read_transport(total);
        let read = read_catalog_item(&transport, &large_file_item()).unwrap();
        let KvItemValue::File(content) = read.value else {
            panic!("large file did not return file content")
        };
        assert_eq!(content.len() as u64, total);
        let operations = transport.operations.lock().unwrap();
        // One node read and exactly two chunk reads: no size probe, and no
        // further request once the flag arrived.
        assert_eq!(operations.len(), 3);
        assert!(matches!(operations[0], Operation::ReadKv { .. }));
        assert!(operations[1..]
            .iter()
            .all(|operation| matches!(operation, Operation::ReadKvChunk { .. })));
    }

    /// The reveal cap is now a cap on what has accumulated. A file past it is
    /// refused part-way through rather than before the first chunk, and the
    /// loop cannot be run forever by an agent that never sets the flag.
    #[test]
    fn a_large_read_stops_at_the_reveal_cap_while_it_accumulates() {
        let transport = large_read_transport(MAXIMUM_DESKTOP_REVEAL_BYTES + 1);
        let error = read_catalog_item(&transport, &large_file_item()).unwrap_err();
        assert!(
            matches!(&error, AgentError::Transport(message) if message.contains("preview limit")),
            "{error:?}"
        );
        let requested = u64::from(KV_READ_CHUNK_BYTES)
            * (transport.operations.lock().unwrap().len() as u64 - 1);
        assert!(requested <= MAXIMUM_DESKTOP_REVEAL_BYTES + u64::from(KV_READ_CHUNK_BYTES));
    }

    #[test]
    fn account_and_team_item_mutations_are_cas_bound() {
        let account = CatalogStoreRef::Account(AccountStoreRef {
            profile: "local".to_owned(),
            account_alias: "personal".to_owned(),
        });
        let item = CatalogItem {
            store: account.clone(),
            metadata: KvEntryMetadata {
                dirent_id: [7; 16],
                path: "/password".to_owned(),
                node_type: "small-file".to_owned(),
                version: 7,
                size: None,
                read_role: KvRole::Member { visibility: -1 },
                write_role: KvRole::Admin,
                shape: None,
            },
        };
        assert!(matches!(
            create_kv_file_mutation(&account, "/new", b"secret".to_vec()).unwrap(),
            KvAccountMutation::Inline(Operation::PutKv {
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                precondition: KvPrecondition::Create,
                mkdir_p: true,
                ..
            })
        ));
        assert!(matches!(
            edit_kv_file_mutation(&item, b"replacement".to_vec()).unwrap(),
            KvAccountMutation::Inline(Operation::PutKv {
                read_role: KvRole::Member { visibility: -1 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::ExactEntry { version: 7, .. },
                mkdir_p: false,
                ..
            })
        ));
        assert!(matches!(
            edit_kv_file_mutation(&item, vec![0; MAXIMUM_INLINE_KV_BYTES + 1]).unwrap(),
            KvAccountMutation::Stream {
                header: KvUploadHeader {
                    upload_id: None,
                    adapter: None,
                    precondition: KvPrecondition::ExactEntry { version: 7, .. },
                    read_role: KvRole::Member { visibility: -1 },
                    write_role: KvRole::Admin,
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            remove_kv_operation(&item, false).unwrap(),
            Operation::RemoveKv {
                precondition: KvPrecondition::ExactVersion { version: 7 },
                ..
            }
        ));
        assert!(matches!(
            create_kv_file_upload(&account, "/large", 84 * 1024 * 1024).unwrap(),
            KvUploadHeader {
                upload_id: None,
                adapter: None,
                total_length: 88_080_384,
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                precondition: KvPrecondition::Create,
                mkdir_p: true,
                ..
            }
        ));
        let mut file = item.clone();
        file.metadata.node_type = "file".to_owned();
        assert!(matches!(
            edit_kv_file_upload(&file, 64).unwrap(),
            KvUploadHeader {
                upload_id: None,
                adapter: None,
                total_length: 64,
                read_role: KvRole::Member { visibility: -1 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::ExactEntry { version: 7, .. },
                mkdir_p: false,
                ..
            }
        ));
        assert!(matches!(
            edit_kv_file_upload(&item, 64).unwrap(),
            KvUploadHeader {
                upload_id: None,
                adapter: None,
                total_length: 64,
                read_role: KvRole::Member { visibility: -1 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::ExactEntry { version: 7, .. },
                ..
            }
        ));
        assert_eq!(
            create_kv_symlink_operation(&account, "/link ", " target ").unwrap(),
            Operation::PutKvSymlink {
                store: kv_store_ref(&account),
                path: "/link ".to_owned(),
                target: " target ".to_owned(),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                precondition: KvPrecondition::Create,
                mkdir_p: true,
            }
        );
        let mut symlink = item.clone();
        symlink.metadata.node_type = "symlink".to_owned();
        assert_eq!(
            edit_kv_symlink_operation(&symlink, "replacement").unwrap_err(),
            "symlink targets cannot be edited directly; delete and recreate the symlink"
        );

        let team = CatalogStoreRef::Team(TeamStoreRef {
            profile: "local".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "aa".to_owned(),
        });
        assert!(matches!(
            create_kv_file_mutation(&team, "/shared", Vec::new()).unwrap(),
            KvAccountMutation::Inline(Operation::PutKv {
                store: KvStoreRef::Team(_),
                precondition: KvPrecondition::Create,
                mkdir_p: true,
                ..
            })
        ));
    }

    #[test]
    fn catalog_worker_bound_never_exceeds_four() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        let output = run_bounded((0..12).collect(), |value| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(2));
            active.fetch_sub(1, Ordering::SeqCst);
            value
        });
        assert_eq!(output, (0..12).collect::<Vec<_>>());
        assert_eq!(maximum.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn agent_error_classification_keeps_ambiguous_and_fatal_failures_distinct() {
        let deadline = AgentError::Protocol {
            code: ErrorCode::DeadlineExceeded,
            message: "late".to_owned(),
            fields: Box::default(),
        };
        assert!(deadline.transient());
        assert!(deadline.ambiguous());
        assert!(!deadline.fatal());
        assert!(AgentError::Protocol {
            code: ErrorCode::RateLimited,
            message: "wait".to_owned(),
            fields: Box::default(),
        }
        .transient());
        assert!(!AgentError::Protocol {
            code: ErrorCode::QuotaExceeded,
            message: "full".to_owned(),
            fields: Box::default(),
        }
        .transient());

        let mismatch = AgentError::Protocol {
            code: ErrorCode::VersionMismatch,
            message: "upgrade".to_owned(),
            fields: Box::default(),
        };
        assert!(mismatch.fatal());
        assert!(!mismatch.transient());
        assert!(!mismatch.ambiguous());

        assert!(AgentError::Transport("socket closed".to_owned()).transient());
        let upload = AgentError::Ambiguous("socket closed after commit".to_owned());
        assert!(upload.transient());
        assert!(upload.ambiguous());
        assert_eq!(AgentError::Cancelled.user_message(), "Request cancelled.");

        let decoded_mismatch = agent_client_error(foks_agent_client::Error::Protocol(
            foks_agent_proto::Error::Version,
        ));
        assert!(decoded_mismatch.fatal());

        // The frozen refusal names the agent's versions and is definite: the
        // agent did not decode the request, mutation or not.
        let refused = agent_client_error(foks_agent_client::Error::Protocol(
            foks_agent_proto::Error::VersionMismatch {
                agent: 34,
                supported: Some(foks_agent_proto::ProtocolRange::exactly(34)),
            },
        ));
        assert!(refused.fatal());
        assert!(!refused.ambiguous());
        assert!(!refused.transient());
        let AgentError::Protocol {
            code: ErrorCode::VersionMismatch,
            message,
            ..
        } = &refused
        else {
            panic!("unexpected mapping {refused:?}");
        };
        assert_eq!(
            *message,
            format!(
                "desktop protocol version {} does not match agent protocol version 34",
                foks_agent_proto::PROTOCOL_VERSION
            )
        );
    }

    #[test]
    fn team_store_identity_includes_the_owning_account_alias() {
        let left = CatalogStoreRef::Team(TeamStoreRef {
            profile: "local".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "0d".repeat(33),
        });
        let right = CatalogStoreRef::Team(TeamStoreRef {
            profile: "local".to_owned(),
            account_alias: "work".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "0d".repeat(33),
        });
        assert_ne!(left, right);
    }

    #[test]
    fn crash_markers_are_private_and_exclude_failure_payloads() {
        let temporary = tempfile::tempdir().unwrap();
        let marker = write_crash_marker(&temporary.path().join("crashes")).unwrap();
        let contents = fs::read_to_string(marker).unwrap();
        assert!(contents.contains("foks-desktop-version="));
        assert!(!contents.contains("panic"));
        assert!(!contents.contains(temporary.path().to_string_lossy().as_ref()));
    }
}

#[cfg(test)]
mod chat_transport_tests {
    use super::*;

    #[test]
    fn fallback_cancellation_preserves_started_mutation_uncertainty() {
        struct Transport(AtomicBool);
        impl AgentTransport for Transport {
            fn call(&self, _: Operation) -> Result<Value, AgentError> {
                self.0.store(true, Ordering::Release);
                Ok(Value::Null)
            }
        }
        let transport = Transport(AtomicBool::new(false));
        let cancelled = || transport.0.load(Ordering::Acquire);
        let error = transport
            .call_cancellable(
                Operation::SyncAccount {
                    profile: "test".into(),
                    alias: "me".into(),
                },
                &cancelled,
            )
            .unwrap_err();
        assert!(matches!(error, AgentError::Ambiguous(_)));
        transport.0.store(false, Ordering::Release);
        assert_eq!(
            transport
                .call_cancellable(Operation::Ping, &cancelled)
                .unwrap_err(),
            AgentError::Cancelled
        );
    }
}

fn new_upload_id(_length: u64) -> Result<Option<foks_agent_proto::UploadHandle>, &'static str> {
    // Issuance requires the profile's checked hard-state generation, obtained by
    // ensure_upload_identity immediately before using the source.
    Ok(None)
}

/// Allocate and retain the complete handle before registration or source reads.
pub fn ensure_upload_identity(
    transport: &dyn AgentTransport,
    header: &mut KvUploadHeader,
) -> Result<(), AgentError> {
    if header.upload_id.is_none() && header.adapter.is_none() {
        let value = transport.call(Operation::KvUploadIssuance {
            store: header.store.clone(),
        })?;
        let context: foks_agent_proto::KvUploadIssuance = serde_json::from_value(value)
            .map_err(|_| AgentError::Transport("invalid upload issuance context".into()))?;
        let mut random = [0; 16];
        getrandom::fill(&mut random)
            .map_err(|_| AgentError::Transport("upload randomness unavailable".into()))?;
        header.upload_id = Some(foks_agent_proto::UploadHandle::new(
            context.database_id,
            context.issued_at,
            random,
        ));
    }
    Ok(())
}
