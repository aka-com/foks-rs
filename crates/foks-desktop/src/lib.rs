//! Screen state and agent operations for the FOKS desktop application.

#![forbid(unsafe_code)]

mod chat;
pub use chat::{chat_request, chat_request_cancellable, validate_chat_reply};

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::{fs, path::PathBuf};

use foks_agent_client::AgentClient;
use foks_agent_proto::{
    AccountStoreRef, AccountSummary, ErrorCode, ErrorFields, FederationRole, KnownStoreSummary,
    KvChunkResult, KvEntryMetadata, KvPage, KvPrecondition, KvReadResult, KvRole, KvStoreRef,
    KvUploadHeader, Operation, ProfileOverview, ProfileProtocol, ProfileTrust, ResponseResult,
    SecretString, ServerStatusSnapshot, TeamKind, TeamRole, TeamStoreRef, TeamSummary,
    YubiRetryConfiguration,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeroize::{Zeroize as _, Zeroizing};

// Local protocol v2 encodes byte vectors as JSON integer arrays; these bounds
// keep worst-case payloads comfortably below its 1 MiB frame ceiling.
pub const MAXIMUM_INLINE_KV_BYTES: usize = 128 * 1024;
const KV_READ_CHUNK_BYTES: u32 = 128 * 1024;
const MAXIMUM_DESKTOP_REVEAL_BYTES: u64 = 16 * 1024 * 1024;

/// Typed desktop-side classification of local-agent failures.
///
/// Desktop error variants mapped from local agent and transport failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentError {
    Protocol {
        code: ErrorCode,
        message: String,
        fields: ErrorFields,
    },
    Transport(String),
    Local(LocalAgentCondition),
    Ambiguous(String),
    Cancelled,
    DeadlineExceeded,
    Ipc {
        code: &'static str,
        message: String,
        ambiguous: bool,
        connection_lost: bool,
    },
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
                | Self::DeadlineExceeded
                | Self::Ipc {
                    connection_lost: true,
                    ..
                }
        )
    }

    pub fn fatal(&self) -> bool {
        matches!(
            self,
            Self::Protocol {
                code: ErrorCode::VersionMismatch,
                ..
            } | Self::Ipc {
                code: "unsafe-socket" | "protocol" | "response-binding" | "version-mismatch",
                ..
            }
        )
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Items,
    Notifications,
    GetStarted,
    Stores,
    Parties,
    Servers,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassphraseAction {
    Set,
    Change,
    Verify,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YubiAction {
    ListCards,
    ResumeAccount,
    Sync,
    /// Sync plus every federated security responder this key can drive.
    SyncWithFederation,
    PinStatus,
    RotateManagementKey,
    ResumeManagementKey,
    RecoverManagementKey,
    RecoverSubkey,
    Revoke,
}

impl Screen {
    pub const ALL: [Self; 7] = [
        Self::Items,
        Self::Notifications,
        Self::GetStarted,
        Self::Stores,
        Self::Parties,
        Self::Servers,
        Self::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Items => "Items",
            Self::Notifications => "Notifications",
            Self::GetStarted => "Get Started",
            Self::Stores => "Stores",
            Self::Parties => "Parties",
            Self::Servers => "Servers",
            Self::Settings => "Settings",
        }
    }
}

pub trait AgentTransport: Send + Sync + 'static {
    fn call(&self, operation: Operation) -> Result<Value, AgentError>;
    fn call_cancellable(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, AgentError> {
        if cancelled() {
            return Err(AgentError::Cancelled);
        }
        let mutation = operation.is_mutation();
        let result = self.call(operation);
        if result.is_ok() && cancelled() {
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
                fields,
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
                fields,
            }),
        }
    }
}

pub fn agent_client_error(error: foks_agent_client::Error) -> AgentError {
    use foks_agent_client::Error;
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
        Error::Ambiguous(cause) => {
            let cause = agent_client_error(*cause);
            let code = match &cause {
                AgentError::Ipc { code, .. } => *code,
                AgentError::Protocol {
                    code: ErrorCode::VersionMismatch,
                    ..
                } => "version-mismatch",
                _ => "ambiguous",
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
            fields: ErrorFields::default(),
        },
        Error::Io(_) if connection_lost => AgentError::Transport(message),
        Error::Io(_) => ipc("io"),
        Error::UploadSource(_) => ipc("upload-source"),
        Error::UnsafeSocket => ipc("unsafe-socket"),
        Error::Protocol(_) => ipc("protocol"),
        Error::ResponseBinding => ipc("response-binding"),
        Error::Unsupported => ipc("unsupported"),
    }
}

#[cfg(test)]
mod ipc_error_tests {
    use super::*;

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
                code: "unsafe-socket",
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
            fields,
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogSnapshot {
    pub profiles: Vec<String>,
    pub stores: Vec<CatalogStoreSummary>,
    pub known_stores: Vec<CatalogStoreSummary>,
    pub inventory: Vec<CatalogInventoryState>,
    pub profile_overviews: Vec<ProfileOverview>,
    pub items: Vec<CatalogItem>,
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
    load_catalog_with_token(transport, true, CatalogLoadToken::default())
}

/// Loads account and team discovery without walking any KV tree.
pub fn load_stores(transport: Arc<dyn AgentTransport>) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_with_token(transport, false, CatalogLoadToken::default())
}

pub fn load_catalog_cancellable(
    transport: Arc<dyn AgentTransport>,
    token: CatalogLoadToken,
) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_with_token(transport, true, token)
}

pub fn load_profile_catalog_cancellable(
    transport: Arc<dyn AgentTransport>,
    profile: String,
    token: CatalogLoadToken,
) -> Result<CatalogSnapshot, AgentError> {
    token.check()?;
    let profiles: Vec<ProfileSummary> =
        decode_agent_value(transport.call(Operation::ListProfiles)?)?;
    token.check()?;
    if profiles
        .iter()
        .filter(|candidate| candidate.name == profile)
        .count()
        != 1
    {
        return Err(AgentError::Transport(
            "profile is missing or duplicated in the configured profiles".to_owned(),
        ));
    }
    let mut snapshot = load_profile_catalog(transport.as_ref(), profile.clone(), true, token)?;
    snapshot.profiles.push(profile);
    Ok(snapshot)
}

pub fn load_stores_cancellable(
    transport: Arc<dyn AgentTransport>,
    token: CatalogLoadToken,
) -> Result<CatalogSnapshot, AgentError> {
    load_catalog_with_token(transport, false, token)
}

fn load_catalog_with_token(
    transport: Arc<dyn AgentTransport>,
    include_items: bool,
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
    load_catalog_profiles(transport, profiles, include_items, token, &|_| {})
}

pub fn load_catalog_progressive_cancellable(
    transport: Arc<dyn AgentTransport>,
    token: CatalogLoadToken,
    publish: impl Fn(&CatalogSnapshot) + Sync,
) -> Result<CatalogSnapshot, AgentError> {
    token.check()?;
    let profiles: Vec<ProfileSummary> = decode_agent_value(
        transport.call_cancellable(Operation::ListProfiles, &|| token.check().is_err())?,
    )?;
    load_catalog_profiles(
        transport,
        profiles.into_iter().map(|profile| profile.name).collect(),
        true,
        token,
        &publish,
    )
}

fn load_catalog_profiles(
    transport: Arc<dyn AgentTransport>,
    profiles: Vec<String>,
    include_items: bool,
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
    token: CatalogLoadToken,
) -> Result<CatalogSnapshot, AgentError> {
    load_profile_catalog_progress(transport, profile, include_items, token, &|_| {})
}

fn load_profile_catalog_progress(
    transport: &dyn AgentTransport,
    profile: String,
    include_items: bool,
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
    if include_items {
        publish_local(&CatalogSnapshot {
            known_stores: snapshot.known_stores.clone(),
            failures: snapshot.failures.clone(),
            blocked_profiles: snapshot.blocked_profiles.clone(),
            ..CatalogSnapshot::default()
        });
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
        match load_store_pages(transport, &store, &token) {
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
    match error {
        AgentError::Protocol {
            code: ErrorCode::RollbackDetected | ErrorCode::CheckpointResetRequired,
            ..
        } => true,
        _ => false,
    }
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
        version: item.metadata.version,
    })?;
    let mut read: KvReadResult = decode_agent_value(value)?;
    if read.store != store
        || read.path != item.metadata.path
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
            let total = read.size.ok_or_else(|| {
                AgentError::Transport("missing file size in large-file response".to_owned())
            })?;
            if total > MAXIMUM_DESKTOP_REVEAL_BYTES {
                return Err(AgentError::Transport(format!(
                    "File size of {total} bytes exceeds maximum in-memory preview limit of {MAXIMUM_DESKTOP_REVEAL_BYTES} bytes"
                )));
            }
            let capacity = usize::try_from(total).map_err(|_| {
                AgentError::Transport("file size exceeds system memory address space".to_owned())
            })?;
            let mut content = Zeroizing::new(Vec::with_capacity(capacity));
            let mut offset = 0u64;
            while offset < total {
                let remaining = total - offset;
                let length = u32::try_from(remaining.min(u64::from(KV_READ_CHUNK_BYTES)))
                    .expect("chunk bound fits in u32");
                let mut chunk: KvChunkResult =
                    decode_agent_value(transport.call(Operation::ReadKvChunk {
                        store: store.clone(),
                        path: item.metadata.path.clone(),
                        version: item.metadata.version,
                        offset,
                        length,
                    })?)?;
                let invalid = chunk.store != store
                    || chunk.path != item.metadata.path
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
                if offset > total || chunk.eof != (offset == total) {
                    chunk.content.zeroize();
                    return Err(AgentError::Transport(
                        "inconsistent end-of-file indicator in chunk response".to_owned(),
                    ));
                }
                content.extend_from_slice(&chunk.content);
                chunk.content.zeroize();
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
        KvPrecondition::ExactVersion {
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
        KvPrecondition::ExactVersion {
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

pub fn remove_kv_operation(item: &CatalogItem, recursive: bool) -> Result<Operation, &'static str> {
    Ok(Operation::RemoveKv {
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
        },
    }
}

fn load_store_pages(
    transport: &dyn AgentTransport,
    store: &CatalogStoreRef,
    token: &CatalogLoadToken,
) -> Result<Vec<KvEntryMetadata>, AgentError> {
    match load_store_pages_once(transport, store, token) {
        Err(error) if catalog_cursor_snapshot_changed(&error) => {
            token.check()?;
            load_store_pages_once(transport, store, token)
        }
        result => result,
    }
}

fn load_store_pages_once(
    transport: &dyn AgentTransport,
    store: &CatalogStoreRef,
    token: &CatalogLoadToken,
) -> Result<Vec<KvEntryMetadata>, AgentError> {
    const PAGE_LIMIT: u32 = 200;
    const MAXIMUM_PAGES: usize = 4096;
    let mut cursor = None;
    let mut snapshot_version = None;
    let mut entries = Vec::new();
    for _ in 0..MAXIMUM_PAGES {
        token.check()?;
        let operation = match store {
            CatalogStoreRef::Account(store) => Operation::ListKv {
                store: store.clone(),
                cursor: cursor.clone(),
                limit: PAGE_LIMIT,
            },
            CatalogStoreRef::Team(store) => Operation::ListTeamKv {
                store: store.clone(),
                cursor: cursor.clone(),
                limit: PAGE_LIMIT,
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
    AgentError::Transport(message.to_owned())
}

fn catalog_accounts(catalog: &CatalogSnapshot, profile: Option<&str>) -> Vec<String> {
    catalog
        .stores
        .iter()
        .filter_map(|store| match store {
            CatalogStoreSummary::Account { store } if Some(store.profile.as_str()) == profile => {
                Some(store.account_alias.clone())
            }
            _ => None,
        })
        .collect()
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

pub struct DesktopModel {
    transport: Arc<dyn AgentTransport>,
    screen: Screen,
    selected_profile: Option<String>,
    selected_account: Option<String>,
    profiles: Vec<String>,
    accounts: Vec<String>,
    catalog: Option<CatalogSnapshot>,
    value: Option<Value>,
    error: Option<String>,
}

impl DesktopModel {
    pub fn new(transport: Arc<dyn AgentTransport>) -> Self {
        Self {
            transport,
            screen: Screen::Items,
            selected_profile: None,
            selected_account: None,
            profiles: Vec::new(),
            accounts: Vec::new(),
            catalog: None,
            value: None,
            error: None,
        }
    }

    pub fn transport(&self) -> Arc<dyn AgentTransport> {
        Arc::clone(&self.transport)
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn selected_profile(&self) -> Option<&str> {
        self.selected_profile.as_deref()
    }

    pub fn selected_account(&self) -> Option<&str> {
        self.selected_account.as_deref()
    }

    pub fn profiles(&self) -> &[String] {
        &self.profiles
    }

    pub fn accounts(&self) -> &[String] {
        &self.accounts
    }

    pub fn has_any_accounts(&self) -> bool {
        self.catalog.as_ref().is_some_and(|catalog| {
            catalog
                .stores
                .iter()
                .any(|store| matches!(store, CatalogStoreSummary::Account { .. }))
        }) || !self.accounts.is_empty()
    }

    pub fn catalog(&self) -> Option<&CatalogSnapshot> {
        self.catalog.as_ref()
    }

    pub fn accept_catalog(&mut self, result: Result<CatalogSnapshot, AgentError>) {
        match result {
            Ok(catalog) => {
                self.profiles = catalog.profiles.clone();
                if self
                    .selected_profile
                    .as_ref()
                    .is_none_or(|selected| !self.profiles.contains(selected))
                {
                    self.selected_profile = self.profiles.first().cloned();
                }
                self.accounts = catalog_accounts(&catalog, self.selected_profile.as_deref());
                if self
                    .selected_account
                    .as_ref()
                    .is_none_or(|selected| !self.accounts.contains(selected))
                {
                    self.selected_account = self.accounts.first().cloned();
                }
                self.catalog = Some(catalog);
                self.error = None;
                self.value = None;
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.value = None;
            }
        }
    }

    pub fn value(&self) -> Option<&Value> {
        self.value.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn navigate(&mut self, screen: Screen) {
        self.screen = screen;
        self.value = None;
        self.error = None;
    }

    pub fn select_profile(&mut self, profile: impl Into<String>) {
        self.selected_profile = Some(profile.into());
        self.selected_account = None;
        self.accounts = self
            .catalog
            .as_ref()
            .map(|catalog| catalog_accounts(catalog, self.selected_profile.as_deref()))
            .unwrap_or_default();
        self.selected_account = self.accounts.first().cloned();
    }

    pub fn select_account(&mut self, account: impl Into<String>) {
        self.selected_account = Some(account.into());
    }

    pub fn probe_operation(&self) -> Result<Operation, &'static str> {
        Ok(Operation::Probe {
            profile: self.selected_profile.clone().ok_or("no profile selected")?,
        })
    }

    pub fn add_profile_operation(
        &self,
        name: &str,
        probe: &str,
    ) -> Result<Operation, &'static str> {
        Ok(Operation::AddProfile {
            name: required_text(name, "enter a profile name")?,
            probe: required_text(probe, "enter a probe address")?,
            protocol: ProfileProtocol::V019,
            trust: ProfileTrust::WebPki,
        })
    }

    pub fn remove_profile_operation(&self) -> Result<Operation, &'static str> {
        Ok(Operation::RemoveProfile {
            name: self
                .selected_profile
                .clone()
                .ok_or("select a profile first")?,
        })
    }

    pub fn reset_hard_state_operation(&self) -> Result<Operation, &'static str> {
        Err("hard reset is not supported from this screen")
    }

    pub fn resume_account_operation(&self, alias: &str) -> Result<Operation, &'static str> {
        Ok(Operation::ResumeAccount {
            profile: self
                .selected_profile
                .clone()
                .ok_or("select a profile first")?,
            alias: required_text(alias, "enter the local account alias")?,
        })
    }

    /// Records an account created this session so the selector reflects it
    /// without waiting for the next `ListAccounts` round trip.
    pub fn record_account(&mut self, alias: impl Into<String>) {
        let alias = alias.into();
        if !self.accounts.contains(&alias) {
            self.accounts.push(alias.clone());
        }
        self.selected_account = Some(alias);
    }

    pub fn record_profile(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !self.profiles.contains(&name) {
            self.profiles.push(name.clone());
            self.profiles.sort();
        }
        self.selected_profile = Some(name);
        self.selected_account = None;
        self.accounts.clear();
    }

    pub fn forget_profile(&mut self, name: &str) {
        self.profiles.retain(|profile| profile != name);
        if self.selected_profile.as_deref() == Some(name) {
            self.selected_profile = self.profiles.first().cloned();
            self.selected_account = None;
            self.accounts.clear();
        }
    }

    pub fn operation(&self) -> Result<Operation, &'static str> {
        let profile = || {
            self.selected_profile
                .clone()
                .ok_or("select a profile first")
        };
        match self.screen {
            Screen::Items | Screen::Stores => Err("catalog data must be refreshed directly"),
            Screen::Notifications => Err("notifications are updated automatically"),
            Screen::GetStarted => Ok(Operation::AgentStatus),
            Screen::Servers => Ok(Operation::ListProfiles),
            Screen::Settings => Ok(Operation::ListYubiAccounts {
                profile: profile()?,
            }),
            Screen::Parties => Ok(Operation::ListTeams {
                profile: profile()?,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_account_operation(
        &self,
        alias: &str,
        username: &str,
        device_name: &str,
        email: &str,
        invite: SecretString,
        passphrase: Option<SecretString>,
        confirmation: Option<SecretString>,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if alias.trim().is_empty() || username.trim().is_empty() || device_name.trim().is_empty() {
            return Err("alias, username, and device name are required");
        }
        let passphrase = confirmed_optional_passphrase(passphrase, confirmation)?;
        Ok(Operation::CreateAccount {
            profile,
            alias: alias.to_owned(),
            username: username.to_owned(),
            device_name: device_name.to_owned(),
            email: email.to_owned(),
            invite,
            passphrase,
        })
    }

    pub fn passphrase_operation(
        &self,
        action: PassphraseAction,
        passphrase: SecretString,
        confirmation: Option<SecretString>,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        let alias = self
            .selected_account
            .clone()
            .ok_or("select an account first")?;
        if passphrase.expose().is_empty() || passphrase.expose().len() > 1024 {
            return Err("passphrase must contain 1 to 1024 bytes");
        }
        match action {
            PassphraseAction::Set | PassphraseAction::Change => {
                let confirmation = confirmation.ok_or("passphrase confirmation is required")?;
                if passphrase.expose() != confirmation.expose() {
                    return Err("passphrase confirmation does not match");
                }
            }
            PassphraseAction::Verify if confirmation.is_some() => {
                return Err("verification does not take a confirmation");
            }
            PassphraseAction::Verify => {}
        }
        Ok(match action {
            PassphraseAction::Set => Operation::SetPassphrase {
                profile,
                alias,
                passphrase,
            },
            PassphraseAction::Change => Operation::ChangePassphrase {
                profile,
                alias,
                passphrase,
            },
            PassphraseAction::Verify => Operation::VerifyPassphrase {
                profile,
                alias,
                passphrase,
            },
        })
    }

    pub fn start_device_pairing_operation(&self) -> Result<Operation, &'static str> {
        let (profile, account_alias) = self.selected_account_context()?;
        Ok(Operation::StartDevicePairing {
            profile,
            account_alias,
        })
    }

    pub fn republish_device_pairing_operation(&self) -> Result<Operation, &'static str> {
        let (profile, account_alias) = self.selected_account_context()?;
        Ok(Operation::RepublishDevicePairing {
            profile,
            account_alias,
        })
    }

    pub fn finish_device_pairing_operation(&self) -> Result<Operation, &'static str> {
        let (profile, account_alias) = self.selected_account_context()?;
        Ok(Operation::FinishDevicePairing {
            profile,
            account_alias,
        })
    }

    pub fn accept_device_pairing_operation(
        &self,
        target_alias: &str,
        device_name: &str,
        serial: u64,
        phrase: SecretString,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if target_alias.trim().is_empty() || device_name.trim().is_empty() {
            return Err("alias and device name are required");
        }
        if serial == 0 {
            return Err("device serial must be positive");
        }
        if phrase.expose().split_whitespace().count() != 13 {
            return Err("pairing phrase must contain exactly 13 words");
        }
        Ok(Operation::AcceptDevicePairing {
            profile,
            target_alias: target_alias.to_owned(),
            device_name: device_name.to_owned(),
            serial,
            phrase,
        })
    }

    pub fn resume_device_pairing_acceptance_operation(
        &self,
        target_alias: &str,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if target_alias.trim().is_empty() {
            return Err("alias is required");
        }
        Ok(Operation::ResumeDevicePairingAcceptance {
            profile,
            target_alias: target_alias.to_owned(),
        })
    }

    pub fn list_devices_operation(&self) -> Result<Operation, &'static str> {
        let (profile, alias) = self.selected_account_context()?;
        Ok(Operation::ListDevices { profile, alias })
    }

    pub fn provision_owner_device_operation(
        &self,
        target_alias: &str,
        device_name: &str,
        serial: u64,
    ) -> Result<Operation, &'static str> {
        let (profile, source_alias) = self.selected_account_context()?;
        let target_alias = required_text(target_alias, "enter the new local device alias")?;
        let device_name = required_text(device_name, "enter the device name")?;
        if serial == 0 {
            return Err("device serial must be positive");
        }
        Ok(Operation::ProvisionOwnerDevice {
            profile,
            source_alias,
            target_alias,
            device_name,
            serial,
        })
    }

    pub fn resume_owner_device_provision_operation(
        &self,
        target_alias: &str,
    ) -> Result<Operation, &'static str> {
        Ok(Operation::ResumeOwnerDeviceProvision {
            profile: self
                .selected_profile
                .clone()
                .ok_or("select a profile first")?,
            target_alias: required_text(target_alias, "pending device alias is required")?,
        })
    }

    pub fn prepare_owner_backup_operation(
        &self,
        backup_alias: &str,
    ) -> Result<Operation, &'static str> {
        let (profile, account_alias) = self.selected_account_context()?;
        Ok(Operation::PrepareOwnerBackup {
            profile,
            account_alias,
            backup_alias: required_text(backup_alias, "backup alias is required")?,
        })
    }

    pub fn commit_owner_backup_operation(
        &self,
        backup_alias: &str,
        phrase: SecretString,
    ) -> Result<Operation, &'static str> {
        let (profile, account_alias) = self.selected_account_context()?;
        validate_backup_phrase(&phrase)?;
        Ok(Operation::CommitOwnerBackup {
            profile,
            account_alias,
            backup_alias: required_text(backup_alias, "enter a local backup alias")?,
            phrase,
        })
    }

    pub fn recover_owner_account_operation(
        &self,
        target_alias: &str,
        phrase: SecretString,
        device_name: &str,
        serial: u64,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        let target_alias = required_text(target_alias, "enter the recovered account alias")?;
        let device_name = required_text(device_name, "enter the recovery device name")?;
        validate_backup_phrase(&phrase)?;
        if serial == 0 {
            return Err("device serial must be positive");
        }
        Ok(Operation::RecoverOwnerAccount {
            profile,
            target_alias,
            phrase,
            device_name,
            serial,
        })
    }

    pub fn resume_owner_recovery_operation(
        &self,
        target_alias: &str,
        phrase: SecretString,
        device_name: &str,
    ) -> Result<Operation, &'static str> {
        validate_backup_phrase(&phrase)?;
        Ok(Operation::ResumeOwnerRecovery {
            profile: self
                .selected_profile
                .clone()
                .ok_or("select a profile first")?,
            target_alias: required_text(target_alias, "enter the pending recovery alias")?,
            phrase,
            device_name: required_text(device_name, "enter the recovery device name")?,
        })
    }

    fn selected_account_context(&self) -> Result<(String, String), &'static str> {
        Ok((
            self.selected_profile
                .clone()
                .ok_or("select a profile first")?,
            self.selected_account
                .clone()
                .ok_or("select an account first")?,
        ))
    }

    pub fn federation_admission_operation(
        &self,
        local_team_alias: &str,
        remote_profile: &str,
        remote_team_alias: &str,
        role: FederationRole,
        visibility: i16,
    ) -> Result<Operation, &'static str> {
        let local_profile = self
            .selected_profile
            .clone()
            .ok_or("select the local profile first")?;
        if local_team_alias.trim().is_empty()
            || remote_profile.trim().is_empty()
            || remote_team_alias.trim().is_empty()
            || remote_profile == local_profile
        {
            return Err("enter distinct profiles and both team aliases");
        }
        if role != FederationRole::Member {
            return Err("federated teams can only hold member roles");
        }
        Ok(Operation::AdmitFederatedTeam {
            local_profile,
            local_team_alias: local_team_alias.to_owned(),
            remote_profile: remote_profile.to_owned(),
            remote_team_alias: remote_team_alias.to_owned(),
            role,
            visibility,
        })
    }

    pub fn federated_teams_operation(
        &self,
        local_team_alias: &str,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select the local profile first")?;
        if local_team_alias.trim().is_empty() {
            return Err("enter the local team alias");
        }
        Ok(Operation::ListFederatedTeams {
            profile,
            team_alias: local_team_alias.to_owned(),
        })
    }

    pub fn team_members_operation(&self, team_alias: &str) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if team_alias.trim().is_empty() {
            return Err("enter the team alias");
        }
        Ok(Operation::ListTeamMembers {
            profile,
            team_alias: team_alias.to_owned(),
        })
    }

    pub fn create_team_operation(
        &self,
        account_alias: &str,
        team_alias: &str,
        name: &str,
        kind: TeamKind,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        let account_alias = required_text(account_alias, "enter the owner account alias")?;
        let team_alias = required_text(team_alias, "enter a local team alias")?;
        let name = match kind {
            TeamKind::Named => required_text(name, "enter the team name")?,
            TeamKind::AdHoc if name.trim().is_empty() => String::new(),
            TeamKind::AdHoc => return Err("ad-hoc teams do not support team names"),
        };
        Ok(Operation::CreateTeam {
            profile,
            account_alias,
            team_alias,
            name,
            kind,
        })
    }

    pub fn resume_team_creation_operation(
        &self,
        team_alias: &str,
    ) -> Result<Operation, &'static str> {
        Ok(Operation::ResumeTeamCreation {
            profile: self
                .selected_profile
                .clone()
                .ok_or("select a profile first")?,
            team_alias: required_text(team_alias, "pending team alias is required")?,
        })
    }

    pub fn add_team_member_operation(
        &self,
        team_alias: &str,
        username: &str,
        role: TeamRole,
        visibility: i16,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        validate_team_member_input(team_alias, username, role, visibility)?;
        Ok(Operation::AddTeamMember {
            profile,
            team_alias: team_alias.to_owned(),
            username: username.to_owned(),
            role,
            visibility,
        })
    }

    pub fn resume_team_member_addition_operation(
        &self,
        team_alias: &str,
        username: &str,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        validate_team_member_input(team_alias, username, TeamRole::Member, 0)?;
        Ok(Operation::ResumeTeamMemberAddition {
            profile,
            team_alias: team_alias.to_owned(),
            username: username.to_owned(),
        })
    }

    pub fn demote_team_member_operation(
        &self,
        team_alias: &str,
        party_id_hex: &str,
        role: TeamRole,
        visibility: i16,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        validate_team_party_input(team_alias, party_id_hex, role, visibility)?;
        Ok(Operation::DemoteTeamMember {
            profile,
            team_alias: team_alias.to_owned(),
            party_id_hex: party_id_hex.to_owned(),
            role,
            visibility,
        })
    }

    pub fn remove_team_member_operation(
        &self,
        team_alias: &str,
        party_id_hex: &str,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        validate_team_party_input(team_alias, party_id_hex, TeamRole::Member, 0)?;
        Ok(Operation::RemoveTeamMember {
            profile,
            team_alias: team_alias.to_owned(),
            party_id_hex: party_id_hex.to_owned(),
        })
    }

    pub fn resume_team_member_edit_operation(
        &self,
        team_alias: &str,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if team_alias.trim().is_empty() {
            return Err("enter the team alias");
        }
        Ok(Operation::ResumeTeamMemberEdit {
            profile,
            team_alias: team_alias.to_owned(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_yubi_account_operation(
        &self,
        alias: &str,
        username: &str,
        device_name: &str,
        email: &str,
        invite: SecretString,
        passphrase: Option<SecretString>,
        confirmation: Option<SecretString>,
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
        retry_configuration: Option<YubiRetryConfiguration>,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if alias.trim().is_empty() || username.trim().is_empty() || device_name.trim().is_empty() {
            return Err("alias, username, and device name are required");
        }
        validate_yubi_inputs(card_serial, signing_slot, pq_slot, pin.expose())?;
        validate_retry_configuration(retry_configuration.as_ref())?;
        Ok(Operation::CreateYubiAccount {
            profile,
            alias: alias.to_owned(),
            username: username.to_owned(),
            device_name: device_name.to_owned(),
            email: email.to_owned(),
            invite,
            passphrase: confirmed_optional_passphrase(passphrase, confirmation)?,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration,
        })
    }

    pub fn yubi_action_operation(
        &self,
        action: YubiAction,
        alias: &str,
        pin: Option<SecretString>,
        software_alias: Option<&str>,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        match action {
            YubiAction::ListCards => Ok(Operation::ListYubiCards { profile }),
            _ if alias.trim().is_empty() => Err("YubiKey account alias is required"),
            YubiAction::ResumeAccount => Ok(Operation::ResumeYubiAccount {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
            }),
            YubiAction::PinStatus => Ok(Operation::YubiPinStatus {
                profile,
                alias: alias.to_owned(),
            }),
            YubiAction::Sync => Ok(Operation::SyncYubiAccount {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
                with_federation: false,
            }),
            YubiAction::SyncWithFederation => Ok(Operation::SyncYubiAccount {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
                with_federation: true,
            }),
            YubiAction::RotateManagementKey => Ok(Operation::RotateYubiManagementKey {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
            }),
            YubiAction::ResumeManagementKey => Ok(Operation::ResumeYubiManagementKey {
                profile,
                alias: alias.to_owned(),
                pin: Some(required_pin(pin)?),
            }),
            YubiAction::RecoverManagementKey => Ok(Operation::RecoverYubiManagementKey {
                profile,
                yubi_alias: alias.to_owned(),
                software_alias: software_alias
                    .filter(|alias| !alias.trim().is_empty())
                    .ok_or("software account alias is required")?
                    .to_owned(),
            }),
            YubiAction::RecoverSubkey => Ok(Operation::RecoverYubiSubkey {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
            }),
            YubiAction::Revoke => Ok(Operation::RevokeYubiDevice {
                profile,
                yubi_alias: alias.to_owned(),
                software_alias: software_alias
                    .filter(|alias| !alias.trim().is_empty())
                    .ok_or("enter a software account alias")?
                    .to_owned(),
            }),
        }
    }

    pub fn yubi_passphrase_operation(
        &self,
        action: PassphraseAction,
        alias: &str,
        pin: SecretString,
        passphrase: SecretString,
        confirmation: Option<SecretString>,
    ) -> Result<Operation, &'static str> {
        let profile = self.selected_yubi_profile()?;
        let alias = required_alias(alias)?;
        validate_pin(pin.expose())?;
        match action {
            PassphraseAction::Set | PassphraseAction::Change => {
                let confirmation = confirmation.ok_or("confirm the passphrase")?;
                if passphrase.expose() != confirmation.expose() {
                    return Err("passphrase confirmation does not match");
                }
            }
            PassphraseAction::Verify if confirmation.is_some() => {
                return Err("verification does not take a confirmation");
            }
            PassphraseAction::Verify => {}
        }
        Ok(match action {
            PassphraseAction::Set => Operation::SetYubiPassphrase {
                profile,
                alias,
                pin,
                passphrase,
            },
            PassphraseAction::Change => Operation::ChangeYubiPassphrase {
                profile,
                alias,
                pin,
                passphrase,
            },
            PassphraseAction::Verify => Operation::VerifyYubiPassphrase {
                profile,
                alias,
                pin,
                passphrase,
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn provision_yubi_operation(
        &self,
        source_alias: &str,
        target_alias: &str,
        device_name: &str,
        serial: u64,
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
        retry_configuration: Option<YubiRetryConfiguration>,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if source_alias.trim().is_empty()
            || target_alias.trim().is_empty()
            || device_name.trim().is_empty()
            || serial == 0
        {
            return Err("source alias, target alias, device name, and serial are required");
        }
        validate_yubi_inputs(card_serial, signing_slot, pq_slot, pin.expose())?;
        validate_retry_configuration(retry_configuration.as_ref())?;
        Ok(Operation::ProvisionYubiDevice {
            profile,
            source_alias: source_alias.to_owned(),
            target_alias: target_alias.to_owned(),
            device_name: device_name.to_owned(),
            serial,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration,
        })
    }

    pub fn change_yubi_pin_operation(
        &self,
        alias: &str,
        old_pin: SecretString,
        new_pin: SecretString,
    ) -> Result<Operation, &'static str> {
        validate_pin(old_pin.expose())?;
        validate_pin(new_pin.expose())?;
        Ok(Operation::ChangeYubiPin {
            profile: self.selected_yubi_profile()?,
            alias: required_alias(alias)?,
            old_pin,
            new_pin,
        })
    }

    pub fn change_yubi_puk_operation(
        &self,
        alias: &str,
        old_puk: SecretString,
        new_puk: SecretString,
    ) -> Result<Operation, &'static str> {
        validate_pin(old_puk.expose())?;
        validate_pin(new_puk.expose())?;
        Ok(Operation::ChangeYubiPuk {
            profile: self.selected_yubi_profile()?,
            alias: required_alias(alias)?,
            old_puk,
            new_puk,
        })
    }

    pub fn unblock_yubi_pin_operation(
        &self,
        alias: &str,
        puk: SecretString,
        new_pin: SecretString,
    ) -> Result<Operation, &'static str> {
        validate_pin(puk.expose())?;
        validate_pin(new_pin.expose())?;
        Ok(Operation::UnblockYubiPin {
            profile: self.selected_yubi_profile()?,
            alias: required_alias(alias)?,
            puk,
            new_pin,
        })
    }

    fn selected_yubi_profile(&self) -> Result<String, &'static str> {
        self.selected_profile
            .clone()
            .ok_or("select a profile first")
    }

    pub fn accept(&mut self, result: Result<Value, String>) {
        match result {
            Ok(value) => {
                match self.screen {
                    Screen::Servers => {
                        if let Ok(parsed) =
                            serde_json::from_value::<Vec<ProfileSummary>>(value.clone())
                        {
                            self.profiles =
                                parsed.into_iter().map(|profile| profile.name).collect();
                            if self.selected_profile.is_none() {
                                self.selected_profile = self.profiles.first().cloned();
                            }
                        }
                    }
                    Screen::Stores => {
                        if let Ok(parsed) =
                            serde_json::from_value::<Vec<AccountSummary>>(value.clone())
                        {
                            self.accounts =
                                parsed.into_iter().map(|account| account.alias).collect();
                            if self.selected_account.is_none() {
                                self.selected_account = self.accounts.first().cloned();
                            }
                        }
                    }
                    _ => {}
                }
                self.value = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.value = None;
                self.error = Some(error);
            }
        }
    }
}

fn validate_team_member_input(
    team_alias: &str,
    username: &str,
    role: TeamRole,
    visibility: i16,
) -> Result<(), &'static str> {
    if team_alias.trim().is_empty() || username.trim().is_empty() {
        return Err("enter the team alias and username");
    }
    if role != TeamRole::Member && visibility != 0 {
        return Err("visibility applies only to member roles");
    }
    Ok(())
}

fn validate_team_party_input(
    team_alias: &str,
    party_id_hex: &str,
    role: TeamRole,
    visibility: i16,
) -> Result<(), &'static str> {
    if team_alias.trim().is_empty()
        || party_id_hex.len() != 66
        || !party_id_hex.starts_with("01")
        || !party_id_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("enter the team alias and authenticated user party id");
    }
    if role != TeamRole::Member && visibility != 0 {
        return Err("visibility applies only to member roles");
    }
    Ok(())
}

fn validate_backup_phrase(phrase: &SecretString) -> Result<(), &'static str> {
    if phrase.expose().split_whitespace().count() != 17 {
        return Err("backup phrase must contain exactly 17 words");
    }
    Ok(())
}

fn validate_yubi_inputs(
    card_serial: u32,
    signing_slot: u8,
    pq_slot: u8,
    pin: &str,
) -> Result<(), &'static str> {
    if card_serial == 0 {
        return Err("select a nonzero YubiKey serial");
    }
    if !(0x82..=0x95).contains(&signing_slot)
        || !(0x82..=0x95).contains(&pq_slot)
        || signing_slot == pq_slot
    {
        return Err("signing and post-quantum keys must use distinct PIV slots (0x82-0x95)");
    }
    if !(6..=8).contains(&pin.len()) || !pin.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("PIN must contain six to eight printable ASCII characters");
    }
    Ok(())
}

fn required_pin(pin: Option<SecretString>) -> Result<SecretString, &'static str> {
    let pin = pin.ok_or("enter the YubiKey PIN")?;
    validate_pin(pin.expose())?;
    Ok(pin)
}

fn validate_pin(pin: &str) -> Result<(), &'static str> {
    if !(6..=8).contains(&pin.len()) || !pin.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("PIN or PUK must contain six to eight printable ASCII characters");
    }
    Ok(())
}

fn validate_retry_configuration(
    retry: Option<&YubiRetryConfiguration>,
) -> Result<(), &'static str> {
    let Some(retry) = retry else {
        return Ok(());
    };
    validate_pin(retry.puk.expose())?;
    if !(1..=15).contains(&retry.pin_attempts) || !(1..=15).contains(&retry.puk_attempts) {
        return Err("PIN and PUK retries must each be from 1 through 15");
    }
    Ok(())
}

fn required_alias(alias: &str) -> Result<String, &'static str> {
    if alias.trim().is_empty() {
        Err("enter a YubiKey account alias")
    } else {
        Ok(alias.to_owned())
    }
}

fn required_text(value: &str, error: &'static str) -> Result<String, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        Err(error)
    } else {
        Ok(value.to_owned())
    }
}

fn required_verbatim_text(value: &str, error: &'static str) -> Result<String, &'static str> {
    if value.trim().is_empty() {
        Err(error)
    } else {
        Ok(value.to_owned())
    }
}

fn confirmed_optional_passphrase(
    passphrase: Option<SecretString>,
    confirmation: Option<SecretString>,
) -> Result<Option<SecretString>, &'static str> {
    match (passphrase, confirmation) {
        (None, None) => Ok(None),
        (Some(passphrase), Some(confirmation))
            if passphrase.expose().is_empty() && confirmation.expose().is_empty() =>
        {
            Ok(None)
        }
        (Some(passphrase), Some(confirmation)) => {
            if passphrase.expose().len() > 1024 {
                return Err("passphrase exceeds 1024 bytes");
            }
            if passphrase.expose() != confirmation.expose() {
                return Err("passphrase confirmation does not match");
            }
            Ok(Some(passphrase))
        }
        _ => Err("signup passphrase and confirmation are required"),
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

    struct MockTransport {
        operations: Mutex<Vec<Operation>>,
    }

    impl AgentTransport for MockTransport {
        fn call(&self, operation: Operation) -> Result<Value, AgentError> {
            self.operations.lock().unwrap().push(operation);
            Ok(Value::Null)
        }
    }

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
                            path: path.to_owned(),
                            node_type: "small-file".to_owned(),
                            version: 1,
                            size: None,
                            read_role: foks_agent_proto::KvRole::Owner,
                            write_role: foks_agent_proto::KvRole::Owner,
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
                    },
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
                if snapshot.inventory.is_empty() && !snapshot.known_stores.is_empty() {
                    local = true;
                    assert_eq!(snapshot.full_item_reads, None);
                    assert!(snapshot.stores.is_empty());
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
                assert!(
                    matches!(&result.failures[0].scope, CatalogFailureScope::Store(store) if store.profile() == "slow")
                );
            }
            assert!(local);
            let healthy = healthy.expect("healthy profile must publish while slow KV is blocked");
            assert_eq!(healthy.profiles, ["slow", "healthy"]);
            assert!(!healthy
                .inventory
                .iter()
                .any(|entry| entry.profile == "slow"));
            if cancel {
                assert!(progress.try_iter().all(|snapshot| !snapshot
                    .inventory
                    .iter()
                    .any(|entry| entry.profile == "slow")));
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
        )
        .unwrap();
        assert_eq!(catalog.profiles, ["local"]);
        assert_eq!(catalog.stores.len(), 2);
        assert!(catalog.blocked_profiles.is_empty());
        transport.0.lock().unwrap().clear();
        assert!(load_profile_catalog_cancellable(
            transport.clone(),
            "missing".into(),
            CatalogLoadToken::default()
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
            load_profile_catalog_cancellable(transport.clone(), "local".into(), token),
            Err(AgentError::Cancelled)
        ));
        assert!(transport.0.lock().unwrap().is_empty());
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
                fields: ErrorFields::default()
            }));
        }
        for capability in ["kv", "chat", "teams"] {
            assert!(!error_blocks_profile_catalog(&AgentError::Protocol {
                code: ErrorCode::CapabilityDenied,
                message: "changed wording".into(),
                fields: ErrorFields {
                    capability: Some(capability.into()),
                    ..ErrorFields::default()
                },
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
                            path: path.to_owned(),
                            node_type: "small-file".to_owned(),
                            version: 1,
                            size: None,
                            read_role: KvRole::Owner,
                            write_role: KvRole::Owner,
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
                        path: "/stale".to_owned(),
                        node_type: "small-file".to_owned(),
                        version: 1,
                        size: None,
                        read_role: KvRole::Owner,
                        write_role: KvRole::Owner,
                    }],
                    next_cursor: Some("stale-cursor".to_owned()),
                })
                .unwrap()),
                2 => Err(AgentError::Protocol {
                    code: ErrorCode::CatalogSnapshotChanged,
                    message: "wording is deliberately irrelevant".to_owned(),
                    fields: ErrorFields::default(),
                }),
                3 => Ok(serde_json::to_value(KvPage {
                    snapshot_version: 8,
                    entries: vec![KvEntryMetadata {
                        path: "/fresh".to_owned(),
                        node_type: "small-file".to_owned(),
                        version: 1,
                        size: None,
                        read_role: KvRole::Owner,
                        write_role: KvRole::Owner,
                    }],
                    next_cursor: Some("fresh-cursor".to_owned()),
                })
                .unwrap()),
                4 => Ok(serde_json::to_value(KvPage {
                    snapshot_version: 8,
                    entries: vec![KvEntryMetadata {
                        path: "/second".to_owned(),
                        node_type: "small-file".to_owned(),
                        version: 1,
                        size: None,
                        read_role: KvRole::Owner,
                        write_role: KvRole::Owner,
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
                        fields: ErrorFields::default(),
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
        let result =
            load_catalog_cancellable(Arc::new(CancellingCatalogTransport(token.clone())), token);
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
                    version,
                } => {
                    assert_eq!(store, self.store);
                    assert_eq!(path, "/large");
                    assert_eq!(version, 9);
                    Ok(serde_json::to_value(KvReadResult {
                        store,
                        path,
                        version,
                        node_type: "file".to_owned(),
                        size: Some(self.total),
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
                path: "/large".to_owned(),
                node_type: "file".to_owned(),
                version: 9,
                size: None,
                read_role: KvRole::Member { visibility: 2 },
                write_role: KvRole::Admin,
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

    #[test]
    fn account_and_team_item_mutations_are_cas_bound() {
        let account = CatalogStoreRef::Account(AccountStoreRef {
            profile: "local".to_owned(),
            account_alias: "personal".to_owned(),
        });
        let item = CatalogItem {
            store: account.clone(),
            metadata: KvEntryMetadata {
                path: "/password".to_owned(),
                node_type: "small-file".to_owned(),
                version: 7,
                size: None,
                read_role: KvRole::Member { visibility: -1 },
                write_role: KvRole::Admin,
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
                precondition: KvPrecondition::ExactVersion { version: 7 },
                mkdir_p: false,
                ..
            })
        ));
        assert!(matches!(
            edit_kv_file_mutation(&item, vec![0; MAXIMUM_INLINE_KV_BYTES + 1]).unwrap(),
            KvAccountMutation::Stream {
                header: KvUploadHeader {
                    adapter: None,
                    precondition: KvPrecondition::ExactVersion { version: 7 },
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
                adapter: None,
                total_length: 64,
                read_role: KvRole::Member { visibility: -1 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::ExactVersion { version: 7 },
                mkdir_p: false,
                ..
            }
        ));
        assert!(matches!(
            edit_kv_file_upload(&item, 64).unwrap(),
            KvUploadHeader {
                adapter: None,
                total_length: 64,
                read_role: KvRole::Member { visibility: -1 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::ExactVersion { version: 7 },
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
            fields: ErrorFields::default(),
        };
        assert!(deadline.transient());
        assert!(deadline.ambiguous());
        assert!(!deadline.fatal());
        assert!(AgentError::Protocol {
            code: ErrorCode::RateLimited,
            message: "wait".to_owned(),
            fields: ErrorFields::default(),
        }
        .transient());
        assert!(!AgentError::Protocol {
            code: ErrorCode::QuotaExceeded,
            message: "full".to_owned(),
            fields: ErrorFields::default(),
        }
        .transient());

        let mismatch = AgentError::Protocol {
            code: ErrorCode::VersionMismatch,
            message: "upgrade".to_owned(),
            fields: ErrorFields::default(),
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
    }

    #[test]
    fn screens_keep_catalog_and_profile_scopes_separate() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.navigate(Screen::Items);
        assert_eq!(
            model.operation(),
            Err("catalog data must be refreshed directly")
        );
        model.navigate(Screen::Settings);
        assert_eq!(model.operation(), Err("select a profile first"));
        model.select_profile("hosted");
        assert_eq!(
            model.probe_operation().unwrap(),
            Operation::Probe {
                profile: "hosted".to_owned(),
            }
        );
        assert_eq!(
            model.operation().unwrap(),
            Operation::ListYubiAccounts {
                profile: "hosted".to_owned(),
            }
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
    fn first_run_operations_are_explicitly_bound_to_native_foks_identity() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        assert_eq!(
            model.add_profile_operation("", "foks.example:443"),
            Err("enter a profile name")
        );
        assert_eq!(
            model
                .add_profile_operation("hosted", "foks.example:443")
                .unwrap(),
            Operation::AddProfile {
                name: "hosted".to_owned(),
                probe: "foks.example:443".to_owned(),
                protocol: ProfileProtocol::V019,
                trust: ProfileTrust::WebPki,
            }
        );
        model.record_profile("hosted");
        assert_eq!(
            model.resume_account_operation("personal").unwrap(),
            Operation::ResumeAccount {
                profile: "hosted".to_owned(),
                alias: "personal".to_owned(),
            }
        );
        assert_eq!(
            model.reset_hard_state_operation().unwrap_err(),
            "hard reset is not supported from this screen"
        );
        model.forget_profile("hosted");
        assert!(model.profiles().is_empty());
    }

    #[test]
    fn profile_and_account_lists_choose_only_explicit_response_values() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.navigate(Screen::Servers);
        model.accept(Ok(serde_json::json!([{"name": "local"}])));
        assert_eq!(model.selected_profile(), Some("local"));
        assert_eq!(model.profiles(), ["local".to_owned()]);
        model.navigate(Screen::Stores);
        model.accept(Ok(serde_json::json!([{
            "profile": "local",
            "alias": "personal",
            "username": "alice"
        }])));
        assert_eq!(model.selected_account(), Some("personal"));
        assert_eq!(model.accounts(), ["personal".to_owned()]);
    }

    #[test]
    fn selection_and_cached_lists_survive_selection_and_form_responses() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.navigate(Screen::Servers);
        model.accept(Ok(
            serde_json::json!([{"name": "local"}, {"name": "partner"}]),
        ));
        // Selecting a profile must not clear the candidate list.
        model.select_profile("partner");
        assert_eq!(model.profiles(), ["local".to_owned(), "partner".to_owned()]);
        assert!(model.value().is_some());

        model.navigate(Screen::Stores);
        model.accept(Ok(serde_json::json!([
            {"profile": "partner", "alias": "personal", "username": "alice"},
            {"profile": "partner", "alias": "work", "username": "alice"}
        ])));
        model.select_account("work");
        assert_eq!(model.accounts(), ["personal".to_owned(), "work".to_owned()]);

        // A non-list response (a form submission report) must not clobber
        // the cached account list or the selection.
        model.accept(Ok(serde_json::json!({"username": "satoshi", "entries": 3})));
        assert_eq!(model.accounts(), ["personal".to_owned(), "work".to_owned()]);
        assert_eq!(model.selected_account(), Some("work"));

        // Switching profile invalidates the cached accounts of the old one.
        model.select_profile("local");
        assert!(model.accounts().is_empty());
        assert_eq!(model.selected_account(), None);

        model.record_account("fresh");
        assert_eq!(model.accounts(), ["fresh".to_owned()]);
        assert_eq!(model.selected_account(), Some("fresh"));
    }

    #[test]
    fn account_creation_is_bound_to_the_selected_profile_and_invite() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        assert_eq!(
            model.create_account_operation(
                "personal",
                "satoshi",
                "laptop",
                "",
                SecretString::new("invite"),
                None,
                None,
            ),
            Err("select a profile first")
        );
        model.select_profile("local");
        assert_eq!(
            model
                .create_account_operation(
                    "personal",
                    "satoshi",
                    "laptop",
                    "satoshi@example.test",
                    SecretString::new("small-team+launch"),
                    None,
                    None,
                )
                .unwrap(),
            Operation::CreateAccount {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                username: "satoshi".to_owned(),
                device_name: "laptop".to_owned(),
                email: "satoshi@example.test".to_owned(),
                invite: SecretString::new("small-team+launch"),
                passphrase: None,
            }
        );
    }

    #[test]
    fn account_and_security_forms_bind_passphrase_operations_to_the_selection() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.select_profile("local");

        assert_eq!(
            model
                .create_account_operation(
                    "personal",
                    "satoshi",
                    "laptop",
                    "",
                    SecretString::new("s.invite"),
                    Some(SecretString::new("signup passphrase")),
                    Some(SecretString::new("signup passphrase")),
                )
                .unwrap(),
            Operation::CreateAccount {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                username: "satoshi".to_owned(),
                device_name: "laptop".to_owned(),
                email: String::new(),
                invite: SecretString::new("s.invite"),
                passphrase: Some(SecretString::new("signup passphrase")),
            }
        );
        assert_eq!(
            model.create_account_operation(
                "personal",
                "satoshi",
                "laptop",
                "",
                SecretString::new(""),
                Some(SecretString::new("one")),
                Some(SecretString::new("two")),
            ),
            Err("passphrase confirmation does not match")
        );

        model.select_account("personal");
        assert_eq!(
            model
                .passphrase_operation(
                    PassphraseAction::Set,
                    SecretString::new("first passphrase"),
                    Some(SecretString::new("first passphrase")),
                )
                .unwrap(),
            Operation::SetPassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("first passphrase"),
            }
        );
        assert_eq!(
            model
                .passphrase_operation(
                    PassphraseAction::Change,
                    SecretString::new("second passphrase"),
                    Some(SecretString::new("second passphrase")),
                )
                .unwrap(),
            Operation::ChangePassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("second passphrase"),
            }
        );
        assert_eq!(
            model
                .passphrase_operation(
                    PassphraseAction::Verify,
                    SecretString::new("second passphrase"),
                    None,
                )
                .unwrap(),
            Operation::VerifyPassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("second passphrase"),
            }
        );
        assert_eq!(
            model.passphrase_operation(
                PassphraseAction::Verify,
                SecretString::new("second passphrase"),
                Some(SecretString::new("second passphrase")),
            ),
            Err("verification does not take a confirmation")
        );
    }

    #[test]
    fn device_pairing_operations_are_bound_and_phrase_checked() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        assert_eq!(
            model.start_device_pairing_operation(),
            Err("select a profile first")
        );
        model.select_profile("local");
        assert_eq!(
            model.start_device_pairing_operation(),
            Err("select an account first")
        );
        model.select_account("personal");
        assert_eq!(
            model.start_device_pairing_operation().unwrap(),
            Operation::StartDevicePairing {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }
        );
        assert_eq!(
            model.finish_device_pairing_operation().unwrap(),
            Operation::FinishDevicePairing {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }
        );
        assert_eq!(
            model.accept_device_pairing_operation(
                "laptop",
                "new device",
                1,
                SecretString::new("too short"),
            ),
            Err("pairing phrase must contain exactly 13 words")
        );
        let phrase = SecretString::new("one 1 two 2 three 3 four 4 five 5 six 6 seven");
        assert_eq!(
            model
                .accept_device_pairing_operation("laptop", "new device", 2, phrase.clone())
                .unwrap(),
            Operation::AcceptDevicePairing {
                profile: "local".to_owned(),
                target_alias: "laptop".to_owned(),
                device_name: "new device".to_owned(),
                serial: 2,
                phrase,
            }
        );
        assert_eq!(
            model
                .resume_device_pairing_acceptance_operation("laptop")
                .unwrap(),
            Operation::ResumeDevicePairingAcceptance {
                profile: "local".to_owned(),
                target_alias: "laptop".to_owned(),
            }
        );
    }

    #[test]
    fn federation_form_binds_both_profiles_and_validates_role_visibility() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        assert_eq!(
            model.federation_admission_operation(
                "engineering",
                "partner",
                "security",
                FederationRole::Member,
                0,
            ),
            Err("select the local profile first")
        );
        model.select_profile("local");
        assert_eq!(
            model
                .federation_admission_operation(
                    "engineering",
                    "partner",
                    "security",
                    FederationRole::Member,
                    4,
                )
                .unwrap(),
            Operation::AdmitFederatedTeam {
                local_profile: "local".to_owned(),
                local_team_alias: "engineering".to_owned(),
                remote_profile: "partner".to_owned(),
                remote_team_alias: "security".to_owned(),
                role: FederationRole::Member,
                visibility: 4,
            }
        );
        assert_eq!(
            model.federation_admission_operation(
                "engineering",
                "local",
                "security",
                FederationRole::Member,
                0,
            ),
            Err("enter distinct profiles and both team aliases")
        );
        assert_eq!(
            model.federation_admission_operation(
                "engineering",
                "partner",
                "security",
                FederationRole::Admin,
                1,
            ),
            Err("federated teams can only hold member roles")
        );
        assert_eq!(
            model.federated_teams_operation("engineering").unwrap(),
            Operation::ListFederatedTeams {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            }
        );
    }

    #[test]
    fn team_member_form_builds_every_agent_operation_and_rejects_bad_visibility() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.select_profile("local");
        assert_eq!(
            model.team_members_operation("engineering").unwrap(),
            Operation::ListTeamMembers {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            }
        );
        assert_eq!(
            model
                .add_team_member_operation("engineering", "alice", TeamRole::Admin, 0)
                .unwrap(),
            Operation::AddTeamMember {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                username: "alice".to_owned(),
                role: TeamRole::Admin,
                visibility: 0,
            }
        );
        assert_eq!(
            model.add_team_member_operation("engineering", "alice", TeamRole::Owner, 1),
            Err("visibility applies only to member roles")
        );
        assert_eq!(
            model
                .resume_team_member_addition_operation("engineering", "alice")
                .unwrap(),
            Operation::ResumeTeamMemberAddition {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                username: "alice".to_owned(),
            }
        );
        assert_eq!(
            model
                .demote_team_member_operation(
                    "engineering",
                    "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    TeamRole::Member,
                    -2,
                )
                .unwrap(),
            Operation::DemoteTeamMember {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                party_id_hex: "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
                role: TeamRole::Member,
                visibility: -2,
            }
        );
        assert_eq!(
            model.demote_team_member_operation("engineering", "alice", TeamRole::Member, 0,),
            Err("enter the team alias and authenticated user party id")
        );
        assert_eq!(
            model
                .remove_team_member_operation(
                    "engineering",
                    "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .unwrap(),
            Operation::RemoveTeamMember {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                party_id_hex: "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
            }
        );
        assert_eq!(
            model
                .resume_team_member_edit_operation("engineering")
                .unwrap(),
            Operation::ResumeTeamMemberEdit {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            }
        );
    }

    #[test]
    fn team_creation_keeps_native_kind_and_has_an_explicit_resume() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.select_profile("local");
        assert_eq!(
            model
                .create_team_operation(
                    "personal",
                    "engineering",
                    "engineeringteam",
                    TeamKind::Named,
                )
                .unwrap(),
            Operation::CreateTeam {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "engineering".to_owned(),
                name: "engineeringteam".to_owned(),
                kind: TeamKind::Named,
            }
        );
        assert_eq!(
            model
                .create_team_operation("personal", "project", "", TeamKind::AdHoc)
                .unwrap(),
            Operation::CreateTeam {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "project".to_owned(),
                name: String::new(),
                kind: TeamKind::AdHoc,
            }
        );
        assert_eq!(
            model.create_team_operation("personal", "project", "invented", TeamKind::AdHoc),
            Err("ad-hoc teams do not support team names")
        );
        assert_eq!(
            model.resume_team_creation_operation("engineering").unwrap(),
            Operation::ResumeTeamCreation {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            }
        );
    }

    #[test]
    fn owner_device_and_recovery_operations_stay_profile_and_account_bound() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.select_profile("local");
        model.record_account("personal".to_owned());
        model.select_account("personal");

        assert_eq!(
            model.list_devices_operation().unwrap(),
            Operation::ListDevices {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
            }
        );
        assert_eq!(
            model
                .provision_owner_device_operation("laptop", "Laptop", 2)
                .unwrap(),
            Operation::ProvisionOwnerDevice {
                profile: "local".to_owned(),
                source_alias: "personal".to_owned(),
                target_alias: "laptop".to_owned(),
                device_name: "Laptop".to_owned(),
                serial: 2,
            }
        );
        assert_eq!(
            model
                .resume_owner_device_provision_operation("laptop")
                .unwrap(),
            Operation::ResumeOwnerDeviceProvision {
                profile: "local".to_owned(),
                target_alias: "laptop".to_owned(),
            }
        );
        assert_eq!(
            model.prepare_owner_backup_operation("offline").unwrap(),
            Operation::PrepareOwnerBackup {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
                backup_alias: "offline".to_owned(),
            }
        );

        let phrase = (1..=17)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            model
                .commit_owner_backup_operation("offline", SecretString::new(&phrase))
                .unwrap(),
            Operation::CommitOwnerBackup {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
                backup_alias: "offline".to_owned(),
                phrase: SecretString::new(&phrase),
            }
        );
        assert_eq!(
            model
                .recover_owner_account_operation(
                    "recovered",
                    SecretString::new(&phrase),
                    "Recovery laptop",
                    3,
                )
                .unwrap(),
            Operation::RecoverOwnerAccount {
                profile: "local".to_owned(),
                target_alias: "recovered".to_owned(),
                phrase: SecretString::new(&phrase),
                device_name: "Recovery laptop".to_owned(),
                serial: 3,
            }
        );
        assert_eq!(
            model.recover_owner_account_operation(
                "recovered",
                SecretString::new("too short"),
                "Recovery laptop",
                3,
            ),
            Err("backup phrase must contain exactly 17 words")
        );
    }

    #[test]
    fn yubikey_screen_binds_hardware_and_recovery_operations_to_the_profile() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.select_profile("local");
        model.navigate(Screen::Settings);
        assert_eq!(
            model.operation().unwrap(),
            Operation::ListYubiAccounts {
                profile: "local".to_owned(),
            }
        );
        assert_eq!(
            model.yubi_action_operation(YubiAction::ListCards, "", None, None),
            Ok(Operation::ListYubiCards {
                profile: "local".to_owned(),
            })
        );
        assert_eq!(
            model
                .create_yubi_account_operation(
                    "hardware",
                    "satoshi",
                    "primary key",
                    "",
                    SecretString::new("s.invite"),
                    None,
                    None,
                    42,
                    0x82,
                    0x83,
                    SecretString::new("123456"),
                    Some(YubiRetryConfiguration {
                        puk: SecretString::new("12345678"),
                        pin_attempts: 5,
                        puk_attempts: 4,
                    }),
                )
                .unwrap(),
            Operation::CreateYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                username: "satoshi".to_owned(),
                device_name: "primary key".to_owned(),
                email: String::new(),
                invite: SecretString::new("s.invite"),
                passphrase: None,
                card_serial: 42,
                signing_slot: 0x82,
                pq_slot: 0x83,
                pin: SecretString::new("123456"),
                retry_configuration: Some(YubiRetryConfiguration {
                    puk: SecretString::new("12345678"),
                    pin_attempts: 5,
                    puk_attempts: 4,
                }),
            }
        );
        assert_eq!(
            model.yubi_action_operation(
                YubiAction::RecoverManagementKey,
                "hardware",
                None,
                Some("personal"),
            ),
            Ok(Operation::RecoverYubiManagementKey {
                profile: "local".to_owned(),
                yubi_alias: "hardware".to_owned(),
                software_alias: "personal".to_owned(),
            })
        );
        assert_eq!(
            model.yubi_action_operation(YubiAction::Sync, "hardware", None, None),
            Err("enter the YubiKey PIN")
        );
        assert_eq!(
            model.yubi_action_operation(
                YubiAction::ResumeAccount,
                "hardware",
                Some(SecretString::new("123456")),
                None,
            ),
            Ok(Operation::ResumeYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("123456"),
            })
        );
        assert_eq!(
            model.yubi_action_operation(
                YubiAction::ResumeManagementKey,
                "hardware",
                Some(SecretString::new("123456")),
                None,
            ),
            Ok(Operation::ResumeYubiManagementKey {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: Some(SecretString::new("123456")),
            })
        );
        assert_eq!(
            model.create_yubi_account_operation(
                "hardware",
                "satoshi",
                "primary key",
                "",
                SecretString::new(""),
                None,
                None,
                42,
                0x82,
                0x82,
                SecretString::new("123456"),
                None,
            ),
            Err("signing and post-quantum keys must use distinct PIV slots (0x82-0x95)")
        );
        assert_eq!(
            model
                .yubi_passphrase_operation(
                    PassphraseAction::Change,
                    "hardware",
                    SecretString::new("123456"),
                    SecretString::new("new passphrase"),
                    Some(SecretString::new("new passphrase")),
                )
                .unwrap(),
            Operation::ChangeYubiPassphrase {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("123456"),
                passphrase: SecretString::new("new passphrase"),
            }
        );
    }

    #[test]
    fn failed_catalog_refresh_keeps_the_last_authenticated_snapshot() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.accept_catalog(Ok(CatalogSnapshot {
            profiles: vec!["local".to_owned()],
            ..CatalogSnapshot::default()
        }));
        model.accept_catalog(Err(AgentError::Transport("agent unavailable".to_owned())));
        assert_eq!(model.catalog().unwrap().profiles, ["local"]);
        assert_eq!(model.error(), Some("agent unavailable"));
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
