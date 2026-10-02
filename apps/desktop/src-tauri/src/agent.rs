//! Native agent facade and shared state.
//!
//! Transport, process ownership, and maintenance coordination keep their own
//! implementations. Shared lock state stays here so extraction does not change
//! ownership, reservation order, or the public API.

use crate::diagnostics::{Label, TimingLog};
use foks_agent_client::AgentClient;
use foks_agent_proto::Response;
use maintenance::safe_selected_root;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::Duration;

mod maintenance;
mod process;
mod transport;

pub(crate) use process::prepare_managed_crash_directory;
#[allow(unused_imports)] // Preserve the facade's existing public entry points.
pub use process::{
    default_socket, resolve_endpoint, smoke_test_packaged_startup, terminate_managed_agent,
};
pub use transport::success_value;

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

struct MaintenanceAdmission<'a>(&'a AtomicBool);

/// Clears the drain signal once the reservation is taken or given up on. The
/// exclusive reservation keeps later calls out by itself, so the signal only
/// has to last as long as the wait for it.
struct MaintenanceDrain<'a>(&'a ObservedTransport);

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

struct ObservedTransport {
    shutting_down: AtomicBool,
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

/// The transport a command hands to the shared catalog, roster and chat
/// code, so the operations issued on the command's behalf, on whatever
/// thread, are recorded under its name and profile.
struct LabelledTransport {
    inner: Arc<ObservedTransport>,
    label: Label,
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
    exit_targets: Mutex<Option<Vec<StaleAgentProcess>>>,
    exit_launch_gate: Mutex<()>,
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

enum SafeRootDisposition {
    Current,
    Selected(PathBuf),
    Recovery(PathBuf),
    CredentialsRequired(PathBuf),
}

const MAX_AGENT_STARTUP_DIAGNOSTIC_BYTES: u64 = 4096;

static MANAGED_AGENT_PID: Mutex<Option<u32>> = Mutex::new(None);

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
                shutting_down: AtomicBool::new(false),
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
            exit_targets: Mutex::new(None),
            exit_launch_gate: Mutex::new(()),
            maintenance_process: Arc::new(NativeMaintenanceProcess),
            maintenance_readiness: Arc::new(safe_selected_root),
            startup_gate,
            #[cfg(test)]
            managed_endpoint_override: false,
        }
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
}

#[cfg(test)]
mod tests;
