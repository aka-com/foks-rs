//! Blocking local-agent transport, managed launch, and connection observation.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use foks_agent_client::AgentClient;
use foks_agent_proto::{ErrorCode, ErrorFields, Operation, Response, ResponseResult};
use foks_desktop::{AgentError as DesktopAgentError, AgentTransport, IpcErrorCode};

use crate::diagnostics::{agent_operation, outcome_of, Label, TimingLog};
use fs2::FileExt as _;
use serde::Serialize;
use serde_json::Value;

pub const SOCKET_ENV: &str = "FOKS_AGENT_SOCKET";
pub const SOCKET_ARG: &str = "--agent-socket";
const AGENT_BINARY_ENV: &str = "FOKS_AGENT_BINARY";
const DEFAULT_SOCKET_NAME: &str = "foks-rs.sock";

/// Identifies the socket endpoint and process approved for replacement.
/// Approval is invalid if either changes before termination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTakeover {
    pub socket: PathBuf,
    pub pid: u32,
    pub executable: PathBuf,
    device: u64,
    inode: u64,
}

const MAX_STARTUP_TAKEOVERS: usize = 4;

/// The socket selected for this process and any state that this desktop owns.
///
/// An explicit argument or environment socket is an external trust boundary:
/// managed crash directories are not created for sockets owned by an external launcher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentEndpoint {
    pub socket: PathBuf,
    pub managed_crash_directory: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub ambiguous: bool,
    pub fatal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Box<AgentErrorDetails>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentErrorDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found_schema: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_schema: Option<u32>,
}

impl AgentError {
    pub fn new(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            retryable,
            ambiguous: false,
            fatal: false,
            details: None,
        }
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new("unknown", message, false)
    }

    pub fn from_client(error: &foks_agent_client::Error) -> Self {
        Self::from_desktop(foks_desktop::agent_client_error(error))
    }

    fn from_ipc(
        code: IpcErrorCode,
        message: String,
        ambiguous: bool,
        connection_lost: bool,
    ) -> Self {
        let ambiguous = ambiguous || code == IpcErrorCode::Ambiguous;
        let security = code.security_failure();
        let slug = if connection_lost && !security {
            "agent-lost"
        } else if ambiguous
            && matches!(
                code,
                IpcErrorCode::Cancelled | IpcErrorCode::DeadlineExceeded
            )
        {
            "ambiguous"
        } else {
            code.as_str()
        };
        let mut mapped = Self::new(
            slug,
            message,
            !ambiguous && !security && (code.retryable() || connection_lost),
        );
        mapped.ambiguous = ambiguous;
        mapped.fatal = security || connection_lost;
        mapped
    }

    pub fn from_agent(code: ErrorCode, message: String) -> Self {
        let (slug, retryable) = match code {
            ErrorCode::CredentialsRequired => ("credentials-required", false),
            ErrorCode::SavedTrustMissing => ("saved-trust-missing", false),
            ErrorCode::ServerUnavailable => ("server-unavailable", true),
            ErrorCode::ServerIdentityRejected => ("server-identity-rejected", false),
            ErrorCode::CompatibilityRejected => ("compatibility-rejected", false),
            ErrorCode::ProfileConfigurationChanged => ("profile-configuration-changed", false),
            ErrorCode::RetentionFull => ("retention-full", false),
            ErrorCode::ClockUntrusted => ("clock-untrusted", false),
            ErrorCode::SubmissionActiveFull => ("submission-active-full", false),
            ErrorCode::SubmissionIdentityConflict => ("submission-identity-conflict", false),
            ErrorCode::SubmissionFuture => ("submission-future", false),
            ErrorCode::WebAdminUnsupported => ("web-admin-unsupported", false),
            ErrorCode::WebAdminExpired => ("web-admin-expired", true),
            ErrorCode::WebAdminWrongAccount => ("web-admin-wrong-account", false),
            ErrorCode::WebAdminDestinationRejected => ("web-admin-destination-rejected", false),
            ErrorCode::WebAdminUnavailable => ("web-admin-unavailable", true),
            ErrorCode::ImportVerificationRequired => ("import-verification-required", false),
            ErrorCode::BotToken => ("bot-token", false),
            ErrorCode::BotTokenLocked => ("bot-token-locked", true),
            ErrorCode::ChatInvalidInput => ("chat-invalid-input", false),
            ErrorCode::ChatUnsupported => ("chat-unsupported", false),
            ErrorCode::ChatAccessDenied => ("chat-access-denied", false),
            ErrorCode::ChatRefreshRequired => ("chat-refresh-required", true),
            ErrorCode::ChatReprepareRequired => ("chat-reprepare-required", false),
            ErrorCode::ChatNotFound => ("chat-not-found", false),
            ErrorCode::ChatKeyUnavailable => ("chat-key-unavailable", false),
            ErrorCode::ChatLimit => ("chat-limit", false),
            ErrorCode::ChatOperationState => ("chat-operation-state", false),
            ErrorCode::ChatNameConflict => ("chat-name-conflict", false),
            ErrorCode::ChatRandomness => ("chat-randomness", true),
            ErrorCode::ChatChannelIntegrity => ("chat-channel-integrity", false),
            ErrorCode::ChatIntegrity => ("chat-integrity", false),
            ErrorCode::InvalidRequest => ("invalid-request", false),
            ErrorCode::VersionMismatch => ("version-mismatch", false),
            ErrorCode::BootstrapRequired => ("bootstrap-required", false),
            ErrorCode::CatalogSnapshotChanged => ("catalog-snapshot-changed", true),
            ErrorCode::UnsupportedSchema => ("unsupported-schema", false),
            ErrorCode::Conflict => ("conflict", false),
            // Retryable: the reader corrects the passphrase in place and
            // submits the same change again.
            ErrorCode::CurrentPassphraseRejected => ("current-passphrase-rejected", true),
            ErrorCode::Busy => ("busy", true),
            ErrorCode::DeadlineExceeded => ("deadline-exceeded", true),
            ErrorCode::CapabilityDenied => ("capability-denied", false),
            ErrorCode::RollbackDetected => ("rollback-detected", false),
            ErrorCode::CheckpointResetRequired => ("checkpoint-reset-required", false),
            ErrorCode::ProfileBusy => ("profile-busy", true),
            ErrorCode::RateLimited => ("rate-limited", true),
            ErrorCode::QuotaExceeded => ("quota-exceeded", false),
            ErrorCode::ReauthenticationRequired => ("reauthentication-required", false),
            ErrorCode::OperationFailed => ("operation-failed", false),
        };
        let mut mapped = Self::new(slug, message, retryable);
        mapped.ambiguous = code == ErrorCode::DeadlineExceeded;
        mapped.fatal = matches!(
            code,
            ErrorCode::VersionMismatch
                | ErrorCode::UnsupportedSchema
                | ErrorCode::ChatIntegrity
                | ErrorCode::ChatChannelIntegrity
        );
        mapped
    }

    pub fn from_desktop(error: DesktopAgentError) -> Self {
        match error {
            DesktopAgentError::Protocol {
                code,
                message,
                fields,
            } => {
                let mut mapped = Self::from_agent(code, message);
                mapped.details = error_details(*fields).map(Box::new);
                mapped
            }
            DesktopAgentError::Transport(message) => {
                Self::from_ipc(IpcErrorCode::Io, message, false, true)
            }
            DesktopAgentError::Local(condition) => match condition {
                foks_desktop::LocalAgentCondition::Maintenance => Self::new(
                    "state-maintenance-active",
                    "State maintenance is in progress.",
                    true,
                ),
                foks_desktop::LocalAgentCondition::RestartRequired => Self::new(
                    "state-restart-required",
                    "Restart FOKS to use the selected state root.",
                    false,
                ),
                foks_desktop::LocalAgentCondition::RecoveryRequired => Self::new(
                    "state-recovery-required",
                    "Recover client state before restarting the local agent.",
                    false,
                ),
                foks_desktop::LocalAgentCondition::RestorationFailed => Self::new(
                    "state-restoration-failed",
                    "The local agent could not be restored after state maintenance.",
                    true,
                ),
            },
            DesktopAgentError::Ambiguous(message) => {
                Self::from_ipc(IpcErrorCode::Ambiguous, message, true, false)
            }
            DesktopAgentError::Cancelled => Self::from_ipc(
                IpcErrorCode::Cancelled,
                "The request was cancelled.".into(),
                false,
                false,
            ),
            DesktopAgentError::DeadlineExceeded => Self::from_ipc(
                IpcErrorCode::DeadlineExceeded,
                "The request deadline exceeded.".into(),
                false,
                false,
            ),
            DesktopAgentError::Ipc {
                code,
                message,
                ambiguous,
                connection_lost,
            } => Self::from_ipc(code, message, ambiguous, connection_lost),
        }
    }
}

fn error_details(fields: ErrorFields) -> Option<AgentErrorDetails> {
    let details = AgentErrorDetails {
        capability: fields.capability,
        profile: fields.profile,
        state_dir: fields.state_dir,
        reason: fields.reason,
        found_schema: fields.found_schema,
        supported_schema: fields.supported_schema,
    };
    (details.capability.is_some()
        || details.profile.is_some()
        || details.state_dir.is_some()
        || details.reason.is_some()
        || details.found_schema.is_some()
        || details.supported_schema.is_some())
    .then_some(details)
}

pub const MAINTENANCE_EVENT: &str = "foks://maintenance-status";
/// Notifies the webview when the transport first records a connection
/// loss. The error message is subsequently retrieved via `take_agent_connection_loss`.
pub const CONNECTION_LOSS_EVENT: &str = "foks://agent-connection-loss";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceKind {
    Export,
    Import,
    Verify,
    Relocate,
    /// Stop the agent and start it again, with no operation in between.
    Restart,
}

/// What Settings shows about the process answering on the socket.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProcessInfo {
    pub pid: Option<u32>,
    pub executable: Option<String>,
    /// Seconds since the Unix epoch.
    pub started_at: Option<u64>,
    /// Whether this app launched — or adopted — the process, so maintenance
    /// can stop it and quitting terminates it.
    pub owned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaleAgentProcess {
    pub pid: u32,
    pub executable: PathBuf,
    pub started_at: u64,
    pub(crate) state_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenancePhase {
    Selecting,
    Confirming,
    Quiescing,
    Running,
    Restoring,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum MaintenanceOperationOutcome {
    Cancelled,
    Completed,
    Failed { error: AgentError },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum MaintenanceDisposition {
    ContinueCurrentRoot,
    RestartSelectedRoot { root: String },
    RecoveryRequired { root: String },
    RestorationFailed { root: String, error: AgentError },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum MaintenanceSnapshot {
    Idle {
        generation: u64,
        revision: u64,
    },
    Active {
        generation: u64,
        revision: u64,
        kind: MaintenanceKind,
        phase: MaintenancePhase,
    },
    Complete {
        generation: u64,
        revision: u64,
        kind: MaintenanceKind,
        operation: MaintenanceOperationOutcome,
        disposition: MaintenanceDisposition,
    },
}

impl MaintenanceSnapshot {
    fn generation(&self) -> u64 {
        match self {
            Self::Idle { generation, .. }
            | Self::Active { generation, .. }
            | Self::Complete { generation, .. } => *generation,
        }
    }

    fn set_revision(&mut self, revision: u64) {
        match self {
            Self::Idle {
                revision: value, ..
            }
            | Self::Active {
                revision: value, ..
            }
            | Self::Complete {
                revision: value, ..
            } => *value = revision,
        }
    }

    fn transition_rank(&self) -> u8 {
        match self {
            Self::Idle { .. } => 0,
            Self::Active { phase, .. } => match phase {
                MaintenancePhase::Selecting => 1,
                MaintenancePhase::Confirming => 2,
                MaintenancePhase::Quiescing => 3,
                MaintenancePhase::Running => 4,
                MaintenancePhase::Restoring => 5,
            },
            Self::Complete { .. } => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportDisposition {
    Current,
    RestartRequired,
    RecoveryRequired,
    RestorationFailed,
}

trait MaintenanceProcess: Send + Sync {
    fn preflight(&self, handle: &AgentHandle) -> Result<(), AgentError>;
    fn stop(&self, handle: &AgentHandle) -> Result<(), AgentError>;
    fn restore(&self, handle: &AgentHandle) -> Result<Response, AgentError>;
}

struct NativeMaintenanceProcess;

impl MaintenanceProcess for NativeMaintenanceProcess {
    fn preflight(&self, handle: &AgentHandle) -> Result<(), AgentError> {
        handle.require_owned_managed_agent()
    }

    fn stop(&self, handle: &AgentHandle) -> Result<(), AgentError> {
        handle.stop_owned_managed_agent()
    }

    fn restore(&self, handle: &AgentHandle) -> Result<Response, AgentError> {
        handle.require_maintenance_stop_settled()?;
        handle.ensure_started_already_reserved()
    }
}

struct MaintenanceAdmission<'a>(&'a AtomicBool);

impl Drop for MaintenanceAdmission<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Clears the drain signal once the reservation is taken or given up on. The
/// exclusive reservation keeps later calls out by itself, so the signal only
/// has to last as long as the wait for it.
struct MaintenanceDrain<'a>(&'a ObservedTransport);

impl Drop for MaintenanceDrain<'_> {
    fn drop(&mut self) {
        self.0.maintenance_pending.store(false, Ordering::Release);
    }
}

pub enum MaintenanceCompletion {
    Cancelled,
    Continue,
    RestartSelected(PathBuf),
    Failed {
        error: AgentError,
        affected_roots: Vec<PathBuf>,
    },
}

pub struct MaintenanceWorker<'a> {
    handle: &'a AgentHandle,
    generation: u64,
    kind: MaintenanceKind,
    stop_attempted: bool,
    notify: &'a dyn Fn(&MaintenanceSnapshot),
}

impl MaintenanceWorker<'_> {
    pub fn source_root(&self) -> Result<PathBuf, AgentError> {
        self.handle
            .socket
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| AgentError::unknown("Agent state path has no parent."))
    }

    pub fn phase(&self, phase: MaintenancePhase) {
        self.handle.publish_maintenance(
            MaintenanceSnapshot::Active {
                generation: self.generation,
                revision: 0,
                kind: self.kind,
                phase,
            },
            self.notify,
        );
    }

    pub fn stop_owned_agent(&mut self) -> Result<(), AgentError> {
        self.phase(MaintenancePhase::Quiescing);
        // Once SIGTERM may have been sent, restoration is mandatory even if
        // confirmation of process exit times out.
        self.stop_attempted = true;
        self.handle.maintenance_process.stop(self.handle)?;
        self.phase(MaintenancePhase::Running);
        Ok(())
    }
}

/// How long a maintenance command waits for the transport to fall idle before
/// it reports that other requests hold it. Read-only calls are asked to cancel
/// as soon as the wait starts, so this budget covers the mutations and uploads
/// that have to run to completion rather than the parked reads.
const MAINTENANCE_RESERVATION_WAIT: Duration = Duration::from_secs(5);

/// Closed while the desktop's startup check runs on a background thread, so a
/// command the webview issues meanwhile waits for a verified agent instead of
/// reaching a socket that is not bound yet.
///
/// The startup check itself never waits on this: it reserves the transport
/// directly and probes with `call_unreserved`, neither of which consults the
/// gate. Gating the reservation instead would deadlock the check against its
/// own gate, because `ensure_started_with_confirmation` reserves while the
/// gate is closed.
#[derive(Default)]
struct StartupGate {
    open: Mutex<bool>,
    ready: Condvar,
}

impl StartupGate {
    fn new_open() -> Self {
        Self {
            open: Mutex::new(true),
            ready: Condvar::new(),
        }
    }

    fn hold(&self) {
        *self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
    }

    fn release(&self) {
        *self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.ready.notify_all();
    }

    fn wait(&self) {
        let mut guard = self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*guard {
            guard = self
                .ready
                .wait(guard)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

struct ObservedTransport {
    client: AgentClient,
    /// Shared with the handle that opens it.
    startup_gate: Arc<StartupGate>,
    maintenance: RwLock<()>,
    /// Set while a maintenance command waits for its exclusive reservation.
    /// Calls that are safe to abandon observe it and cancel, and no further
    /// call starts, so the reservation is not held off indefinitely by reads
    /// that park on the agent: a chat inbox poll alone waits 55 seconds.
    maintenance_pending: AtomicBool,
    disposition: Mutex<TransportDisposition>,
    connection_failure: Arc<Mutex<Option<String>>>,
    /// Callback invoked when a connection loss occurs, eliminating the need
    /// for client polling. Implemented as a generic callback to decouple the
    /// transport from window and application handles.
    connection_loss_notifier: OnceLock<ConnectionLossNotifier>,
    /// Every operation this transport issues, timed, for Copy diagnostics.
    timings: Arc<TimingLog>,
}

impl ObservedTransport {
    #[allow(clippy::result_large_err)] // Uses the existing transport trait error without allocating on success.
    fn reserve_use(&self) -> Result<std::sync::RwLockReadGuard<'_, ()>, DesktopAgentError> {
        if self.maintenance_pending.load(Ordering::Acquire) {
            return Err(DesktopAgentError::Local(
                foks_desktop::LocalAgentCondition::Maintenance,
            ));
        }
        let guard = self.maintenance.try_read().map_err(|_| {
            DesktopAgentError::Local(foks_desktop::LocalAgentCondition::Maintenance)
        })?;
        self.require_current()?;
        Ok(guard)
    }

    /// True once a maintenance command is waiting and this call may be
    /// abandoned. A mutation is never abandoned: its outcome would become
    /// ambiguous, which is worse than making maintenance wait for it.
    fn preempted(&self, preemptible: bool) -> bool {
        preemptible && self.maintenance_pending.load(Ordering::Acquire)
    }

    /// Takes the exclusive reservation that maintenance runs under, asking
    /// in-flight preemptible calls to stop first. Returns `None` if calls were
    /// still outstanding when the wait ran out.
    fn reserve_for_maintenance(
        &self,
        wait: Duration,
    ) -> Option<std::sync::RwLockWriteGuard<'_, ()>> {
        self.maintenance_pending.store(true, Ordering::Release);
        let _drain = MaintenanceDrain(self);
        let deadline = std::time::Instant::now() + wait;
        loop {
            match self.maintenance.try_write() {
                Ok(guard) => return Some(guard),
                Err(std::sync::TryLockError::Poisoned(error)) => return Some(error.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    #[allow(clippy::result_large_err)]
    fn require_current(&self) -> Result<(), DesktopAgentError> {
        match *self
            .disposition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            TransportDisposition::Current => {}
            TransportDisposition::RestartRequired => {
                return Err(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::RestartRequired,
                ));
            }
            TransportDisposition::RecoveryRequired => {
                return Err(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::RecoveryRequired,
                ));
            }
            TransportDisposition::RestorationFailed => {
                return Err(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::RestorationFailed,
                ));
            }
        }
        Ok(())
    }

    fn record<T>(&self, result: &Result<T, DesktopAgentError>) {
        let Err(error) = result else { return };
        if !error.connection_lost() {
            return;
        }
        let first = {
            let mut failure = self
                .connection_failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let first = failure.is_none();
            *failure = Some(error.to_string());
            first
        };
        // Notify only on the initial transition to a disconnected state to
        // avoid redundant events while an error is already pending. Release
        // the lock before calling the notifier because the callback may
        // synchronously query connection status.
        if first {
            if let Some(notify) = self.connection_loss_notifier.get() {
                notify();
            }
        }
    }

    /// Times one round trip and records it under the command that issued it.
    /// A response that carries the agent's own phase timing has it recorded
    /// beside the round trip.
    fn observe(
        &self,
        label: Option<&Label>,
        operation: &'static str,
        started: Instant,
        result: &Result<Response, DesktopAgentError>,
    ) {
        let outcome = outcome_of(result.as_ref().map(|response| &response.result));
        let timing = result
            .as_ref()
            .ok()
            .and_then(|response| response.timing.as_ref());
        self.timings.record(agent_operation(
            label,
            operation,
            started.elapsed(),
            outcome,
            timing,
        ));
    }

    #[allow(clippy::result_large_err)]
    fn call_observed(
        &self,
        label: Option<&Label>,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, DesktopAgentError> {
        self.startup_gate.wait();
        let _use = self.reserve_use()?;
        let name = operation.name();
        let preemptible = !operation.is_mutation();
        let started = Instant::now();
        let response = self
            .client
            .call_cancellable(operation, &|| cancelled() || self.preempted(preemptible))
            .map_err(client_to_desktop);
        self.observe(label, name, started, &response);
        let result = response.and_then(|response| response_result(response.result));
        self.record(&result);
        result
    }

    #[allow(clippy::result_large_err)]
    fn stream_observed(
        &self,
        label: Option<&Label>,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<Value, DesktopAgentError> {
        self.startup_gate.wait();
        let _use = self.reserve_use()?;
        let started = Instant::now();
        let response = self
            .client
            .put_kv_stream(header, reader)
            .map_err(client_to_desktop);
        self.observe(label, "PutKvStream", started, &response);
        let result = response.and_then(|response| response_result(response.result));
        self.record(&result);
        result
    }
}

impl AgentTransport for ObservedTransport {
    fn call_cancellable(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, DesktopAgentError> {
        self.call_observed(None, operation, cancelled)
    }

    fn call(&self, operation: Operation) -> Result<Value, DesktopAgentError> {
        self.call_observed(None, operation, &|| false)
    }

    fn put_kv_stream(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<Value, DesktopAgentError> {
        self.stream_observed(None, header, reader)
    }
}

/// The transport a command hands to the shared catalog, roster and chat
/// code, so the operations issued on the command's behalf, on whatever
/// thread, are recorded under its name and profile.
struct LabelledTransport {
    inner: Arc<ObservedTransport>,
    label: Label,
}

impl AgentTransport for LabelledTransport {
    fn call_cancellable(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, DesktopAgentError> {
        self.inner
            .call_observed(Some(&self.label), operation, cancelled)
    }

    fn call(&self, operation: Operation) -> Result<Value, DesktopAgentError> {
        self.inner
            .call_observed(Some(&self.label), operation, &|| false)
    }

    fn put_kv_stream(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<Value, DesktopAgentError> {
        self.inner
            .stream_observed(Some(&self.label), header, reader)
    }
}

#[allow(clippy::result_large_err)]
fn response_result(result: ResponseResult) -> Result<Value, DesktopAgentError> {
    match result {
        ResponseResult::Success { value } => Ok(value),
        ResponseResult::Error {
            code,
            message,
            fields,
        } => Err(DesktopAgentError::Protocol {
            code,
            message,
            fields: fields.into(),
        }),
    }
}

fn client_to_desktop(error: foks_agent_client::Error) -> DesktopAgentError {
    foks_desktop::agent_client_error(error)
}

type MaintenanceReadiness = dyn Fn(&Path, &[PathBuf]) -> SafeRootDisposition + Send + Sync;

type ConnectionLossNotifier = Arc<dyn Fn() + Send + Sync>;

pub struct AgentHandle {
    transport: Arc<ObservedTransport>,
    socket: PathBuf,
    connection_failure: Arc<Mutex<Option<String>>>,
    maintenance_generation: AtomicU64,
    maintenance_revision: AtomicU64,
    maintenance_in_flight: AtomicBool,
    recovery_credentials_required: AtomicBool,
    maintenance_snapshot: Mutex<MaintenanceSnapshot>,
    pending_stop_pid: Mutex<Option<u32>>,
    maintenance_process: Arc<dyn MaintenanceProcess>,
    maintenance_readiness: Arc<MaintenanceReadiness>,
    /// Closed while the desktop's startup check runs on a background thread
    /// (see `startup::require_agent`). Ordinary commands wait on it so the
    /// frontend cannot reach an agent that has not been verified, started, or
    /// taken over yet. Open by default so tests and the smoke test are
    /// unaffected.
    startup_gate: Arc<StartupGate>,
    #[cfg(test)]
    managed_endpoint_override: bool,
}

impl AgentHandle {
    pub fn new(socket: PathBuf) -> Self {
        let mut client = AgentClient::new(&socket);
        client
            .set_timeout(Duration::from_secs(60))
            .expect("the agent client accepts a 60-second timeout");
        let connection_failure = Arc::new(Mutex::new(None));
        let startup_gate = Arc::new(StartupGate::new_open());
        Self {
            transport: Arc::new(ObservedTransport {
                client,
                startup_gate: Arc::clone(&startup_gate),
                maintenance: RwLock::new(()),
                maintenance_pending: AtomicBool::new(false),
                disposition: Mutex::new(TransportDisposition::Current),
                connection_failure: Arc::clone(&connection_failure),
                connection_loss_notifier: OnceLock::new(),
                timings: Arc::default(),
            }),
            socket,
            connection_failure,
            maintenance_generation: AtomicU64::new(0),
            maintenance_revision: AtomicU64::new(0),
            maintenance_in_flight: AtomicBool::new(false),
            recovery_credentials_required: AtomicBool::new(false),
            maintenance_snapshot: Mutex::new(MaintenanceSnapshot::Idle {
                generation: 0,
                revision: 0,
            }),
            pending_stop_pid: Mutex::new(None),
            maintenance_process: Arc::new(NativeMaintenanceProcess),
            maintenance_readiness: Arc::new(safe_selected_root),
            startup_gate,
            #[cfg(test)]
            managed_endpoint_override: false,
        }
    }

    /// Holds ordinary commands until `release_startup` runs. Call before the
    /// webview can issue its first command.
    pub fn hold_commands_for_startup(&self) {
        self.startup_gate.hold();
    }

    /// Opens the startup gate and wakes every command waiting on it.
    pub fn release_startup(&self) {
        self.startup_gate.release();
    }

    fn wait_for_startup(&self) {
        self.startup_gate.wait();
    }

    #[cfg(test)]
    fn new_for_maintenance_test(
        socket: PathBuf,
        process: Arc<dyn MaintenanceProcess>,
        readiness: impl Fn(&Path, &[PathBuf]) -> SafeRootDisposition + Send + Sync + 'static,
    ) -> Self {
        let mut handle = Self::new(socket);
        handle.maintenance_process = process;
        handle.maintenance_readiness = Arc::new(readiness);
        handle.managed_endpoint_override = true;
        handle
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn auto_recover_blocking(&self) -> Result<Response, AgentError> {
        self.auto_recover_with_reservation_timeout(MAINTENANCE_RESERVATION_WAIT)
    }

    fn auto_recover_with_reservation_timeout(
        &self,
        reservation_timeout: Duration,
    ) -> Result<Response, AgentError> {
        self.wait_for_startup();
        self.maintenance_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                AgentError::from_desktop(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::Maintenance,
                ))
            })?;
        let _admission = MaintenanceAdmission(&self.maintenance_in_flight);
        let _reservation = self
            .transport
            .reserve_for_maintenance(reservation_timeout)
            .ok_or_else(|| {
                AgentError::new(
                    "agent-busy",
                    "Outstanding agent requests are still settling.",
                    true,
                )
            })?;
        self.transport
            .require_current()
            .map_err(AgentError::from_desktop)?;
        let initial_error = match self.probe_status() {
            Ok(response) => {
                self.clear_connection_failure();
                return Ok(response);
            }
            Err(error) if error.code == "agent-lost" => error,
            Err(error) => return Err(error),
        };
        self.require_managed_endpoint()?;
        self.require_missing_agent_endpoint()?;
        if MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
            || self
                .pending_stop_pid
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
        {
            return Err(AgentError::new(
                "agent-busy",
                "The managed agent has not exited.",
                true,
            ));
        }
        self.check_recovery_credentials(false)?;
        let binary = managed_agent_binary(&self.socket).ok_or_else(|| {
            AgentError::new(
                "agent-start-failed",
                "The managed agent binary is unavailable.",
                false,
            )
        })?;
        validate_agent_binary(&binary)?;
        let root = self
            .socket
            .parent()
            .ok_or_else(|| AgentError::unknown("Agent state path has no parent."))?;
        let _root_lease = foks_client_app::ClientStateLease::acquire(root)
            .map_err(|error| AgentError::new("agent-state", error.to_string(), false))?;
        let _spawn_lock = acquire_spawn_lock(&self.socket)?;
        match self.probe_status() {
            Ok(response) => {
                self.clear_connection_failure();
                return Ok(response);
            }
            Err(error) if error.code == "agent-lost" => self.require_missing_agent_endpoint()?,
            Err(error) => return Err(error),
        }
        // Recheck protected recovery facts under the spawn lock.
        self.check_recovery_credentials(false)?;
        let mut launch = launch_agent(&binary, root, &self.socket)?;
        let mut last_error = initial_error;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            match self.probe_status() {
                Ok(response) => {
                    self.clear_connection_failure();
                    return Ok(response);
                }
                Err(error) if error.code == "agent-lost" => last_error = error,
                Err(error) => return Err(error),
            }
            if let Some(error) = launch.early_exit_error() {
                return Err(error);
            }
        }
        Err(last_error)
    }

    fn require_missing_agent_endpoint(&self) -> Result<(), AgentError> {
        use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
        let metadata = match std::fs::symlink_metadata(&self.socket) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(AgentError::from_client(&error.into())),
            Ok(metadata) => metadata,
        };
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(AgentError::from_client(
                &foks_agent_client::Error::UnsafeSocket,
            ));
        }
        match std::os::unix::net::UnixStream::connect(&self.socket) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
            Err(error) => return Err(AgentError::from_client(&error.into())),
            Ok(_) => {
                return Err(AgentError::new(
                    "agent-busy",
                    "An existing listener owns the agent endpoint.",
                    true,
                ))
            }
        }
        let current = std::fs::symlink_metadata(&self.socket)
            .map_err(|error| AgentError::from_client(&error.into()))?;
        if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
            return Err(AgentError::new(
                "agent-busy",
                "The agent endpoint changed during recovery.",
                true,
            ));
        }
        Ok(())
    }

    fn check_recovery_credentials(&self, interactive: bool) -> Result<(), AgentError> {
        let result = if interactive {
            self.require_auto_recovery_root()
        } else {
            foks_keystore::without_user_interaction(|| self.require_auto_recovery_root())
        };
        match &result {
            Ok(()) => self
                .recovery_credentials_required
                .store(false, Ordering::Release),
            Err(error) if error.code == "agent-credentials-required" => {
                self.recovery_credentials_required
                    .store(true, Ordering::Release);
            }
            _ => {}
        }
        result
    }

    fn require_auto_recovery_root(&self) -> Result<(), AgentError> {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let root = self
            .socket
            .parent()
            .ok_or_else(|| AgentError::unknown("Agent state path has no parent."))?;
        for (path, directory) in [
            (root.to_path_buf(), true),
            (root.join("client-state.toml"), false),
        ] {
            let metadata = std::fs::symlink_metadata(path)
                .map_err(|error| AgentError::new("agent-state", error.to_string(), false))?;
            if metadata.file_type().is_symlink()
                || (directory && !metadata.is_dir())
                || (!directory && !metadata.is_file())
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.permissions().mode() & 0o077 != 0
            {
                return Err(AgentError::new(
                    "agent-state",
                    "Automatic recovery requires existing private client state.",
                    false,
                ));
            }
        }
        match (self.maintenance_readiness)(root, &[]) {
            SafeRootDisposition::CredentialsRequired(_) => Err(AgentError::new(
                "agent-credentials-required",
                "Keychain access is needed to restore your connection.",
                false,
            )),
            SafeRootDisposition::Current => Ok(()),
            SafeRootDisposition::Selected(_) => Err(AgentError::from_desktop(
                DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RestartRequired),
            )),
            SafeRootDisposition::Recovery(_) => Err(AgentError::from_desktop(
                DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RecoveryRequired),
            )),
        }
    }

    /// Recovery is limited to a missing, desktop-managed endpoint with a usable
    /// replacement binary. External launcher paths must never become delete targets.
    pub fn startup_reset_directory(&self) -> Option<PathBuf> {
        self.require_managed_endpoint().ok()?;
        if self.socket.exists() {
            return None;
        }
        validate_agent_binary(&managed_agent_binary(&self.socket)?).ok()?;
        let root = self.socket.parent()?;
        root.join("client-state.toml")
            .exists()
            .then(|| root.to_owned())
    }

    pub fn reset_startup_state(&self, confirmed_root: &Path) -> Result<(), AgentError> {
        let root = self.startup_reset_directory().ok_or_else(|| {
            AgentError::new(
                "agent-reset",
                "This agent state cannot be reset from startup.",
                false,
            )
        })?;
        if root != confirmed_root {
            return Err(AgentError::new(
                "agent-reset",
                "The selected state directory changed. Relaunch FOKS to review the reset again.",
                false,
            ));
        }
        foks_client_app::portability::reset_unreadable_state(&root)
            .map_err(|error| AgentError::new("agent-reset", error.to_string(), false))?;
        prepare_managed_crash_directory(&root.join("crashes"))
    }

    pub fn transport(&self) -> Arc<dyn AgentTransport> {
        self.transport.clone()
    }

    /// The transport for one command's work, recording each operation it
    /// issues under the command's name and, when it has one, its profile.
    pub fn transport_for(
        &self,
        command: &'static str,
        scope: Option<&str>,
    ) -> Arc<dyn AgentTransport> {
        Arc::new(LabelledTransport {
            inner: Arc::clone(&self.transport),
            label: Label {
                command,
                scope: scope.map(str::to_owned),
            },
        })
    }

    /// The backend's timing log, read by Copy diagnostics.
    pub fn timings(&self) -> Arc<TimingLog> {
        Arc::clone(&self.transport.timings)
    }

    /// Records background-loop timings reported by agent status. Deduplication
    /// ensures each loop execution appears once in the timing log.
    pub fn note_agent_timers(&self, status: &foks_agent_proto::AgentStatus) {
        self.transport.timings.record_agent_timers(status.timers());
    }

    pub async fn call(self: &Arc<Self>, operation: Operation) -> Result<Response, AgentError> {
        let handle = Arc::clone(self);
        tauri::async_runtime::spawn_blocking(move || handle.call_blocking(operation))
            .await
            .map_err(|error| {
                AgentError::unknown(format!("Agent call failed to complete: {error}"))
            })?
    }

    pub fn call_blocking(&self, operation: Operation) -> Result<Response, AgentError> {
        self.wait_for_startup();
        let _use = self
            .transport
            .reserve_use()
            .map_err(AgentError::from_desktop)?;
        let name = operation.name();
        let preemptible = !operation.is_mutation();
        let started = Instant::now();
        let result = self
            .transport
            .client
            .call_cancellable(operation, &|| self.transport.preempted(preemptible))
            .map_err(client_to_desktop);
        self.transport.observe(None, name, started, &result);
        self.transport.record(&result);
        result.map_err(AgentError::from_desktop)
    }

    fn call_unreserved(&self, operation: Operation) -> Result<Response, AgentError> {
        self.transport
            .client
            .call(operation)
            .map_err(|error| AgentError::from_client(&error))
    }

    fn probe_status(&self) -> Result<Response, AgentError> {
        let response = self.call_unreserved(Operation::AgentStatus)?;
        match &response.result {
            ResponseResult::Success { value } => {
                let status = serde_json::from_value::<foks_agent_proto::AgentStatus>(value.clone())
                    .map_err(|error| {
                        AgentError::new("protocol", format!("Invalid agent status: {error}"), false)
                    })?;
                self.transport.timings.record_agent_timers(status.timers());
            }
            ResponseResult::Error {
                code,
                message,
                fields,
            } => {
                return Err(AgentError::from_desktop(DesktopAgentError::Protocol {
                    code: *code,
                    message: message.clone(),
                    fields: fields.clone().into(),
                }));
            }
        }
        Ok(response)
    }

    #[cfg(test)]
    pub fn ensure_started_blocking(&self) -> Result<Response, AgentError> {
        let _use = self
            .transport
            .reserve_use()
            .map_err(AgentError::from_desktop)?;
        self.require_maintenance_stop_settled()?;
        self.ensure_started_already_reserved()
    }

    pub fn ensure_started_with_confirmation(
        &self,
        confirm: &dyn Fn(&AgentTakeover) -> bool,
    ) -> Result<Response, AgentError> {
        let _use = self
            .transport
            .reserve_use()
            .map_err(AgentError::from_desktop)?;
        self.require_maintenance_stop_settled()?;
        self.start_already_reserved(&|target| Ok(confirm(target)))
    }

    /// Retries agent restoration while excluding ordinary commands. A failed
    /// maintenance restoration remains a typed local transport condition until
    /// this method has verified an authoritative agent status response.
    pub fn retry_started_blocking(&self) -> Result<Response, AgentError> {
        let _reservation = self
            .transport
            .maintenance
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *self
            .transport
            .disposition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            TransportDisposition::Current | TransportDisposition::RestorationFailed => {}
            TransportDisposition::RestartRequired => {
                return Err(AgentError::from_desktop(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::RestartRequired,
                )));
            }
            TransportDisposition::RecoveryRequired => {
                return Err(AgentError::from_desktop(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::RecoveryRequired,
                )));
            }
        }
        // Only an explicit retry may authorize the desktop's protected-state
        // inspection after automatic recovery was blocked by the keychain.
        if self.recovery_credentials_required.load(Ordering::Acquire) {
            self.check_recovery_credentials(true)?;
        }
        let response = self.maintenance_process.restore(self)?;
        *self
            .transport
            .disposition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = TransportDisposition::Current;
        Ok(response)
    }

    /// Starts and verifies the managed agent while the caller owns the
    /// exclusive maintenance reservation. This must never acquire the shared
    /// reservation again.
    fn ensure_started_already_reserved(&self) -> Result<Response, AgentError> {
        self.start_already_reserved(&|_| {
            Err(AgentError::new(
            "agent-takeover-required",
            "Another FOKS version owns the agent socket. Relaunch FOKS to confirm replacing it.",
            false,
        ))
        })
    }

    fn start_already_reserved(
        &self,
        confirm: &dyn Fn(&AgentTakeover) -> Result<bool, AgentError>,
    ) -> Result<Response, AgentError> {
        if let Ok(response) = self.probe_status() {
            self.clear_connection_failure();
            self.adopt_orphaned_agent();
            return Ok(response);
        }
        let Some(binary) = managed_agent_binary(&self.socket) else {
            return self.probe_status();
        };
        self.start_with_binary(&binary, confirm)
    }

    fn start_with_binary(
        &self,
        binary: &Path,
        confirm: &dyn Fn(&AgentTakeover) -> Result<bool, AgentError>,
    ) -> Result<Response, AgentError> {
        // Fail before asking to stop a healthy process if we cannot replace it.
        validate_agent_binary(binary)?;
        let state_dir = self.socket.parent().ok_or_else(|| {
            AgentError::unknown("Configured agent socket path has no parent directory.")
        })?;
        let _root_lease = foks_client_app::ClientStateLease::acquire(state_dir)
            .map_err(|error| AgentError::new("agent-state", error.to_string(), false))?;
        prepare_state_directory(state_dir)?;
        let _spawn_lock = acquire_spawn_lock(&self.socket)?;
        let mut last_error = None;
        // The socket may be rebound after the previous process exits. Require new
        // approval for each replacement process and limit retries for respawning services.
        let mut approvals = 0;
        for _ in 0..=MAX_STARTUP_TAKEOVERS {
            match self.probe_status() {
                Ok(response) => {
                    self.clear_connection_failure();
                    self.adopt_orphaned_agent();
                    return Ok(response);
                }
                #[cfg(unix)]
                Err(error) if error.code == "version-mismatch" => {
                    if let Err(error) =
                        stop_incompatible_agent(&self.socket, confirm, &mut approvals)
                    {
                        if error.code == "agent-takeover-changed" {
                            last_error = Some(error);
                            continue;
                        }
                        return Err(error);
                    }
                    // Recheck before spawning: a new listener may already own
                    // the path, including a compatible agent we can simply use.
                    match self.probe_status() {
                        Ok(response) => {
                            self.clear_connection_failure();
                            return Ok(response);
                        }
                        Err(error) if error.code == "version-mismatch" => {
                            last_error = Some(error);
                            continue;
                        }
                        Err(_) => {}
                    }
                }
                Err(_) => {}
            }
            let mut launch = launch_agent(binary, state_dir, &self.socket)?;
            let mut replaced = false;
            for _ in 0..50 {
                std::thread::sleep(Duration::from_millis(100));
                match self.probe_status() {
                    Ok(response) => {
                        self.clear_connection_failure();
                        return Ok(response);
                    }
                    Err(error) if error.code == "version-mismatch" => {
                        last_error = Some(error);
                        replaced = true;
                        break;
                    }
                    Err(error) => last_error = Some(error),
                }
                if let Some(error) = launch.early_exit_error() {
                    return Err(error);
                }
            }
            if !replaced {
                break;
            }
        }
        Err(last_error.unwrap_or_else(|| {
            AgentError::new(
                "agent-start-failed",
                "Failed to connect: background service endpoint was not created.",
                true,
            )
        }))
    }

    fn require_managed_endpoint(&self) -> Result<(), AgentError> {
        #[cfg(test)]
        if self.managed_endpoint_override {
            return Ok(());
        }
        if socket_from_arguments(std::env::args_os()).is_some()
            || std::env::var_os(SOCKET_ENV).is_some()
            || default_socket().as_deref() != Some(self.socket())
        {
            return Err(AgentError::new(
                "external-agent",
                "This FOKS is using an agent socket set by its launcher. Use the CLI to manage or restart that agent.",
                false,
            ));
        }
        Ok(())
    }

    pub fn maintenance_snapshot(&self) -> MaintenanceSnapshot {
        self.maintenance_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn publish_maintenance(
        &self,
        mut snapshot: MaintenanceSnapshot,
        notify: &dyn Fn(&MaintenanceSnapshot),
    ) {
        let mut current = self
            .maintenance_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restoration_retry = matches!(
            (&*current, &snapshot),
            (
                MaintenanceSnapshot::Complete {
                    disposition: MaintenanceDisposition::RestorationFailed { .. },
                    ..
                },
                MaintenanceSnapshot::Complete {
                    disposition: MaintenanceDisposition::ContinueCurrentRoot,
                    ..
                }
            )
        );
        if snapshot.generation() < current.generation()
            || (snapshot.generation() == current.generation()
                && snapshot.transition_rank() <= current.transition_rank()
                && !restoration_retry)
        {
            return;
        }
        let revision = self.maintenance_revision.fetch_add(1, Ordering::AcqRel) + 1;
        snapshot.set_revision(revision);
        *current = snapshot.clone();
        drop(current);
        notify(&snapshot);
    }

    pub fn record_restoration_success(&self, notify: &dyn Fn(&MaintenanceSnapshot)) {
        let previous = self.maintenance_snapshot();
        if let MaintenanceSnapshot::Complete {
            generation,
            kind,
            operation,
            disposition: MaintenanceDisposition::RestorationFailed { .. },
            ..
        } = previous
        {
            *self
                .transport
                .disposition
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = TransportDisposition::Current;
            self.publish_maintenance(
                MaintenanceSnapshot::Complete {
                    generation,
                    revision: 0,
                    kind,
                    operation,
                    disposition: MaintenanceDisposition::ContinueCurrentRoot,
                },
                notify,
            );
        }
    }

    pub fn run_maintenance(
        &self,
        kind: MaintenanceKind,
        notify: &dyn Fn(&MaintenanceSnapshot),
        operation: impl FnOnce(&mut MaintenanceWorker<'_>) -> MaintenanceCompletion,
    ) -> Result<MaintenanceSnapshot, AgentError> {
        self.require_managed_endpoint()?;
        self.maintenance_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                AgentError::new(
                    "state-busy",
                    "State maintenance is already in progress.",
                    true,
                )
            })?;
        let _admission = MaintenanceAdmission(&self.maintenance_in_flight);
        // Admission precedes endpoint preflight so a second command arriving
        // after the first worker stopped the socket still receives the typed
        // state-busy outcome, rather than a misleading agent-lost result.
        self.maintenance_process.preflight(self)?;
        let _reservation = self
            .transport
            .reserve_for_maintenance(MAINTENANCE_RESERVATION_WAIT)
            .ok_or_else(|| {
                AgentError::new(
                    "state-busy",
                    "Cannot process this right now because of other active requests.",
                    true,
                )
            })?;
        match *self
            .transport
            .disposition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            TransportDisposition::Current => {}
            TransportDisposition::RestartRequired => {
                return Err(AgentError::from_desktop(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::RestartRequired,
                )));
            }
            TransportDisposition::RecoveryRequired => {
                return Err(AgentError::from_desktop(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::RecoveryRequired,
                )));
            }
            TransportDisposition::RestorationFailed => {
                return Err(AgentError::from_desktop(DesktopAgentError::Local(
                    foks_desktop::LocalAgentCondition::RestorationFailed,
                )));
            }
        }
        let generation = self.maintenance_generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.publish_maintenance(
            MaintenanceSnapshot::Active {
                generation,
                revision: 0,
                kind,
                phase: MaintenancePhase::Selecting,
            },
            notify,
        );
        let mut worker = MaintenanceWorker {
            handle: self,
            generation,
            kind,
            stop_attempted: false,
            notify,
        };
        let completion =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&mut worker)))
                .unwrap_or_else(|_| MaintenanceCompletion::Failed {
                    error: AgentError::new(
                        "maintenance-worker-interrupted",
                        "State maintenance worker stopped unexpectedly.",
                        false,
                    ),
                    affected_roots: worker.source_root().into_iter().collect(),
                });
        let source = worker.source_root()?;
        let stop_attempted = worker.stop_attempted;
        let (operation_outcome, mut disposition) = match completion {
            MaintenanceCompletion::Cancelled => (
                MaintenanceOperationOutcome::Cancelled,
                MaintenanceDisposition::ContinueCurrentRoot,
            ),
            MaintenanceCompletion::Continue => (
                MaintenanceOperationOutcome::Completed,
                MaintenanceDisposition::ContinueCurrentRoot,
            ),
            MaintenanceCompletion::RestartSelected(root) => {
                let disposition =
                    match (self.maintenance_readiness)(&source, std::slice::from_ref(&root)) {
                        SafeRootDisposition::Selected(selected) if selected == root => {
                            MaintenanceDisposition::RestartSelectedRoot {
                                root: selected.display().to_string(),
                            }
                        }
                        SafeRootDisposition::Current => MaintenanceDisposition::RecoveryRequired {
                            root: root.display().to_string(),
                        },
                        SafeRootDisposition::Recovery(root)
                        | SafeRootDisposition::CredentialsRequired(root) => {
                            MaintenanceDisposition::RecoveryRequired {
                                root: root.display().to_string(),
                            }
                        }
                        SafeRootDisposition::Selected(selected) => {
                            MaintenanceDisposition::RecoveryRequired {
                                root: selected.display().to_string(),
                            }
                        }
                    };
                (MaintenanceOperationOutcome::Completed, disposition)
            }
            MaintenanceCompletion::Failed {
                error,
                affected_roots,
            } => {
                let disposition = match (self.maintenance_readiness)(&source, &affected_roots) {
                    SafeRootDisposition::Current => MaintenanceDisposition::ContinueCurrentRoot,
                    SafeRootDisposition::Selected(root) => {
                        MaintenanceDisposition::RestartSelectedRoot {
                            root: root.display().to_string(),
                        }
                    }
                    SafeRootDisposition::Recovery(root)
                    | SafeRootDisposition::CredentialsRequired(root) => {
                        MaintenanceDisposition::RecoveryRequired {
                            root: root.display().to_string(),
                        }
                    }
                };
                (MaintenanceOperationOutcome::Failed { error }, disposition)
            }
        };

        if stop_attempted && matches!(disposition, MaintenanceDisposition::ContinueCurrentRoot) {
            disposition = match (self.maintenance_readiness)(&source, &[]) {
                SafeRootDisposition::Current => MaintenanceDisposition::ContinueCurrentRoot,
                SafeRootDisposition::Selected(root) => {
                    MaintenanceDisposition::RestartSelectedRoot {
                        root: root.display().to_string(),
                    }
                }
                SafeRootDisposition::Recovery(root)
                | SafeRootDisposition::CredentialsRequired(root) => {
                    MaintenanceDisposition::RecoveryRequired {
                        root: root.display().to_string(),
                    }
                }
            };
        }
        if stop_attempted && matches!(disposition, MaintenanceDisposition::ContinueCurrentRoot) {
            worker.phase(MaintenancePhase::Restoring);
            if let Err(error) = self.maintenance_process.restore(self) {
                disposition = MaintenanceDisposition::RestorationFailed {
                    root: source.display().to_string(),
                    error,
                };
            }
        }
        let transport_disposition = match disposition {
            MaintenanceDisposition::ContinueCurrentRoot => TransportDisposition::Current,
            MaintenanceDisposition::RestorationFailed { .. } => {
                TransportDisposition::RestorationFailed
            }
            MaintenanceDisposition::RestartSelectedRoot { .. } => {
                TransportDisposition::RestartRequired
            }
            MaintenanceDisposition::RecoveryRequired { .. } => {
                TransportDisposition::RecoveryRequired
            }
        };
        *self
            .transport
            .disposition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = transport_disposition;
        // Intentional stop/start failures are carried by the maintenance
        // snapshot. They must not be reclassified as unrelated agent loss by
        // the connection observer after the exclusive reservation is released.
        self.clear_connection_failure();
        let snapshot = MaintenanceSnapshot::Complete {
            generation,
            revision: 0,
            kind,
            operation: operation_outcome,
            disposition,
        };
        self.publish_maintenance(snapshot, notify);
        Ok(self.maintenance_snapshot())
    }

    fn stop_owned_managed_agent(&self) -> Result<(), AgentError> {
        self.require_owned_managed_agent()?;
        let pid = *MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(pid) = pid else {
            unreachable!("owned managed-agent preflight returned without a pid");
        };
        *self
            .pending_stop_pid
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);
        #[cfg(unix)]
        {
            let signaled = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            if signaled != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                return Err(AgentError::new(
                    "agent-stop-failed",
                    "Failed to stop the managed local agent.",
                    true,
                ));
            }
        }
        for _ in 0..50 {
            if MANAGED_AGENT_PID
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
            {
                self.clear_connection_failure();
                *self
                    .pending_stop_pid
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(AgentError::new(
            "agent-stop-failed",
            "The managed local agent did not stop in time.",
            true,
        ))
    }

    fn require_owned_managed_agent(&self) -> Result<(), AgentError> {
        let owned = *MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(owned) = owned else {
            return Err(AgentError::new(
                "external-agent",
                "The running foks-agent was not started by this FOKS. Quit this app, stop foks-agent, and relaunch to continue.",
                false,
            ));
        };
        #[cfg(unix)]
        {
            let stream =
                std::os::unix::net::UnixStream::connect(&self.socket).map_err(|error| {
                    AgentError::new(
                        "agent-lost",
                        format!("Could not verify the managed agent endpoint: {error}"),
                        true,
                    )
                })?;
            let peer = unix_peer_pid(&stream).map_err(|error| {
                AgentError::new(
                    "external-agent",
                    format!("Could not verify ownership of the local agent: {error}"),
                    false,
                )
            })?;
            if peer != owned {
                return Err(AgentError::new(
                    "external-agent",
                    "The socket is now answered by a foks-agent this FOKS did not start. Quit this app, stop foks-agent, and relaunch to continue.",
                    false,
                ));
            }
        }
        Ok(())
    }

    /// Takes ownership of a compatible agent already answering on the socket
    /// when it is this bundle's own `foks-agent`, left running by a launch of
    /// this app that did not exit cleanly. Its parent is gone — it has been
    /// reparented to init — so no other desktop supervises it, and adopting it
    /// restores what a clean launch would have: maintenance can stop it, and
    /// quitting terminates it. An agent that is another binary, or still has
    /// a live parent, stays external. Best effort: any failure to inspect the
    /// process leaves ownership as it was.
    fn adopt_orphaned_agent(&self) {
        #[cfg(unix)]
        {
            if MANAGED_AGENT_PID
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
            {
                return;
            }
            let Some(binary) = managed_agent_binary(&self.socket) else {
                return;
            };
            let Ok(Some(target)) = inspect_takeover_target(&self.socket) else {
                return;
            };
            if !same_file(&target.executable, &binary) || !process_is_orphaned(target.pid) {
                return;
            }
            adopt_pid(target.pid);
        }
    }

    /// The process answering on the socket, for Settings to describe.
    pub fn process_info(&self) -> AgentProcessInfo {
        #[cfg(unix)]
        {
            let Ok(stream) = std::os::unix::net::UnixStream::connect(&self.socket) else {
                return AgentProcessInfo::default();
            };
            let Ok(pid) = unix_peer_pid(&stream) else {
                return AgentProcessInfo::default();
            };
            drop(stream);
            let owned = *MANAGED_AGENT_PID
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                == Some(pid);
            AgentProcessInfo {
                pid: Some(pid),
                executable: process_executable_path(pid)
                    .ok()
                    .map(|path| path.display().to_string()),
                started_at: process_start_time(pid).ok(),
                owned,
            }
        }
        #[cfg(not(unix))]
        {
            AgentProcessInfo::default()
        }
    }

    pub fn stale_agent_processes(&self) -> Vec<StaleAgentProcess> {
        #[cfg(unix)]
        {
            self.socket
                .parent()
                .and_then(|state_dir| matching_agent_processes(state_dir).ok())
                .unwrap_or_default()
        }
        #[cfg(not(unix))]
        {
            Vec::new()
        }
    }

    /// Only offer extra processes when the current socket owner can be identified.
    pub fn additional_agent_processes(&self) -> Vec<StaleAgentProcess> {
        let candidates = self.stale_agent_processes();
        let Some(active) = self.process_info().pid else {
            return Vec::new();
        };
        candidates
            .into_iter()
            .filter(|target| target.pid != active)
            .collect()
    }

    /// Recheck the socket after the dialog: an extra process may now be active.
    pub fn terminate_additional_agent(&self, target: &StaleAgentProcess) -> Result<(), AgentError> {
        match self.process_info().pid {
            Some(pid) if pid != target.pid => self.terminate_stale_agent(target),
            _ => Err(AgentError::new(
                "agent-cleanup-changed",
                "The active agent changed or could not be verified. This process was not terminated.",
                false,
            )),
        }
    }

    pub fn terminate_stale_agent(&self, target: &StaleAgentProcess) -> Result<(), AgentError> {
        #[cfg(unix)]
        {
            if !agent_process_matches(target) {
                return Err(AgentError::new(
                    "agent-cleanup-changed",
                    "The agent process changed after confirmation and was not terminated.",
                    false,
                ));
            }
            let result = unsafe { libc::kill(target.pid as i32, libc::SIGTERM) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                return Err(AgentError::new(
                    "agent-cleanup-failed",
                    format!(
                        "Failed to terminate foks-agent process {}: {}",
                        target.pid,
                        std::io::Error::last_os_error()
                    ),
                    false,
                ));
            }
        }
        #[cfg(not(unix))]
        let _ = target;
        Ok(())
    }

    /// Takes ownership of a foks-agent this app did not start, on the reader's
    /// say-so, so that a restart can stop it. Refused when the socket's owner
    /// is not a foks-agent at all.
    fn claim_external_agent(&self) -> Result<(), AgentError> {
        #[cfg(unix)]
        {
            if MANAGED_AGENT_PID
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
            {
                return Ok(());
            }
            let target = inspect_takeover_target(&self.socket)
                .map_err(|error| {
                    AgentError::new(
                        "external-agent",
                        if error.fatal {
                            "The process on the agent socket is not a foks-agent, so FOKS cannot stop it.".to_owned()
                        } else {
                            error.message
                        },
                        false,
                    )
                })?
                .ok_or_else(|| {
                    AgentError::new(
                        "agent-lost",
                        "No agent is answering on the socket.",
                        true,
                    )
                })?;
            adopt_pid(target.pid);
        }
        Ok(())
    }

    /// Stops the agent and starts it again. With `takeover`, an agent this app
    /// did not start is claimed first; without it, such an agent is refused as
    /// every other maintenance refuses it.
    pub fn restart_agent(
        &self,
        takeover: bool,
        notify: &dyn Fn(&MaintenanceSnapshot),
    ) -> Result<MaintenanceSnapshot, AgentError> {
        if takeover {
            self.claim_external_agent()?;
        }
        self.run_maintenance(MaintenanceKind::Restart, notify, |worker| {
            match worker.stop_owned_agent() {
                Ok(()) => MaintenanceCompletion::Continue,
                Err(error) => MaintenanceCompletion::Failed {
                    error,
                    affected_roots: Vec::new(),
                },
            }
        })
    }

    fn require_maintenance_stop_settled(&self) -> Result<(), AgentError> {
        let pending = *self
            .pending_stop_pid
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(pid) = pending else {
            return Ok(());
        };
        let live = *MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        #[cfg(unix)]
        let exited = process_has_exited(pid);
        #[cfg(not(unix))]
        let exited = live != Some(pid);
        if live != Some(pid) || exited {
            *self
                .pending_stop_pid
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            return Ok(());
        }
        Err(AgentError::new(
            "agent-stop-pending",
            "The previously managed local agent has not finished stopping.",
            true,
        ))
    }

    /// Registers a callback invoked upon the initial recording of a connection loss.
    /// The stored error flag remains the authoritative source of truth; dropped
    /// or duplicate notifications result only in a delayed or redundant status query.
    pub fn set_connection_loss_notifier<F>(&self, notify: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let _ = self
            .transport
            .connection_loss_notifier
            .set(Arc::new(notify));
    }

    pub fn take_connection_failure(&self) -> Option<String> {
        self.connection_failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn clear_connection_failure(&self) {
        *self
            .connection_failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

enum SafeRootDisposition {
    Current,
    Selected(PathBuf),
    Recovery(PathBuf),
    CredentialsRequired(PathBuf),
}

fn safe_selected_root(source: &Path, affected_roots: &[PathBuf]) -> SafeRootDisposition {
    safe_selected_root_with(
        source,
        affected_roots,
        foks_client_app::portability::selected_desktop_state_root(),
    )
}

fn safe_selected_root_with(
    source: &Path,
    affected_roots: &[PathBuf],
    selected: foks_client_app::Result<PathBuf>,
) -> SafeRootDisposition {
    let selected = match selected {
        Ok(root) => root,
        Err(_) => return SafeRootDisposition::Recovery(source.to_path_buf()),
    };
    let mut roots = vec![source.to_path_buf(), selected.clone()];
    roots.extend_from_slice(affected_roots);
    roots.sort();
    roots.dedup();
    for root in roots {
        match foks_client_app::portability::maintenance_readiness(&root) {
            Ok(foks_client_app::portability::MaintenanceReadiness::RecoveryRequired) => {
                return SafeRootDisposition::Recovery(root);
            }
            Err(foks_client_app::Error::Keystore(foks_keystore::Error::CredentialsRequired)) => {
                return SafeRootDisposition::CredentialsRequired(root);
            }
            Err(_) => return SafeRootDisposition::Recovery(root),
            Ok(foks_client_app::portability::MaintenanceReadiness::Openable) => {}
        }
    }
    if selected == source {
        SafeRootDisposition::Current
    } else {
        SafeRootDisposition::Selected(selected)
    }
}

fn managed_agent_binary(socket: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(AGENT_BINARY_ENV) {
        return Some(PathBuf::from(path));
    }
    if default_socket().as_deref() != Some(socket) {
        return None;
    }
    let executable = std::env::current_exe().ok()?;
    let candidate = packaged_agent_candidate(&executable)?;
    candidate.exists().then_some(candidate)
}

fn packaged_agent_candidate(executable: &Path) -> Option<PathBuf> {
    let candidate = executable.parent()?.join("foks-agent");
    Some(candidate)
}

/// Exercise the packaged binary's real discovery and launch path without a GUI.
/// Uses fresh private state and never connects to the user's existing agent.
pub fn smoke_test_packaged_startup() -> Result<(), String> {
    if tauri::is_dev() {
        return Err("expected a production build with embedded frontend assets".into());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let binary = packaged_agent_candidate(&executable)
        .ok_or("cannot locate packaged agent beside the desktop executable")?;
    let state = tempfile::Builder::new()
        .prefix("foks-smoke-")
        .tempdir_in("/tmp")
        .map_err(|error| error.to_string())?;
    let socket = state.path().join("agent.sock");
    prepare_state_directory(state.path()).map_err(|error| error.message)?;
    // The same launch routine validates ownership/permissions and passes state/socket.
    let mut launch = launch_agent(&binary, state.path(), &socket).map_err(|error| error.message)?;
    let handle = AgentHandle::new(socket);
    let result = (|| {
        let mut last_error = "agent did not become ready".to_owned();
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            match handle.call_blocking(Operation::AgentStatus) {
                Ok(response) => return success_value(response).map(|_| ()).map_err(|e| e.message),
                Err(error) => last_error = error.message,
            }
            if let Some(error) = launch.early_exit_error() {
                return Err(error.message);
            }
        }
        Err(last_error)
    })();
    // Leave the PID registered until the reaper confirms exit, so temporary
    // state is not removed while the child is still using it.
    {
        let pid = MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(pid) = *pid {
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
    }
    for _ in 0..50 {
        if MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if MANAGED_AGENT_PID
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some()
    {
        terminate_managed_agent();
        return Err("packaged agent did not exit after SIGTERM".into());
    }
    if result.is_err() {
        if let Ok(log) = std::fs::read_to_string(state.path().join("agent.log")) {
            eprintln!("{log}");
        }
    }
    result
}

fn acquire_spawn_lock(socket: &Path) -> Result<File, AgentError> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

    let path = socket.with_extension("desktop-agent.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|error| {
            AgentError::new(
                "agent-lock",
                format!("Failed to open {}: {error}", path.display()),
                true,
            )
        })?;
    let metadata = file
        .metadata()
        .map_err(|error| AgentError::new("agent-lock", error.to_string(), false))?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file() || metadata.uid() != uid || metadata.permissions().mode() & 0o077 != 0 {
        return Err(AgentError::new(
            "agent-lock",
            "Desktop agent lock file must be a regular file owned by the current user with private permissions.",
            false,
        ));
    }
    file.lock_exclusive().map_err(|error| {
        AgentError::new(
            "agent-lock",
            format!("Failed to lock {}: {error}", path.display()),
            true,
        )
    })?;
    Ok(file)
}

fn validate_agent_binary(binary: &Path) -> Result<(), AgentError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = std::fs::symlink_metadata(binary).map_err(|error| {
        AgentError::new(
            "agent-binary",
            format!(
                "The packaged local agent at {} is unavailable: {error}",
                binary.display()
            ),
            false,
        )
    })?;
    let desktop = std::fs::metadata(
        std::env::current_exe()
            .map_err(|error| AgentError::new("agent-binary", error.to_string(), false))?,
    )
    .map_err(|error| AgentError::new("agent-binary", error.to_string(), false))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != desktop.uid()
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(AgentError::new("agent-binary", "Agent binary must be an executable regular file owned by the same user as the desktop application.", false));
    }
    Ok(())
}

fn prepare_state_directory(directory: &Path) -> Result<(), AgentError> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
    if !directory.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(|error| AgentError::new("agent-state", error.to_string(), false))?;
    }
    let metadata = std::fs::symlink_metadata(directory)
        .map_err(|error| AgentError::new("agent-state", error.to_string(), false))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(AgentError::new(
            "agent-state",
            "The FOKS state path must be a directory owned by the current user.",
            false,
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| AgentError::new("agent-state", error.to_string(), false))?;
    }
    Ok(())
}

pub(crate) fn prepare_managed_crash_directory(directory: &Path) -> Result<(), AgentError> {
    let state = directory.parent().ok_or_else(|| {
        AgentError::new(
            "crash-state",
            "Invalid crash report path: missing parent directory.",
            false,
        )
    })?;
    // Crash reporting is filesystem-only startup infrastructure. Reserve the
    // path against relocation without parsing its credential envelope or
    // accessing native credential storage before Tauri can display an agent
    // startup error.
    let _path_lease = foks_client_app::portability::ClientStatePathLease::acquire(state)
        .map_err(|error| AgentError::new("crash-state", error.to_string(), false))?;
    prepare_state_directory(state)?;
    prepare_state_directory(directory).map_err(|error| AgentError {
        code: "crash-state".to_owned(),
        ..error
    })
}

fn managed_agent_arguments(state_dir: &Path, socket: &Path) -> [std::ffi::OsString; 6] {
    [
        "--state-dir".into(),
        state_dir.as_os_str().to_owned(),
        "--socket".into(),
        socket.as_os_str().to_owned(),
        // The resident agent must allow as long as the desktop client waits.
        // Accessing a Go CLI credential can raise a blocking macOS Keychain
        // prompt, so the default 15 seconds is too short for that path.
        "--request-timeout-seconds".into(),
        "60".into(),
    ]
}

const MAX_AGENT_STARTUP_DIAGNOSTIC_BYTES: u64 = 4096;

struct ManagedAgentLaunch {
    pid: u32,
    exit: mpsc::Receiver<std::io::Result<ExitStatus>>,
    log: File,
    log_offset: u64,
}

impl ManagedAgentLaunch {
    fn early_exit_error(&mut self) -> Option<AgentError> {
        match self.exit.try_recv() {
            Ok(Ok(status)) => {
                let mut message = format!(
                    "The local background service process {} exited before FOKS could connect to its socket ({status}).",
                    self.pid
                );
                if let Some(diagnostic) = self.diagnostic() {
                    message.push_str("\n\nAgent output:\n");
                    message.push_str(&diagnostic);
                }
                Some(AgentError::new("agent-start-failed", message, true))
            }
            Ok(Err(error)) => Some(AgentError::new(
                "agent-start-failed",
                format!(
                    "Failed to supervise local background service process {}: {error}",
                    self.pid
                ),
                true,
            )),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(AgentError::new(
                "agent-start-failed",
                format!(
                    "Lost supervision of local background service process {} before FOKS could connect to its socket.",
                    self.pid
                ),
                true,
            )),
        }
    }

    fn diagnostic(&mut self) -> Option<String> {
        use std::io::{Read as _, Seek as _, SeekFrom};

        self.log.seek(SeekFrom::Start(self.log_offset)).ok()?;
        let mut bytes = Vec::new();
        self.log
            .by_ref()
            .take(MAX_AGENT_STARTUP_DIAGNOSTIC_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        let diagnostic = String::from_utf8_lossy(&bytes).trim().to_owned();
        (!diagnostic.is_empty()).then_some(diagnostic)
    }
}

fn launch_agent(
    binary: &Path,
    state_dir: &Path,
    socket: &Path,
) -> Result<ManagedAgentLaunch, AgentError> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
    validate_agent_binary(binary)?;
    let log = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(state_dir.join("agent.log"))
        .map_err(|error| {
            AgentError::new(
                "agent-log",
                format!("Failed to open agent log: {error}"),
                false,
            )
        })?;
    let log_metadata = log
        .metadata()
        .map_err(|error| AgentError::new("agent-log", error.to_string(), false))?;
    if !log_metadata.is_file() || log_metadata.uid() != unsafe { libc::geteuid() } {
        return Err(AgentError::new(
            "agent-log",
            "The agent log must be a regular file owned by the current user.",
            false,
        ));
    }
    log.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| AgentError::new("agent-log", error.to_string(), false))?;
    let diagnostic_log = log
        .try_clone()
        .map_err(|error| AgentError::new("agent-log", error.to_string(), false))?;
    let stderr = log
        .try_clone()
        .map_err(|error| AgentError::new("agent-log", error.to_string(), false))?;
    let mut child = Command::new(binary)
        .args(managed_agent_arguments(state_dir, socket))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| {
            AgentError::new(
                "agent-start-failed",
                format!("Failed to launch local agent: {error}"),
                true,
            )
        })?;
    let pid = child.id();
    *MANAGED_AGENT_PID
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);
    let (exit_sender, exit) = mpsc::channel();
    std::thread::Builder::new()
        .name("foks-agent-reaper".to_owned())
        .spawn(move || {
            let status = child.wait();
            let mut guard = MANAGED_AGENT_PID
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *guard == Some(pid) {
                *guard = None;
            }
            let _ = exit_sender.send(status);
        })
        .map_err(|error| {
            AgentError::new(
                "agent-start-failed",
                format!("Failed to supervise local agent: {error}"),
                false,
            )
        })?;
    Ok(ManagedAgentLaunch {
        pid,
        exit,
        log: diagnostic_log,
        log_offset: log_metadata.len(),
    })
}

static MANAGED_AGENT_PID: Mutex<Option<u32>> = Mutex::new(None);

/// Records `pid` as the managed agent and watches for its exit. The process
/// is not this one's child, so it cannot be waited on: a thread polls it and
/// clears the record when it is gone.
#[cfg(unix)]
fn adopt_pid(pid: u32) {
    {
        let mut guard = MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_some() {
            return;
        }
        *guard = Some(pid);
    }
    let _ = std::thread::Builder::new()
        .name("foks-agent-adopted-reaper".to_owned())
        .spawn(move || {
            while !process_has_exited(pid) {
                std::thread::sleep(Duration::from_millis(500));
            }
            let mut guard = MANAGED_AGENT_PID
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *guard == Some(pid) {
                *guard = None;
            }
        });
}

pub fn terminate_managed_agent() {
    if let Some(pid) = MANAGED_AGENT_PID
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
}

#[cfg(unix)]
fn inspect_takeover_target(socket: &Path) -> Result<Option<AgentTakeover>, AgentError> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
    use std::os::unix::net::UnixStream;

    let metadata = match std::fs::symlink_metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AgentError::new(
                "version-mismatch",
                format!("Failed to inspect the incompatible agent socket: {error}"),
                false,
            ));
        }
    };
    if !metadata.file_type().is_socket()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(AgentError::new(
            "unsafe-socket",
            "The agent socket is not private to this user account and cannot be used.",
            false,
        ));
    }
    let stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Err(AgentError::new(
                "version-mismatch",
                format!("Failed to reconnect to the incompatible agent: {error}"),
                true,
            ));
        }
    };
    let pid = unix_peer_pid(&stream).map_err(|error| {
        AgentError::new(
            "version-mismatch",
            format!("Failed to identify the incompatible agent process: {error}"),
            false,
        )
    })?;
    drop(stream);
    if !is_replaceable_agent_process(pid) {
        let mut error = AgentError::new(
            "version-mismatch",
            "Desktop and agent protocol versions do not match, and the running process is not a replaceable foks-agent.",
            false,
        );
        error.fatal = true;
        return Err(error);
    }
    let executable = process_executable_path(pid).map_err(|error| {
        AgentError::new(
            "agent-takeover",
            format!("Failed to inspect the agent executable: {error}"),
            false,
        )
    })?;
    // Connection and pathname inspection must describe the same socket.
    let current = match std::fs::symlink_metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AgentError::new("agent-takeover", error.to_string(), true)),
    };
    if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Err(AgentError::new(
            "agent-takeover-changed",
            "The socket changed repeatedly while its owner was being identified. Startup could not safely take over.",
            true,
        ));
    }
    Ok(Some(AgentTakeover {
        socket: socket.to_path_buf(),
        pid,
        executable,
        device: metadata.dev(),
        inode: metadata.ino(),
    }))
}

#[cfg(unix)]
fn stop_incompatible_agent(
    socket: &Path,
    confirm: &dyn Fn(&AgentTakeover) -> Result<bool, AgentError>,
    approvals: &mut usize,
) -> Result<(), AgentError> {
    let Some(target) = inspect_takeover_target(socket)? else {
        return Ok(());
    };
    if *approvals >= MAX_STARTUP_TAKEOVERS {
        return Err(AgentError::new(
            "agent-takeover-limit",
            "Startup stopped because another process repeatedly claimed the FOKS socket; no further process was terminated.",
            false,
        ));
    }
    *approvals += 1;
    if !confirm(&target)? {
        return Err(AgentError::new(
            "agent-takeover-declined",
            "Agent takeover was cancelled. The existing process was left running.",
            false,
        ));
    }
    // Reinspect the endpoint after confirmation because it may have changed while
    // the dialog was open. Approval applies only to the endpoint originally shown.
    if inspect_takeover_target(socket)?.as_ref() != Some(&target) {
        // Let the caller check protocol compatibility before considering
        // another takeover. A compatible successor needs no termination.
        return Ok(());
    }
    terminate_takeover_target(&target)
}

#[cfg(unix)]
fn terminate_takeover_target(target: &AgentTakeover) -> Result<(), AgentError> {
    let pid = target.pid;
    let socket = &target.socket;
    {
        let mut guard = MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *guard == Some(pid) {
            *guard = None;
        }
    }
    let signaled = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if signaled != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(AgentError::new(
            "version-mismatch",
            format!("Failed to stop the incompatible agent: {error}"),
            true,
        ));
    }
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if incompatible_listener_is_gone(socket, pid) {
            return Ok(());
        }
    }
    Err(AgentError::new(
        "version-mismatch",
        "The incompatible local agent did not exit after SIGTERM.",
        true,
    ))
}

#[cfg(unix)]
fn matching_agent_processes(state_dir: &Path) -> std::io::Result<Vec<StaleAgentProcess>> {
    let state_dir = state_dir.canonicalize()?;
    let mut processes = process_ids()?
        .into_iter()
        .filter(|pid| *pid > 1 && *pid != std::process::id())
        .filter_map(|pid| {
            if process_owner_uid(pid).ok()? != unsafe { libc::geteuid() }
                || !is_foks_agent_executable(&process_executable_path(pid).ok()?)
            {
                return None;
            }
            let configured = agent_state_dir(&process_arguments(pid).ok()?)?;
            let configured = configured.canonicalize().ok()?;
            if !same_file(&configured, &state_dir) {
                return None;
            }
            Some(StaleAgentProcess {
                pid,
                executable: process_executable_path(pid).ok()?,
                started_at: process_start_time(pid).ok()?,
                state_dir: configured,
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by_key(|process| process.pid);
    Ok(processes)
}

#[cfg(unix)]
fn agent_process_matches(target: &StaleAgentProcess) -> bool {
    process_owner_uid(target.pid).ok() == Some(unsafe { libc::geteuid() })
        && process_start_time(target.pid).ok() == Some(target.started_at)
        && process_executable_path(target.pid).ok().as_ref() == Some(&target.executable)
        && process_arguments(target.pid)
            .ok()
            .and_then(|arguments| agent_state_dir(&arguments))
            .and_then(|state_dir| state_dir.canonicalize().ok())
            .is_some_and(|state_dir| same_file(&state_dir, &target.state_dir))
}

#[cfg(unix)]
fn agent_state_dir(arguments: &[OsString]) -> Option<PathBuf> {
    arguments.iter().enumerate().find_map(|(index, argument)| {
        if argument == "--state-dir" {
            return arguments.get(index + 1).map(PathBuf::from);
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
            argument
                .as_os_str()
                .as_bytes()
                .strip_prefix(b"--state-dir=")
                .map(|path| PathBuf::from(OsString::from_vec(path.to_vec())))
        }
    })
}

#[cfg(target_os = "macos")]
fn process_ids() -> std::io::Result<Vec<u32>> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut pids = vec![0 as libc::pid_t; count as usize + 64];
    let listed = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(pids.as_slice()) as libc::c_int,
        )
    };
    if listed < 0 {
        return Err(std::io::Error::last_os_error());
    }
    pids.truncate(listed as usize);
    Ok(pids
        .into_iter()
        .filter_map(|pid| u32::try_from(pid).ok())
        .collect())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn process_ids() -> std::io::Result<Vec<u32>> {
    Ok(std::fs::read_dir("/proc")?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn process_ids() -> std::io::Result<Vec<u32>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process enumeration is unavailable on this platform",
    ))
}

#[cfg(target_os = "macos")]
fn process_owner_uid(pid: u32) -> std::io::Result<libc::uid_t> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if written != size {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info.pbi_uid)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn process_owner_uid(pid: u32) -> std::io::Result<libc::uid_t> {
    use std::os::unix::fs::MetadataExt as _;
    Ok(std::fs::metadata(format!("/proc/{pid}"))?.uid())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn process_owner_uid(_pid: u32) -> std::io::Result<libc::uid_t> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process ownership is unavailable on this platform",
    ))
}

#[cfg(target_os = "macos")]
fn process_arguments(pid: u32) -> std::io::Result<Vec<OsString>> {
    use std::os::unix::ffi::OsStringExt as _;

    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut size = 0;
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let mut bytes = vec![0u8; size];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    bytes.truncate(size);
    if bytes.len() < std::mem::size_of::<libc::c_int>() {
        return Err(std::io::Error::other("malformed process arguments"));
    }
    let argc = libc::c_int::from_ne_bytes(
        bytes[..std::mem::size_of::<libc::c_int>()]
            .try_into()
            .map_err(|_| std::io::Error::other("malformed process arguments"))?,
    );
    let mut offset = std::mem::size_of::<libc::c_int>();
    offset += bytes[offset..]
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| std::io::Error::other("malformed process arguments"))?;
    while bytes.get(offset) == Some(&0) {
        offset += 1;
    }
    Ok(bytes[offset..]
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .take(argc.max(0) as usize)
        .map(|argument| OsString::from_vec(argument.to_vec()))
        .collect())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn process_arguments(pid: u32) -> std::io::Result<Vec<OsString>> {
    use std::os::unix::ffi::OsStringExt as _;

    Ok(std::fs::read(format!("/proc/{pid}/cmdline"))?
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| OsString::from_vec(argument.to_vec()))
        .collect())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn process_arguments(_pid: u32) -> std::io::Result<Vec<OsString>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process arguments are unavailable on this platform",
    ))
}

#[cfg(unix)]
fn process_has_exited(pid: u32) -> bool {
    let alive = unsafe { libc::kill(pid as i32, 0) };
    alive != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Whether two paths name the same file, by device and inode, so a bundle
/// reached through a symlink or a different prefix still matches its agent.
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Whether `pid` has been reparented to init: the process that launched it
/// has exited, so nothing else supervises it.
#[cfg(unix)]
fn process_is_orphaned(pid: u32) -> bool {
    process_parent_pid(pid).is_ok_and(|parent| parent == 1)
}

/// When `pid` started, in seconds since the Unix epoch.
#[cfg(unix)]
fn process_start_time(pid: u32) -> std::io::Result<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let written = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                size,
            )
        };
        if written != size {
            return Err(std::io::Error::last_os_error());
        }
        Ok(info.pbi_start_tvsec)
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // Field 22 of `/proc/<pid>/stat` is the start time in clock ticks
        // since boot; `/proc/stat`'s `btime` is the boot time.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let rest = stat
            .rsplit_once(')')
            .map(|(_, rest)| rest)
            .ok_or_else(|| std::io::Error::other("malformed /proc stat"))?;
        let ticks: u64 = rest
            .split_whitespace()
            .nth(19)
            .and_then(|field| field.parse().ok())
            .ok_or_else(|| std::io::Error::other("malformed /proc stat"))?;
        let boot: u64 = std::fs::read_to_string("/proc/stat")?
            .lines()
            .find_map(|line| line.strip_prefix("btime "))
            .and_then(|value| value.trim().parse().ok())
            .ok_or_else(|| std::io::Error::other("no btime in /proc/stat"))?;
        let hertz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hertz <= 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(boot + ticks / hertz as u64)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
    {
        let _ = pid;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "agent start time is unavailable on this platform",
        ))
    }
}

#[cfg(unix)]
fn process_parent_pid(pid: u32) -> std::io::Result<u32> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let written = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                size,
            )
        };
        if written != size {
            return Err(std::io::Error::last_os_error());
        }
        Ok(info.pbi_ppid)
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // `/proc/<pid>/stat`: the parent pid is the field after the
        // parenthesized command name, which may itself contain spaces.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let rest = stat
            .rsplit_once(')')
            .map(|(_, rest)| rest)
            .ok_or_else(|| std::io::Error::other("malformed /proc stat"))?;
        rest.split_whitespace()
            .nth(1)
            .and_then(|field| field.parse().ok())
            .ok_or_else(|| std::io::Error::other("malformed /proc stat"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
    {
        let _ = pid;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "agent parent pid is unavailable on this platform",
        ))
    }
}

#[cfg(unix)]
fn incompatible_listener_is_gone(socket: &Path, pid: u32) -> bool {
    use std::os::unix::net::UnixStream;

    if process_has_exited(pid) {
        return true;
    }
    match UnixStream::connect(socket) {
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            true
        }
        Ok(stream) => unix_peer_pid(&stream).ok() != Some(pid),
        Err(_) => false,
    }
}

#[cfg(unix)]
fn unix_peer_pid(stream: &std::os::unix::net::UnixStream) -> std::io::Result<u32> {
    use std::os::unix::io::AsRawFd;

    #[cfg(target_os = "macos")]
    let mut pid: libc::pid_t = 0;
    #[cfg(not(target_os = "macos"))]
    let pid: libc::pid_t;
    #[cfg(target_os = "macos")]
    {
        let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&raw mut pid).cast(),
                &mut length,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut credential = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&raw mut credential).cast(),
                &mut length,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error());
        }
        pid = credential.pid;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
    {
        let _ = stream;
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "agent peer pid is unavailable on this platform",
        ));
    }
    if pid <= 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "agent peer pid is invalid",
        ));
    }
    Ok(pid as u32)
}

#[cfg(unix)]
fn process_executable_path(pid: u32) -> std::io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStringExt as _;
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let length = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
            )
        };
        if length <= 0 {
            return Err(std::io::Error::last_os_error());
        }
        buffer.truncate(length as usize);
        Ok(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        std::fs::read_link(format!("/proc/{pid}/exe"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
    {
        let _ = pid;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "agent process path is unavailable on this platform",
        ))
    }
}

#[cfg(unix)]
fn is_foks_agent_executable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "foks-agent" || name.starts_with("foks-agent (deleted)")
}

#[cfg(unix)]
fn is_replaceable_agent_process(pid: u32) -> bool {
    if pid <= 1 || pid == std::process::id() {
        return false;
    }
    process_executable_path(pid).is_ok_and(|path| is_foks_agent_executable(&path))
}

pub fn success_value(response: Response) -> Result<Value, AgentError> {
    match response.result {
        ResponseResult::Success { value } => Ok(value),
        ResponseResult::Error {
            code,
            message,
            fields,
        } => {
            let mut error = AgentError::from_agent(code, message);
            error.details = error_details(fields).map(Box::new);
            Err(error)
        }
    }
}

pub fn resolve_endpoint<I, S>(arguments: I, environment: Option<PathBuf>) -> Option<AgentEndpoint>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if let Some(socket) = socket_from_arguments(arguments) {
        return Some(AgentEndpoint {
            socket,
            managed_crash_directory: None,
        });
    }
    if let Some(socket) = environment.filter(|socket| !socket.as_os_str().is_empty()) {
        return Some(AgentEndpoint {
            socket,
            managed_crash_directory: None,
        });
    }
    let state = default_state_directory()?;
    Some(AgentEndpoint {
        socket: state.join(DEFAULT_SOCKET_NAME),
        managed_crash_directory: Some(state.join("crashes")),
    })
}

fn socket_from_arguments<I, S>(arguments: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let argument = argument.as_ref();
        if argument == OsStr::new(SOCKET_ARG) {
            if let Some(value) = arguments.next() {
                if !value.as_ref().is_empty() {
                    return Some(PathBuf::from(value.as_ref()));
                }
            }
        } else if let Some(value) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix("--agent-socket="))
        {
            if !value.is_empty() {
                return Some(PathBuf::from(value));
            }
        }
    }
    None
}

fn default_state_directory() -> Option<PathBuf> {
    foks_client_app::portability::selected_desktop_state_root().ok()
}

pub fn default_socket() -> Option<PathBuf> {
    Some(default_state_directory()?.join(DEFAULT_SOCKET_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    struct FakeMaintenanceProcess {
        stop_calls: AtomicUsize,
        restore_calls: AtomicUsize,
        stop_error: Option<AgentError>,
        restore_error: Option<AgentError>,
    }

    impl FakeMaintenanceProcess {
        fn healthy() -> Arc<Self> {
            Arc::new(Self {
                stop_calls: AtomicUsize::new(0),
                restore_calls: AtomicUsize::new(0),
                stop_error: None,
                restore_error: None,
            })
        }
    }

    impl MaintenanceProcess for FakeMaintenanceProcess {
        fn preflight(&self, _handle: &AgentHandle) -> Result<(), AgentError> {
            Ok(())
        }

        fn stop(&self, _handle: &AgentHandle) -> Result<(), AgentError> {
            self.stop_calls.fetch_add(1, Ordering::AcqRel);
            self.stop_error.clone().map_or(Ok(()), Err)
        }

        fn restore(&self, _handle: &AgentHandle) -> Result<Response, AgentError> {
            self.restore_calls.fetch_add(1, Ordering::AcqRel);
            match &self.restore_error {
                Some(error) => Err(error.clone()),
                None => Ok(Response::success(
                    0,
                    serde_json::to_value(foks_agent_proto::AgentStatus::ready()).unwrap(),
                )),
            }
        }
    }

    #[test]
    fn maintenance_worker_owns_exclusion_and_restores_after_completion() {
        let dir = tempfile::tempdir().unwrap();
        let process = FakeMaintenanceProcess::healthy();
        let handle = AgentHandle::new_for_maintenance_test(
            dir.path().join(DEFAULT_SOCKET_NAME),
            process.clone(),
            |_, _| SafeRootDisposition::Current,
        );
        let observed = Mutex::new(Vec::new());
        let snapshot = handle
            .run_maintenance(
                MaintenanceKind::Verify,
                &|snapshot| observed.lock().unwrap().push(snapshot.clone()),
                |worker| {
                    let blocked = handle.transport().call(Operation::AgentStatus).unwrap_err();
                    assert_eq!(
                        blocked,
                        DesktopAgentError::Local(foks_desktop::LocalAgentCondition::Maintenance)
                    );
                    worker.stop_owned_agent().unwrap();
                    assert!(handle
                        .run_maintenance(MaintenanceKind::Export, &|_| {}, |_| {
                            MaintenanceCompletion::Continue
                        })
                        .is_err());
                    MaintenanceCompletion::Continue
                },
            )
            .unwrap();
        assert_eq!(process.stop_calls.load(Ordering::Acquire), 1);
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
        assert!(matches!(
            snapshot,
            MaintenanceSnapshot::Complete {
                operation: MaintenanceOperationOutcome::Completed,
                disposition: MaintenanceDisposition::ContinueCurrentRoot,
                ..
            }
        ));
        let phases: Vec<_> = observed
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|snapshot| match snapshot {
                MaintenanceSnapshot::Active { phase, .. } => Some(phase),
                MaintenanceSnapshot::Idle { .. } | MaintenanceSnapshot::Complete { .. } => None,
            })
            .collect();
        assert_eq!(
            phases,
            vec![
                Some(MaintenancePhase::Selecting),
                Some(MaintenancePhase::Quiescing),
                Some(MaintenancePhase::Running),
                Some(MaintenancePhase::Restoring),
                None,
            ]
        );
    }

    #[test]
    fn stop_uncertainty_still_runs_explicit_restoration_and_preserves_both_errors() {
        let dir = tempfile::tempdir().unwrap();
        let stop_error = AgentError::new("agent-stop-failed", "exit timed out", true);
        let restore_error = AgentError::new("agent-start-failed", "spawn failed", true);
        let process = Arc::new(FakeMaintenanceProcess {
            stop_calls: AtomicUsize::new(0),
            restore_calls: AtomicUsize::new(0),
            stop_error: Some(stop_error.clone()),
            restore_error: Some(restore_error.clone()),
        });
        let handle = AgentHandle::new_for_maintenance_test(
            dir.path().join(DEFAULT_SOCKET_NAME),
            process.clone(),
            |_, _| SafeRootDisposition::Current,
        );
        let snapshot = handle
            .run_maintenance(MaintenanceKind::Export, &|_| {}, |worker| {
                let error = worker.stop_owned_agent().unwrap_err();
                MaintenanceCompletion::Failed {
                    error,
                    affected_roots: Vec::new(),
                }
            })
            .unwrap();
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
        assert!(matches!(
            &snapshot,
            MaintenanceSnapshot::Complete {
                operation: MaintenanceOperationOutcome::Failed { error },
                disposition: MaintenanceDisposition::RestorationFailed { error: restoration, .. },
                ..
            } if error == &stop_error && restoration == &restore_error
        ));
        assert_eq!(
            handle.transport().call(Operation::AgentStatus).unwrap_err(),
            DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RestorationFailed)
        );
        let failed_revision = match &snapshot {
            MaintenanceSnapshot::Complete { revision, .. } => *revision,
            _ => unreachable!(),
        };
        handle.record_restoration_success(&|_| {});
        assert!(matches!(
            handle.maintenance_snapshot(),
            MaintenanceSnapshot::Complete {
                revision,
                disposition: MaintenanceDisposition::ContinueCurrentRoot,
                ..
            } if revision > failed_revision
        ));
    }

    #[test]
    fn restoration_retry_keeps_typed_gate_until_fake_process_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let process = FakeMaintenanceProcess::healthy();
        let handle = AgentHandle::new_for_maintenance_test(
            dir.path().join(DEFAULT_SOCKET_NAME),
            process.clone(),
            |_, _| SafeRootDisposition::Current,
        );
        *handle.transport.disposition.lock().unwrap() = TransportDisposition::RestorationFailed;
        assert_eq!(
            handle.transport().call(Operation::AgentStatus).unwrap_err(),
            DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RestorationFailed)
        );

        let response = handle.retry_started_blocking().unwrap();
        assert!(matches!(response.result, ResponseResult::Success { .. }));
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
        assert_eq!(
            *handle.transport.disposition.lock().unwrap(),
            TransportDisposition::Current
        );
    }

    #[test]
    fn cancellation_before_stop_does_not_touch_the_process() {
        let dir = tempfile::tempdir().unwrap();
        let process = FakeMaintenanceProcess::healthy();
        let handle = AgentHandle::new_for_maintenance_test(
            dir.path().join(DEFAULT_SOCKET_NAME),
            process.clone(),
            |_, _| SafeRootDisposition::Current,
        );
        for kind in [
            MaintenanceKind::Export,
            MaintenanceKind::Import,
            MaintenanceKind::Relocate,
        ] {
            let snapshot = handle
                .run_maintenance(kind, &|_| {}, |_| MaintenanceCompletion::Cancelled)
                .unwrap();
            assert!(matches!(
                snapshot,
                MaintenanceSnapshot::Complete {
                    operation: MaintenanceOperationOutcome::Cancelled,
                    disposition: MaintenanceDisposition::ContinueCurrentRoot,
                    ..
                }
            ));
        }
        assert_eq!(process.stop_calls.load(Ordering::Acquire), 0);
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
    }

    #[test]
    fn worker_panic_after_stop_is_failed_operation_and_restores() {
        let dir = tempfile::tempdir().unwrap();
        let process = FakeMaintenanceProcess::healthy();
        let handle = AgentHandle::new_for_maintenance_test(
            dir.path().join(DEFAULT_SOCKET_NAME),
            process.clone(),
            |_, _| SafeRootDisposition::Current,
        );
        let snapshot = handle
            .run_maintenance(MaintenanceKind::Verify, &|_| {}, |worker| {
                worker.stop_owned_agent().unwrap();
                panic!("simulated worker failure");
            })
            .unwrap();
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
        assert!(matches!(
            snapshot,
            MaintenanceSnapshot::Complete {
                operation: MaintenanceOperationOutcome::Failed { ref error },
                disposition: MaintenanceDisposition::ContinueCurrentRoot,
                ..
            } if error.code == "maintenance-worker-interrupted"
        ));
    }

    #[test]
    fn recovery_and_selected_root_dispositions_never_reopen_the_old_root() {
        let dir = tempfile::tempdir().unwrap();
        let process = FakeMaintenanceProcess::healthy();
        let recovery_root = dir.path().join("recover");
        let recovery = recovery_root.clone();
        let handle = AgentHandle::new_for_maintenance_test(
            dir.path().join("current").join(DEFAULT_SOCKET_NAME),
            process.clone(),
            move |_, _| SafeRootDisposition::Recovery(recovery.clone()),
        );
        let snapshot = handle
            .run_maintenance(MaintenanceKind::Import, &|_| {}, |worker| {
                worker.stop_owned_agent().unwrap();
                MaintenanceCompletion::Failed {
                    error: AgentError::new("import-failed", "durable boundary", false),
                    affected_roots: vec![recovery_root.clone()],
                }
            })
            .unwrap();
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
        assert!(matches!(
            snapshot,
            MaintenanceSnapshot::Complete {
                operation: MaintenanceOperationOutcome::Failed { .. },
                disposition: MaintenanceDisposition::RecoveryRequired { .. },
                ..
            }
        ));
        assert_eq!(
            handle.transport().call(Operation::AgentStatus).unwrap_err(),
            DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RecoveryRequired)
        );

        let selected = dir.path().join("selected");
        let expected = selected.clone();
        let process = FakeMaintenanceProcess::healthy();
        let moved = AgentHandle::new_for_maintenance_test(
            dir.path().join("old").join(DEFAULT_SOCKET_NAME),
            process.clone(),
            move |_, _| SafeRootDisposition::Selected(expected.clone()),
        );
        let snapshot = moved
            .run_maintenance(MaintenanceKind::Relocate, &|_| {}, |worker| {
                worker.stop_owned_agent().unwrap();
                MaintenanceCompletion::RestartSelected(selected.clone())
            })
            .unwrap();
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
        assert!(matches!(
            snapshot,
            MaintenanceSnapshot::Complete {
                disposition: MaintenanceDisposition::RestartSelectedRoot { .. },
                ..
            }
        ));
        assert_eq!(
            moved.transport().call(Operation::AgentStatus).unwrap_err(),
            DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RestartRequired)
        );

        let selected = dir.path().join("import-selected-after-failure");
        let expected = selected.clone();
        let process = FakeMaintenanceProcess::healthy();
        let imported = AgentHandle::new_for_maintenance_test(
            dir.path().join("import-old").join(DEFAULT_SOCKET_NAME),
            process.clone(),
            move |_, _| SafeRootDisposition::Selected(expected.clone()),
        );
        let snapshot = imported
            .run_maintenance(MaintenanceKind::Import, &|_| {}, |worker| {
                worker.stop_owned_agent().unwrap();
                MaintenanceCompletion::Failed {
                    error: AgentError::new(
                        "import-failed",
                        "selection was durably published before cleanup failed",
                        false,
                    ),
                    affected_roots: vec![selected.clone()],
                }
            })
            .unwrap();
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
        assert!(matches!(
            snapshot,
            MaintenanceSnapshot::Complete {
                operation: MaintenanceOperationOutcome::Failed { .. },
                disposition: MaintenanceDisposition::RestartSelectedRoot { .. },
                ..
            }
        ));
    }

    #[test]
    fn maintenance_reservation_drains_preemptible_calls_and_releases_the_signal() {
        let dir = tempfile::tempdir().unwrap();
        let handle = AgentHandle::new(dir.path().join(DEFAULT_SOCKET_NAME));
        let transport = Arc::clone(&handle.transport);

        // A call that parks on the agent, as a chat inbox poll does for its
        // full 55 seconds, releases its reservation once maintenance asks.
        let started = Arc::new((Mutex::new(false), Condvar::new()));
        let poll_started = Arc::clone(&started);
        let polling = Arc::clone(&transport);
        let poll = std::thread::spawn(move || {
            let _use = polling.reserve_use().unwrap();
            let (held, ready) = &*poll_started;
            *held.lock().unwrap() = true;
            ready.notify_all();
            while !polling.preempted(true) {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!polling.preempted(false), "a mutation is never preempted");
        });
        let (held, ready) = &*started;
        let mut waiting = held.lock().unwrap();
        while !*waiting {
            waiting = ready.wait(waiting).unwrap();
        }
        drop(waiting);
        let reservation = transport
            .reserve_for_maintenance(MAINTENANCE_RESERVATION_WAIT)
            .expect("a preemptible call gives the reservation up");
        poll.join().unwrap();
        // The reservation keeps later calls out on its own, so the drain
        // signal is not left set behind it.
        assert!(!transport.maintenance_pending.load(Ordering::Acquire));
        assert_eq!(
            transport.reserve_use().unwrap_err(),
            DesktopAgentError::Local(foks_desktop::LocalAgentCondition::Maintenance)
        );
        drop(reservation);

        // A call that will not give it up costs the wait, and leaves the
        // transport usable rather than refusing every later call.
        let retained = transport.reserve_use().unwrap();
        assert!(transport
            .reserve_for_maintenance(Duration::from_millis(60))
            .is_none());
        assert!(!transport.maintenance_pending.load(Ordering::Acquire));
        drop(retained);
        assert!(transport.reserve_use().is_ok());
    }

    #[test]
    fn the_startup_gate_holds_an_ordinary_command_and_releases_it() {
        use std::sync::mpsc;

        let gate = Arc::new(StartupGate::new_open());
        gate.hold();

        let (sender, receiver) = mpsc::channel();
        let waiter = Arc::clone(&gate);
        let thread = std::thread::spawn(move || {
            waiter.wait();
            sender.send(()).expect("the receiver outlives this send");
        });

        // The command is held: nothing arrives while the gate is closed.
        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
        gate.release();
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("releasing the gate wakes the waiting command");
        thread.join().expect("the waiting thread finishes");

        // Once open it stays open, so a later command does not wait at all.
        gate.wait();
    }

    #[test]
    fn maintenance_excludes_retained_transports_and_restart_required_startup() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("absent-state");
        let handle = AgentHandle::new(root.join(DEFAULT_SOCKET_NAME));
        let transport = handle.transport();
        {
            let _maintenance = handle.transport.maintenance.write().unwrap();
            assert!(handle.call_blocking(Operation::AgentStatus).is_err());
            assert!(transport.call(Operation::AgentStatus).is_err());
            assert!(handle.ensure_started_blocking().is_err());
        }
        *handle.transport.disposition.lock().unwrap() = TransportDisposition::RestartRequired;
        assert!(transport.call(Operation::AgentStatus).is_err());
        assert!(handle.ensure_started_blocking().is_err());
        assert!(!root.exists());
    }

    #[test]
    fn external_agent_endpoint_rejects_maintenance_before_running_worker() {
        let dir = tempfile::tempdir().unwrap();
        let handle = AgentHandle::new(dir.path().join(DEFAULT_SOCKET_NAME));
        let ran = AtomicBool::new(false);
        let error = handle
            .run_maintenance(MaintenanceKind::Export, &|_| {}, |_| {
                ran.store(true, Ordering::Release);
                MaintenanceCompletion::Continue
            })
            .unwrap_err();
        assert_eq!(error.code, "external-agent");
        assert!(!ran.load(Ordering::Acquire));
        assert!(matches!(
            handle.maintenance_snapshot(),
            MaintenanceSnapshot::Idle { .. }
        ));
    }

    #[test]
    fn stale_maintenance_events_cannot_replace_a_newer_generation() {
        let dir = tempfile::tempdir().unwrap();
        let handle = AgentHandle::new(dir.path().join(DEFAULT_SOCKET_NAME));
        let observed = Mutex::new(Vec::new());
        let notify = |snapshot: &MaintenanceSnapshot| {
            observed.lock().unwrap().push(snapshot.generation());
        };
        handle.publish_maintenance(
            MaintenanceSnapshot::Active {
                generation: 7,
                revision: 0,
                kind: MaintenanceKind::Verify,
                phase: MaintenancePhase::Running,
            },
            &notify,
        );
        handle.publish_maintenance(
            MaintenanceSnapshot::Complete {
                generation: 7,
                revision: 0,
                kind: MaintenanceKind::Export,
                operation: MaintenanceOperationOutcome::Completed,
                disposition: MaintenanceDisposition::ContinueCurrentRoot,
            },
            &notify,
        );
        handle.publish_maintenance(
            MaintenanceSnapshot::Active {
                generation: 7,
                revision: 0,
                kind: MaintenanceKind::Verify,
                phase: MaintenancePhase::Restoring,
            },
            &notify,
        );
        assert_eq!(observed.into_inner().unwrap(), vec![7, 7]);
        assert_eq!(handle.maintenance_snapshot().generation(), 7);
        assert!(matches!(
            handle.maintenance_snapshot(),
            MaintenanceSnapshot::Complete { .. }
        ));
    }

    #[test]
    fn managed_endpoint_uses_rust_specific_names() {
        let state = default_state_directory().unwrap();
        #[cfg(target_os = "macos")]
        assert!(state.ends_with("Library/Application Support/foks-rs"));
        #[cfg(not(target_os = "macos"))]
        assert!(state.ends_with("foks-rs"));
        assert_eq!(default_socket(), Some(state.join(DEFAULT_SOCKET_NAME)));
    }

    #[test]
    fn socket_resolution_precedence_is_stable() {
        assert_eq!(
            resolve_endpoint(
                ["--agent-socket", "/run/argument.sock"],
                Some(PathBuf::from("/run/environment.sock"))
            )
            .map(|endpoint| endpoint.socket),
            Some(PathBuf::from("/run/argument.sock"))
        );
        assert_eq!(
            resolve_endpoint(
                ["--unrelated"],
                Some(PathBuf::from("/run/environment.sock"))
            )
            .map(|endpoint| endpoint.socket),
            Some(PathBuf::from("/run/environment.sock"))
        );
    }

    #[test]
    fn only_the_default_managed_endpoint_owns_crash_marker_storage() {
        let explicit = resolve_endpoint(
            ["--agent-socket", "/tmp/caller-owned/agent.sock"],
            Some(PathBuf::from("/tmp/environment-owned/agent.sock")),
        )
        .unwrap();
        assert_eq!(
            explicit.socket,
            PathBuf::from("/tmp/caller-owned/agent.sock")
        );
        assert_eq!(explicit.managed_crash_directory, None);

        let managed_socket = default_socket().unwrap();
        let explicit_managed_path = resolve_endpoint(
            [
                std::ffi::OsString::from(SOCKET_ARG),
                managed_socket.as_os_str().to_os_string(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(explicit_managed_path.socket, managed_socket);
        assert_eq!(explicit_managed_path.managed_crash_directory, None);

        let environment = resolve_endpoint(
            ["--unrelated"],
            Some(PathBuf::from("/tmp/environment-owned/agent.sock")),
        )
        .unwrap();
        assert_eq!(
            environment.socket,
            PathBuf::from("/tmp/environment-owned/agent.sock")
        );
        assert_eq!(environment.managed_crash_directory, None);

        let environment_managed_path =
            resolve_endpoint(std::iter::empty::<&str>(), default_socket()).unwrap();
        assert_eq!(environment_managed_path.socket, default_socket().unwrap());
        assert_eq!(environment_managed_path.managed_crash_directory, None);

        let empty_environment =
            resolve_endpoint(std::iter::empty::<&str>(), Some(PathBuf::new())).unwrap();
        assert_eq!(empty_environment.socket, default_socket().unwrap());
        assert!(empty_environment.managed_crash_directory.is_some());

        let managed = resolve_endpoint(std::iter::empty::<&str>(), None).unwrap();
        assert_eq!(managed.socket, default_socket().unwrap());
        assert_eq!(
            managed.managed_crash_directory,
            default_state_directory().map(|state| state.join("crashes"))
        );
    }

    #[test]
    fn managed_state_and_spawn_lock_are_private() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        prepare_state_directory(&state).unwrap();
        assert_eq!(
            std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(acquire_spawn_lock(&state.join("agent.sock"))
            .unwrap()
            .metadata()
            .unwrap()
            .is_file());
    }

    #[test]
    fn managed_crash_storage_prepares_private_owned_state() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        let crashes = state.join("crashes");
        prepare_managed_crash_directory(&crashes).unwrap();
        for directory in [state, crashes] {
            let metadata = std::fs::symlink_metadata(directory).unwrap();
            assert!(metadata.is_dir());
            assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
            assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn managed_crash_storage_does_not_parse_client_state() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        prepare_state_directory(&state).unwrap();
        std::fs::write(
            state.join("client-state.toml"),
            b"version = 2\nstate_id = \"legacy\"\ncredential_backend = \"native\"\n",
        )
        .unwrap();

        let crashes = state.join("crashes");
        prepare_managed_crash_directory(&crashes).unwrap();
        assert!(crashes.is_dir());
    }

    #[test]
    fn managed_launch_passes_state_socket_and_the_desktop_request_timeout() {
        let arguments = managed_agent_arguments(
            Path::new("/private/foks-state"),
            Path::new("/private/foks-state/agent.sock"),
        );
        assert_eq!(
            arguments,
            [
                std::ffi::OsString::from("--state-dir"),
                std::ffi::OsString::from("/private/foks-state"),
                std::ffi::OsString::from("--socket"),
                std::ffi::OsString::from("/private/foks-state/agent.sock"),
                std::ffi::OsString::from("--request-timeout-seconds"),
                std::ffi::OsString::from("60"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_launch_reports_only_bounded_output_from_the_failed_process() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempfile::tempdir().unwrap();
        let binary = temporary.path().join("foks-agent");
        let socket = temporary.path().join("agent.sock");
        let log = temporary.path().join("agent.log");
        std::fs::write(&log, "stale failure from an earlier launch\n").unwrap();
        let output = format!(
            "foks-agent: bind failed: Address already in use (os error 48)\n{}",
            "x".repeat(5000)
        );
        std::fs::write(
            &binary,
            format!("#!/bin/sh\nprintf '%s' '{output}' >&2\nexit 23\n"),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();

        let mut launch = launch_agent(&binary, temporary.path(), &socket).unwrap();
        let error = (0..100)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(10));
                launch.early_exit_error()
            })
            .expect("failed agent should exit promptly");
        assert_eq!(error.code, "agent-start-failed");
        assert!(error.retryable);
        assert!(!error.fatal);
        assert!(error.message.contains("exit status: 23"));
        assert!(!error.message.contains("stale failure"));
        let diagnostic = error.message.split_once("Agent output:\n").unwrap().1;
        assert!(diagnostic.starts_with("foks-agent: bind failed: Address already in use"));
        assert!(diagnostic.len() <= MAX_AGENT_STARTUP_DIAGNOSTIC_BYTES as usize);

        std::fs::write(&binary, "#!/bin/sh\nexit 7\n").unwrap();
        let mut launch = launch_agent(&binary, temporary.path(), &socket).unwrap();
        let error = (0..100)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(10));
                launch.early_exit_error()
            })
            .expect("silent failed agent should exit promptly");
        assert!(error.message.contains("exit status: 7"));
        assert!(!error.message.contains("Agent output:"));
        assert!(!error.message.contains("bind failed"));
    }

    #[test]
    fn packaged_agent_discovery_matches_the_platform_bundle_layout() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            packaged_agent_candidate(Path::new(
                "/Applications/FOKS.app/Contents/MacOS/foks-desktop"
            )),
            Some(PathBuf::from(
                "/Applications/FOKS.app/Contents/MacOS/foks-agent"
            ))
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            packaged_agent_candidate(Path::new("/usr/bin/foks-desktop")),
            Some(PathBuf::from("/usr/bin/foks-agent"))
        );
    }

    fn read_socket_request(
        stream: &mut std::os::unix::net::UnixStream,
    ) -> foks_agent_proto::Request {
        use std::io::Read as _;
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut prefix = [0; 4];
        stream.read_exact(&mut prefix).unwrap();
        let mut frame = vec![0; 4 + u32::from_be_bytes(prefix) as usize];
        frame[..4].copy_from_slice(&prefix);
        stream.read_exact(&mut frame[4..]).unwrap();
        foks_agent_proto::decode_request(&frame).unwrap()
    }

    #[test]
    fn ipc_mapping_uses_code_policy_without_confusing_transience_and_health() {
        use IpcErrorCode::*;
        for (code, connection_lost, ambiguous, expected, retryable, fatal) in [
            (
                DeadlineExceeded,
                false,
                false,
                "deadline-exceeded",
                true,
                false,
            ),
            (Cancelled, false, false, "cancelled", true, false),
            (Io, false, false, "io", false, false),
            (Io, true, false, "agent-lost", true, true),
            (Io, true, true, "agent-lost", false, true),
            (DeadlineExceeded, true, false, "agent-lost", true, true),
            (DeadlineExceeded, false, true, "ambiguous", false, false),
            (Protocol, true, false, "protocol", false, true),
            (
                ResponseBinding,
                false,
                true,
                "response-binding",
                false,
                true,
            ),
        ] {
            let mapped = AgentError::from_desktop(DesktopAgentError::Ipc {
                code,
                message: "original cause".into(),
                ambiguous,
                connection_lost,
            });
            assert_eq!(mapped.code, expected);
            assert_eq!(mapped.message, "original cause");
            assert_eq!(
                (mapped.retryable, mapped.fatal, mapped.ambiguous),
                (retryable, fatal, ambiguous),
                "{code:?}"
            );
        }
    }

    #[test]
    fn client_and_desktop_mappings_preserve_health_separately_from_ambiguity() {
        use foks_agent_client::Error;
        let errors = vec![
            Error::Unsupported,
            Error::Io(std::io::ErrorKind::InvalidInput.into()),
            Error::Io(std::io::ErrorKind::PermissionDenied.into()),
            Error::Cancelled,
            Error::DeadlineExceeded,
            Error::UnsafeSocket,
            Error::ResponseBinding,
            Error::Protocol(foks_agent_proto::Error::Version),
            Error::Protocol(foks_agent_proto::Error::TooLarge),
            Error::Io(std::io::ErrorKind::ConnectionRefused.into()),
            Error::Io(std::io::ErrorKind::UnexpectedEof.into()),
            Error::UploadSource(std::io::ErrorKind::UnexpectedEof.into()),
        ];
        for error in errors {
            let lost = error.is_connection_loss();
            let direct = AgentError::from_client(&error);
            let desktop = client_to_desktop(error);
            assert_eq!(desktop.connection_lost(), lost);
            let mapped = AgentError::from_desktop(desktop);
            assert_eq!(mapped, direct);
        }
        for cause in [
            Error::Cancelled,
            Error::DeadlineExceeded,
            Error::Io(std::io::ErrorKind::UnexpectedEof.into()),
            Error::ResponseBinding,
            Error::Protocol(foks_agent_proto::Error::Version),
        ] {
            let error = Error::Ambiguous(Box::new(cause));
            let lost = error.is_connection_loss();
            let direct = AgentError::from_client(&error);
            let desktop = client_to_desktop(error);
            assert!(desktop.ambiguous());
            assert_eq!(desktop.connection_lost(), lost);
            let mapped = AgentError::from_desktop(desktop);
            assert!(mapped.ambiguous && !mapped.retryable);
            assert_eq!(mapped, direct);
        }
    }

    #[test]
    fn observed_cancellation_and_deadline_leave_the_next_call_healthy() {
        use std::io::{Read as _, Write as _};
        use std::os::unix::net::UnixListener;
        for deadline in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("agent.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
            let cancelled = Arc::new(AtomicBool::new(false));
            let signal = cancelled.clone();
            let server = std::thread::spawn(move || {
                let (mut first, _) = listener.accept().unwrap();
                let request = read_socket_request(&mut first);
                assert!(matches!(request.operation, Operation::Ping));
                first.write_all(&[0, 0]).unwrap();
                if !deadline {
                    signal.store(true, Ordering::Release);
                }
                assert_eq!(first.read(&mut [0; 1]).unwrap(), 0);
                let (mut second, _) = listener.accept().unwrap();
                let request = read_socket_request(&mut second);
                second
                    .write_all(
                        &foks_agent_proto::encode(&Response::success(
                            request.id,
                            serde_json::json!({"healthy": true}),
                        ))
                        .unwrap(),
                    )
                    .unwrap();
            });
            let mut handle = AgentHandle::new(socket);
            Arc::get_mut(&mut handle.transport)
                .unwrap()
                .client
                .set_timeout(Duration::from_millis(500))
                .unwrap();
            let result = handle
                .transport()
                .call_cancellable(Operation::Ping, &|| cancelled.load(Ordering::Acquire));
            if deadline {
                assert!(matches!(result, Err(DesktopAgentError::DeadlineExceeded)));
            } else {
                assert!(matches!(result, Err(DesktopAgentError::Cancelled)));
            }
            assert_eq!(handle.take_connection_failure(), None);
            assert!(handle.transport().call(Operation::Ping).is_ok());
            assert_eq!(handle.take_connection_failure(), None);
            server.join().unwrap();
        }
    }

    #[test]
    fn observed_eof_records_loss_even_when_the_mutation_is_ambiguous() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_socket_request(&mut stream);
        });
        let handle = AgentHandle::new(socket);
        let error = handle
            .transport()
            .call(Operation::RemoveProfile {
                name: "local".into(),
            })
            .unwrap_err();
        assert!(error.ambiguous() && error.connection_lost());
        assert!(handle.take_connection_failure().is_some());
        server.join().unwrap();
    }

    #[test]
    fn connection_loss_notifies_once_per_pending_failure() {
        let directory = tempfile::tempdir().unwrap();
        let handle = AgentHandle::new(directory.path().join("agent.sock"));
        let notifications = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&notifications);
        let flag = Arc::clone(&handle.connection_failure);
        handle.set_connection_loss_notifier(move || {
            // The app layer reads the flag back from this callback, so it must
            // not run under the flag's own lock.
            assert!(flag.try_lock().is_ok());
            counter.fetch_add(1, Ordering::Release);
        });

        let ambiguous: Result<(), DesktopAgentError> = Err(DesktopAgentError::Ambiguous(
            "the write may have applied".into(),
        ));
        handle.transport.record(&ambiguous);
        assert_eq!(notifications.load(Ordering::Acquire), 0);
        assert_eq!(handle.take_connection_failure(), None);

        let lost: Result<(), DesktopAgentError> = Err(DesktopAgentError::Transport(
            "the agent socket closed".into(),
        ));
        handle.transport.record(&lost);
        assert_eq!(notifications.load(Ordering::Acquire), 1);

        // Subsequent errors while a failure is already pending should not trigger
        // extra notifications.
        let again: Result<(), DesktopAgentError> = Err(DesktopAgentError::Transport(
            "the agent socket closed again".into(),
        ));
        handle.transport.record(&again);
        assert_eq!(notifications.load(Ordering::Acquire), 1);
        assert_eq!(
            handle.take_connection_failure().as_deref(),
            Some("the agent socket closed again")
        );

        // Once cleared, a subsequent loss represents a new state transition and
        // notifies again.
        handle.transport.record(&lost);
        assert_eq!(notifications.load(Ordering::Acquire), 2);
        assert!(handle.take_connection_failure().is_some());
    }

    #[test]
    fn auto_recovery_validates_status_and_clears_only_successful_probe_failures() {
        use std::io::Write as _;
        for response_kind in ["ready", "bootstrap", "error", "invalid", "version"] {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("agent.sock");
            let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_socket_request(&mut stream);
                let response = match response_kind {
                    "error" => Response::error(request.id, ErrorCode::Busy, "not ready"),
                    "version" => {
                        Response::error(request.id, ErrorCode::VersionMismatch, "incompatible")
                    }
                    "invalid" => {
                        Response::success(request.id, serde_json::json!({"unrelated": true}))
                    }
                    "bootstrap" => Response::success(
                        request.id,
                        serde_json::to_value(foks_agent_proto::AgentStatus::Bootstrap {
                            step: "choose".into(),
                        })
                        .unwrap(),
                    ),
                    _ => Response::success(
                        request.id,
                        serde_json::to_value(foks_agent_proto::AgentStatus::ready()).unwrap(),
                    ),
                };
                stream
                    .write_all(&foks_agent_proto::encode(&response).unwrap())
                    .unwrap();
            });
            let handle = AgentHandle::new(socket);
            *handle.connection_failure.lock().unwrap() = Some("older connection loss".into());
            let result = handle.auto_recover_blocking();
            assert_eq!(
                result.is_ok(),
                matches!(response_kind, "ready" | "bootstrap")
            );
            assert_eq!(handle.take_connection_failure().is_none(), result.is_ok());
            if let Err(error) = result {
                assert_eq!(
                    error.code,
                    match response_kind {
                        "version" => "version-mismatch",
                        "invalid" => "protocol",
                        _ => "busy",
                    }
                );
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn recovery_credentials_are_distinct_and_explicit_retry_rechecks_them() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = root.join("client-state.toml");
        std::fs::write(&config, b"fixture").unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        let checks = Arc::new(AtomicU64::new(0));
        let observed = checks.clone();
        let process = FakeMaintenanceProcess::healthy();
        let handle = AgentHandle::new_for_maintenance_test(
            root.join("agent.sock"),
            process.clone(),
            move |root, _| {
                if observed.fetch_add(1, Ordering::SeqCst) < 2 {
                    SafeRootDisposition::CredentialsRequired(root.to_owned())
                } else {
                    SafeRootDisposition::Current
                }
            },
        );
        assert_eq!(
            handle.check_recovery_credentials(false).unwrap_err().code,
            "agent-credentials-required"
        );
        assert_eq!(
            handle.retry_started_blocking().unwrap_err().code,
            "agent-credentials-required"
        );
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
        handle.retry_started_blocking().unwrap();
        assert_eq!(checks.load(Ordering::SeqCst), 3);
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
        assert!(!handle.recovery_credentials_required.load(Ordering::Acquire));
    }

    #[test]
    fn automatic_recovery_checks_live_listener_before_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let handle = AgentHandle::new_for_maintenance_test(
            socket,
            FakeMaintenanceProcess::healthy(),
            |_, _| panic!("must not access credentials"),
        );
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_socket_request(&mut stream);
            drop(stream);
            // Keep the listener live through the second, socket-only check.
            let _ = listener.accept().unwrap();
        });
        assert_eq!(
            handle.auto_recover_blocking().unwrap_err().code,
            "agent-busy"
        );
        server.join().unwrap();
    }

    #[test]
    fn auto_recovery_endpoint_checks_allow_only_missing_or_private_stale_sockets() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let handle = AgentHandle::new(socket.clone());
        assert!(handle.require_missing_agent_endpoint().is_ok());
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            handle.require_missing_agent_endpoint().unwrap_err().code,
            "agent-busy"
        );
        drop(listener);
        assert!(handle.require_missing_agent_endpoint().is_ok());
        assert!(socket.exists());
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            handle.require_missing_agent_endpoint().unwrap_err().code,
            "unsafe-socket"
        );
        std::fs::remove_file(&socket).unwrap();
        std::os::unix::fs::symlink(directory.path().join("missing"), &socket).unwrap();
        assert_eq!(
            handle.require_missing_agent_endpoint().unwrap_err().code,
            "unsafe-socket"
        );
    }

    #[test]
    fn auto_recovery_never_initializes_or_overrides_maintenance_or_unsafe_endpoints() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let process = FakeMaintenanceProcess::healthy();
        let handle =
            AgentHandle::new_for_maintenance_test(socket.clone(), process.clone(), |_, _| {
                SafeRootDisposition::Current
            });
        assert_eq!(
            handle.auto_recover_blocking().unwrap_err().code,
            "agent-state"
        );
        assert!(!directory.path().join("client-state.toml").exists());
        assert_eq!(handle.take_connection_failure(), None);
        for (disposition, code) in [
            (
                TransportDisposition::RestorationFailed,
                "state-restoration-failed",
            ),
            (
                TransportDisposition::RecoveryRequired,
                "state-recovery-required",
            ),
            (
                TransportDisposition::RestartRequired,
                "state-restart-required",
            ),
        ] {
            *handle.transport.disposition.lock().unwrap() = disposition;
            assert_eq!(handle.auto_recover_blocking().unwrap_err().code, code);
        }
        assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
        *handle.transport.disposition.lock().unwrap() = TransportDisposition::Current;
        handle.maintenance_in_flight.store(true, Ordering::Release);
        assert_eq!(
            handle.auto_recover_blocking().unwrap_err().code,
            "state-maintenance-active"
        );
        handle.maintenance_in_flight.store(false, Ordering::Release);
        {
            let _request = handle.transport.reserve_use().unwrap();
            assert_eq!(
                handle
                    .auto_recover_with_reservation_timeout(Duration::ZERO)
                    .unwrap_err()
                    .code,
                "agent-busy"
            );
        }
        std::fs::write(&socket, b"not a socket").unwrap();
        assert_eq!(
            handle.auto_recover_blocking().unwrap_err().code,
            "unsafe-socket"
        );
        assert_eq!(
            handle.call_blocking(Operation::Ping).unwrap_err().code,
            "unsafe-socket"
        );
        assert!(!handle
            .transport()
            .call(Operation::Ping)
            .unwrap_err()
            .connection_lost());
        assert_eq!(handle.take_connection_failure(), None);
        assert_eq!(std::fs::read(&socket).unwrap(), b"not a socket");
        std::fs::remove_file(&socket).unwrap();
        let external = AgentHandle::new(socket);
        assert_eq!(
            external.auto_recover_blocking().unwrap_err().code,
            "external-agent"
        );
        assert_eq!(external.take_connection_failure(), None);
        assert_eq!(
            external.call_blocking(Operation::Ping).unwrap_err().code,
            "agent-lost"
        );
        assert!(external.take_connection_failure().is_some());
    }

    #[test]
    fn classifications_preserve_ambiguity_and_fatality() {
        let credentials =
            AgentError::from_agent(ErrorCode::CredentialsRequired, "locked".to_owned());
        assert_eq!(credentials.code, "credentials-required");
        assert!(!credentials.retryable && !credentials.ambiguous && !credentials.fatal);
        assert_ne!(
            credentials.code,
            AgentError::from_agent(ErrorCode::ReauthenticationRequired, "sign in".to_owned()).code
        );
        let timeout = AgentError::from_agent(ErrorCode::DeadlineExceeded, "slow".to_owned());
        assert!(timeout.retryable && timeout.ambiguous);
        assert!(AgentError::from_agent(ErrorCode::VersionMismatch, "old".to_owned()).fatal);
        let rate_limited = AgentError::from_agent(ErrorCode::RateLimited, "slow down".to_owned());
        assert_eq!(rate_limited.code, "rate-limited");
        assert!(rate_limited.retryable);
        let quota = AgentError::from_agent(ErrorCode::QuotaExceeded, "full".to_owned());
        assert_eq!(quota.code, "quota-exceeded");
        assert!(!quota.retryable);
        let ambiguous = AgentError::from_client(&foks_agent_client::Error::Ambiguous(Box::new(
            foks_agent_client::Error::Cancelled,
        )));
        assert!(ambiguous.ambiguous && !ambiguous.retryable);
        let protocol_version = AgentError::from_client(&foks_agent_client::Error::Protocol(
            foks_agent_proto::Error::Version,
        ));
        assert_eq!(protocol_version.code, "version-mismatch");
        assert!(protocol_version.fatal && !protocol_version.retryable);
    }

    #[cfg(unix)]
    #[test]
    fn only_named_foks_agent_processes_are_replaceable() {
        assert!(is_foks_agent_executable(Path::new(
            "/Applications/FOKS.app/Contents/MacOS/foks-agent"
        )));
        assert!(is_foks_agent_executable(Path::new(
            "/tmp/foks-agent (deleted)"
        )));
        assert!(!is_foks_agent_executable(Path::new(
            "/Applications/FOKS.app/Contents/MacOS/foks-desktop"
        )));
        assert!(!is_replaceable_agent_process(0));
        assert!(!is_replaceable_agent_process(1));
        assert!(!is_replaceable_agent_process(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn agent_state_directory_requires_an_explicit_argument() {
        assert_eq!(
            agent_state_dir(&[
                "foks-agent".into(),
                "--state-dir".into(),
                "/private/foks".into(),
            ]),
            Some(PathBuf::from("/private/foks"))
        );
        assert_eq!(
            agent_state_dir(&["foks-agent".into(), "--state-dir=/private/other".into(),]),
            Some(PathBuf::from("/private/other"))
        );
        assert_eq!(agent_state_dir(&["foks-agent".into()]), None);
        assert_eq!(
            agent_state_dir(&["foks-agent".into(), "--state-dir".into()]),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_inspection_identifies_the_current_invocation() {
        let pid = std::process::id();
        assert_eq!(process_owner_uid(pid).unwrap(), unsafe { libc::geteuid() });
        assert!(!process_arguments(pid).unwrap().is_empty());
        assert!(process_ids().unwrap().contains(&pid));
        assert!(process_start_time(pid).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn unix_peer_pid_identifies_the_same_process_listener() {
        use std::os::unix::fs::PermissionsExt as _;
        use std::os::unix::net::{UnixListener, UnixStream};

        let temporary = tempfile::tempdir().unwrap();
        let socket = temporary.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(200));
        });
        let stream = UnixStream::connect(&socket).unwrap();
        assert_eq!(unix_peer_pid(&stream).unwrap(), std::process::id());
        drop(stream);
        server.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn parent_pid_and_orphan_detection_read_the_process_table() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        assert_eq!(process_parent_pid(pid).unwrap(), std::process::id());
        // A live child is supervised by this process, not reparented to init.
        assert!(!process_is_orphaned(pid));
        assert!(!process_is_orphaned(std::process::id()));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(process_parent_pid(pid).is_err() || process_has_exited(pid));
    }

    #[cfg(unix)]
    #[test]
    fn same_file_compares_by_identity_not_path() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("foks-agent");
        std::fs::write(&file, b"").unwrap();
        let link = temporary.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(same_file(&file, &link));
        let other = temporary.path().join("other");
        std::fs::write(&other, b"").unwrap();
        assert!(!same_file(&file, &other));
        assert!(!same_file(&file, &temporary.path().join("missing")));
    }

    #[cfg(unix)]
    #[test]
    fn takeover_confirmation_revalidates_owners_and_continues_startup() {
        use std::cell::RefCell;
        use std::os::unix::process::ExitStatusExt as _;

        struct Fixture {
            root: tempfile::TempDir,
            binary: PathBuf,
            children: RefCell<Vec<std::process::Child>>,
        }
        impl Fixture {
            fn spawn(&self, socket: &Path, version: u32) -> u32 {
                let marker = self
                    .root
                    .path()
                    .join(format!("ready-{}", self.children.borrow().len()));
                let child = Command::new(&self.binary)
                    .arg(socket)
                    .arg(&marker)
                    .arg(version.to_string())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let pid = child.id();
                self.children.borrow_mut().push(child);
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while !marker.exists() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "dummy agent did not bind"
                    );
                    assert!(self
                        .children
                        .borrow_mut()
                        .last_mut()
                        .unwrap()
                        .try_wait()
                        .unwrap()
                        .is_none());
                    std::thread::sleep(Duration::from_millis(10));
                }
                pid
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                // Only clean up processes launched from this temporary test binary.
                let pid = *MANAGED_AGENT_PID.lock().unwrap();
                if let Some(pid) = pid
                    .filter(|pid| process_executable_path(*pid).ok().as_ref() == Some(&self.binary))
                {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                    for _ in 0..100 {
                        if process_has_exited(pid) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                for child in self.children.get_mut() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let binary = root.path().join("foks-agent");
        let source = root.path().join("agent.c");
        std::fs::write(&source, include_str!("../tests/fixtures/takeover-agent.c")).unwrap();
        assert!(Command::new("cc")
            .arg(format!(
                "-DPROTOCOL_VERSION={}",
                foks_agent_proto::PROTOCOL_VERSION
            ))
            .arg("-o")
            .arg(&binary)
            .arg(&source)
            .status()
            .unwrap()
            .success());
        let fixture = Fixture {
            root,
            binary: binary.canonicalize().unwrap(),
            children: RefCell::new(Vec::new()),
        };
        let socket = fixture.root.path().join("agent.sock");
        let handle = AgentHandle::new(socket.clone());
        let old = fixture.spawn(&socket, 1);
        assert_eq!(
            handle
                .call_blocking(Operation::AgentStatus)
                .unwrap_err()
                .code,
            "version-mismatch"
        );

        let error = handle
            .start_with_binary(&fixture.root.path().join("missing-agent"), &|_| {
                panic!("must validate replacement before requesting termination")
            })
            .unwrap_err();
        assert_eq!(error.code, "agent-binary");
        let error = handle
            .start_with_binary(&fixture.binary, &|_| Ok(false))
            .unwrap_err();
        assert_eq!(error.code, "agent-takeover-declined");
        assert_eq!(inspect_takeover_target(&socket).unwrap().unwrap().pid, old);

        // Replacement while the confirmation is open requires a new approval.
        let prompts = RefCell::new(Vec::new());
        let error = handle
            .start_with_binary(&fixture.binary, &|target| {
                prompts.borrow_mut().push(target.pid);
                if target.pid == old {
                    fixture.spawn(&socket, 1); // deletes and rebinds the original path
                    Ok(true)
                } else {
                    Ok(false)
                }
            })
            .unwrap_err();
        assert_eq!(error.code, "agent-takeover-declined");
        assert_eq!(prompts.borrow().len(), 2);
        assert_ne!(prompts.borrow()[0], prompts.borrow()[1]);
        assert!(fixture
            .children
            .borrow_mut()
            .iter_mut()
            .all(|child| child.try_wait().unwrap().is_none()));

        // A new explicit approval actually stops that owner and launches our agent.
        let response = handle
            .start_with_binary(&fixture.binary, &|target| {
                assert_eq!(target.pid, prompts.borrow()[1]);
                Ok(true)
            })
            .unwrap();
        assert_eq!(success_value(response).unwrap()["state"], "ready");
        let status = fixture.children.borrow_mut()[1].wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
        assert!(handle.call_blocking(Operation::Ping).is_ok());

        // An already-compatible successor is reused, never terminated or prompted.
        let compatible_socket = fixture.root.path().join("compatible.sock");
        fixture.spawn(&compatible_socket, 1);
        let compatible = AgentHandle::new(compatible_socket.clone());
        let calls = RefCell::new(0);
        compatible
            .start_with_binary(&fixture.binary, &|_| {
                *calls.borrow_mut() += 1;
                fixture.spawn(&compatible_socket, foks_agent_proto::PROTOCOL_VERSION);
                Ok(true)
            })
            .unwrap();
        assert_eq!(*calls.borrow(), 1);
        assert!(fixture
            .children
            .borrow_mut()
            .last_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none());

        // A respawning supervisor cannot trap startup in an unbounded kill loop.
        let contested_socket = fixture.root.path().join("contested.sock");
        fixture.spawn(&contested_socket, 1);
        let contested = AgentHandle::new(contested_socket.clone());
        let calls = RefCell::new(0);
        let error = contested
            .start_with_binary(&fixture.binary, &|_| {
                *calls.borrow_mut() += 1;
                fixture.spawn(&contested_socket, 1);
                Ok(true)
            })
            .unwrap_err();
        assert_eq!(error.code, "agent-takeover-limit");
        assert_eq!(*calls.borrow(), MAX_STARTUP_TAKEOVERS);
    }

    #[cfg(unix)]
    #[test]
    fn takeover_refuses_unsafe_sockets_and_unrelated_processes() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let socket = root.path().join("agent.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            inspect_takeover_target(&socket).unwrap_err().code,
            "unsafe-socket"
        );
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            inspect_takeover_target(&socket).unwrap_err().code,
            "version-mismatch"
        );
        assert!(socket.exists());
    }

    #[cfg(unix)]
    #[test]
    fn stop_incompatible_agent_terminates_a_named_listener() {
        use std::os::unix::process::ExitStatusExt as _;
        use std::process::{Command, Stdio};

        struct ChildGuard(std::process::Child);
        impl std::ops::Deref for ChildGuard {
            type Target = std::process::Child;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl std::ops::DerefMut for ChildGuard {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let temporary = tempfile::tempdir().unwrap();
        let socket = temporary.path().join("agent.sock");
        let ready = temporary.path().join("ready");
        let source = temporary.path().join("agent.c");
        let binary = temporary.path().join("foks-agent");
        std::fs::write(
            &source,
            r#"
#include <fcntl.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>
int main(int argc, char **argv) {
    int server = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un address;
    memset(&address, 0, sizeof address);
    address.sun_family = AF_UNIX;
    strncpy(address.sun_path, argv[1], sizeof address.sun_path - 1);
    unlink(argv[1]);
    if (server < 0 || bind(server, (struct sockaddr *)&address, sizeof address) != 0
        || chmod(argv[1], 0600) != 0 || listen(server, 8) != 0) {
        return 1;
    }
    int marker = open(argv[2], O_CREAT | O_WRONLY, 0600);
    if (marker >= 0) {
        close(marker);
    }
    for (;;) {
        int client = accept(server, 0, 0);
        if (client >= 0) {
            char byte;
            read(client, &byte, 1);
            close(client);
        }
    }
}
"#,
        )
        .unwrap();
        let compiled = Command::new("cc")
            .args(["-o"])
            .arg(&binary)
            .arg(&source)
            .status()
            .expect("cc is required to build the dummy agent");
        assert!(compiled.success(), "cc failed to build the dummy agent");
        let mut child = ChildGuard(
            Command::new(&binary)
                .args([&socket, &ready])
                .arg("--state-dir")
                .arg(temporary.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let started = std::time::Instant::now();
        while !ready.exists() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("dummy foks-agent exited early: {status}");
            }
            if started.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                panic!("dummy foks-agent did not bind");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let handle = AgentHandle::new(socket.clone());
        let candidates = handle.stale_agent_processes();
        let active = candidates
            .iter()
            .find(|target| target.pid == child.id())
            .unwrap();
        assert!(handle.additional_agent_processes().is_empty());
        assert_eq!(
            handle.terminate_additional_agent(active).unwrap_err().code,
            "agent-cleanup-changed"
        );
        let missing = AgentHandle::new(temporary.path().join("missing.sock"));
        assert!(missing.additional_agent_processes().is_empty());
        assert_eq!(
            missing.terminate_additional_agent(active).unwrap_err().code,
            "agent-cleanup-changed"
        );

        let extra_socket = temporary.path().join("extra.sock");
        let extra_ready = temporary.path().join("extra-ready");
        let mut extra = ChildGuard(
            Command::new(&binary)
                .args([&extra_socket, &extra_ready])
                .arg("--state-dir")
                .arg(temporary.path())
                .spawn()
                .unwrap(),
        );
        let started = std::time::Instant::now();
        while !extra_ready.exists() {
            if started.elapsed() > Duration::from_secs(5) {
                let _ = extra.kill();
                let _ = extra.wait();
                let _ = child.kill();
                let _ = child.wait();
                panic!("additional agent did not bind");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let candidates = handle.additional_agent_processes();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pid, extra.id());
        let now_active = AgentHandle::new(extra_socket.clone());
        assert_eq!(
            now_active
                .terminate_additional_agent(&candidates[0])
                .unwrap_err()
                .code,
            "agent-cleanup-changed"
        );
        // An unreachable listener is still discoverable and can be cleaned up.
        std::fs::remove_file(&extra_socket).unwrap();
        assert_eq!(handle.additional_agent_processes(), candidates);
        let mut changed = candidates[0].clone();
        changed.started_at += 1;
        assert_eq!(
            handle
                .terminate_additional_agent(&changed)
                .unwrap_err()
                .code,
            "agent-cleanup-changed"
        );
        handle.terminate_additional_agent(&candidates[0]).unwrap();
        assert_eq!(extra.wait().unwrap().signal(), Some(libc::SIGTERM));
        assert!(child.try_wait().unwrap().is_none());

        let mut approvals = 0;
        stop_incompatible_agent(
            &socket,
            &|target| {
                assert_eq!(target.pid, child.id());
                assert_eq!(target.executable, binary.canonicalize().unwrap());
                Ok(true)
            },
            &mut approvals,
        )
        .unwrap();
        assert_eq!(approvals, 1);
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }
}
