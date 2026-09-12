//! Blocking local-agent transport, managed launch, and connection observation.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use foks_agent_client::AgentClient;
use foks_agent_proto::{ErrorCode, ErrorFields, Operation, Response, ResponseResult};
use foks_desktop::{AgentError as DesktopAgentError, AgentTransport};
use fs2::FileExt as _;
use serde::Serialize;
use serde_json::Value;

pub const SOCKET_ENV: &str = "FOKS_AGENT_SOCKET";
pub const SOCKET_ARG: &str = "--agent-socket";
const AGENT_BINARY_ENV: &str = "FOKS_AGENT_BINARY";
const DEFAULT_SOCKET_NAME: &str = "foks-rs.sock";

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
    pub reason: Option<String>,
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
        use foks_agent_client::Error;
        match error {
            Error::Unsupported => Self::new(
                "unsupported",
                "Local agent communication is not supported on this platform.",
                false,
            ),
            Error::UnsafeSocket => Self::new(
                "unsafe-socket",
                "The agent socket is not private to this user account and cannot be used.",
                false,
            ),
            Error::Io(error) => {
                Self::new("io", format!("Failed to connect to agent: {error}"), true)
            }
            Error::Protocol(foks_agent_proto::Error::Version) => Self::from_agent(
                ErrorCode::VersionMismatch,
                "Desktop and agent protocol versions do not match".to_owned(),
            ),
            Error::Protocol(error) => Self::new(
                "protocol",
                format!("Unexpected agent protocol response: {error}"),
                false,
            ),
            Error::ResponseBinding => Self::new(
                "response-binding",
                "The agent response did not match the request.",
                true,
            ),
            Error::Ambiguous(detail) => {
                let mut mapped = Self::new(
                    "ambiguous",
                    format!("Ambiguous operation result: {detail}"),
                    false,
                );
                mapped.ambiguous = true;
                mapped
            }
        }
    }

    pub fn from_agent(code: ErrorCode, message: String) -> Self {
        let (slug, retryable) = match code {
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
            ErrorCode::Conflict => ("conflict", false),
            ErrorCode::Busy => ("busy", true),
            ErrorCode::DeadlineExceeded => ("deadline-exceeded", true),
            ErrorCode::CapabilityDenied => ("capability-denied", false),
            ErrorCode::RollbackDetected => ("rollback-detected", false),
            ErrorCode::CheckpointResetRequired => ("checkpoint-reset-required", false),
            ErrorCode::ProfileBusy => ("profile-busy", true),
            ErrorCode::RateLimited => ("rate-limited", true),
            ErrorCode::QuotaExceeded => ("quota-exceeded", false),
            ErrorCode::OperationFailed => ("operation-failed", false),
        };
        let mut mapped = Self::new(slug, message, retryable);
        mapped.ambiguous = code == ErrorCode::DeadlineExceeded;
        mapped.fatal = matches!(
            code,
            ErrorCode::VersionMismatch | ErrorCode::ChatIntegrity | ErrorCode::ChatChannelIntegrity
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
                mapped.details = error_details(fields).map(Box::new);
                mapped
            }
            DesktopAgentError::Transport(message) => Self::new("io", message, true),
            DesktopAgentError::Ambiguous(message) => {
                let mut mapped = Self::new("ambiguous", message, false);
                mapped.ambiguous = true;
                mapped
            }
            DesktopAgentError::Cancelled => {
                Self::new("cancelled", "The request was cancelled.", true)
            }
        }
    }
}

fn error_details(fields: ErrorFields) -> Option<AgentErrorDetails> {
    let details = AgentErrorDetails {
        capability: fields.capability,
        profile: fields.profile,
        reason: fields.reason,
    };
    (details.capability.is_some() || details.profile.is_some() || details.reason.is_some())
        .then_some(details)
}

struct ObservedTransport {
    client: AgentClient,
    connection_failure: Arc<Mutex<Option<String>>>,
}

impl ObservedTransport {
    fn record(&self, result: &Result<Value, DesktopAgentError>) {
        if let Err(DesktopAgentError::Transport(message)) = result {
            *self
                .connection_failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.clone());
        }
    }
}

impl AgentTransport for ObservedTransport {
    fn call_cancellable(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, DesktopAgentError> {
        let result = match self
            .client
            .call_cancellable(operation, cancelled)
            .map_err(client_to_desktop)
        {
            Ok(response) => response_result(response.result),
            Err(error) => Err(error),
        };
        self.record(&result);
        result
    }

    fn call(&self, operation: Operation) -> Result<Value, DesktopAgentError> {
        let result = match self.client.call(operation).map_err(client_to_desktop) {
            Ok(response) => response_result(response.result),
            Err(error) => Err(error),
        };
        self.record(&result);
        result
    }

    fn put_kv_stream(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<Value, DesktopAgentError> {
        let result = match self
            .client
            .put_kv_stream(header, reader)
            .map_err(client_to_desktop)
        {
            Ok(response) => response_result(response.result),
            Err(error) => Err(error),
        };
        self.record(&result);
        result
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
            fields,
        }),
    }
}

fn client_to_desktop(error: foks_agent_client::Error) -> DesktopAgentError {
    match error {
        foks_agent_client::Error::Ambiguous(message) => DesktopAgentError::Ambiguous(message),
        foks_agent_client::Error::Protocol(foks_agent_proto::Error::Version) => {
            DesktopAgentError::Protocol {
                code: ErrorCode::VersionMismatch,
                message: "Desktop and agent protocol versions do not match".to_owned(),
                fields: Default::default(),
            }
        }
        error => DesktopAgentError::Transport(error.to_string()),
    }
}

pub struct AgentHandle {
    transport: Arc<ObservedTransport>,
    socket: PathBuf,
    connection_failure: Arc<Mutex<Option<String>>>,
}

impl AgentHandle {
    pub fn new(socket: PathBuf) -> Self {
        let mut client = AgentClient::new(&socket);
        client
            .set_timeout(Duration::from_secs(60))
            .expect("the agent client accepts a 60-second timeout");
        let connection_failure = Arc::new(Mutex::new(None));
        Self {
            transport: Arc::new(ObservedTransport {
                client,
                connection_failure: Arc::clone(&connection_failure),
            }),
            socket,
            connection_failure,
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn transport(&self) -> Arc<dyn AgentTransport> {
        self.transport.clone()
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
        match self.transport.client.call(operation) {
            Ok(response) => Ok(response),
            Err(error) => {
                *self
                    .connection_failure
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error.to_string());
                Err(AgentError::from_client(&error))
            }
        }
    }

    pub fn ensure_started_blocking(&self) -> Result<Response, AgentError> {
        let mut incompatible = false;
        match self.call_blocking(Operation::AgentStatus) {
            Ok(response) => {
                self.clear_connection_failure();
                return Ok(response);
            }
            Err(error) if error.code == "version-mismatch" => incompatible = true,
            Err(_) => {}
        }
        let Some(binary) = managed_agent_binary(&self.socket) else {
            return self.call_blocking(Operation::AgentStatus);
        };
        let state_dir = self.socket.parent().ok_or_else(|| {
            AgentError::unknown("Configured agent socket path has no parent directory.")
        })?;
        prepare_state_directory(state_dir)?;
        let spawn_lock = acquire_spawn_lock(&self.socket)?;
        match self.call_blocking(Operation::AgentStatus) {
            Ok(response) => {
                drop(spawn_lock);
                self.clear_connection_failure();
                return Ok(response);
            }
            Err(error) if error.code == "version-mismatch" => incompatible = true,
            Err(_) => {}
        }
        // A previous FOKS build's resident agent can still own this socket
        // after a protocol bump; it must be stopped before this binary can bind.
        #[cfg(unix)]
        if incompatible {
            stop_incompatible_agent(&self.socket)?;
            if let Ok(response) = self.call_blocking(Operation::AgentStatus) {
                drop(spawn_lock);
                self.clear_connection_failure();
                return Ok(response);
            }
        }
        #[cfg(not(unix))]
        let _ = incompatible;
        launch_agent(&binary, state_dir, &self.socket)?;
        let mut last_error = None;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            match self.call_blocking(Operation::AgentStatus) {
                Ok(response) => {
                    drop(spawn_lock);
                    self.clear_connection_failure();
                    return Ok(response);
                }
                Err(error) => last_error = Some(error),
            }
        }
        drop(spawn_lock);
        Err(last_error.unwrap_or_else(|| {
            AgentError::new(
                "agent-start-failed",
                "Failed to connect: background service endpoint was not created.",
                true,
            )
        }))
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
    launch_agent(&binary, state.path(), &socket).map_err(|error| error.message)?;
    let handle = AgentHandle::new(socket);
    let result = (|| {
        let mut last_error = "agent did not become ready".to_owned();
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            match handle.call_blocking(Operation::AgentStatus) {
                Ok(response) => return success_value(response).map(|_| ()).map_err(|e| e.message),
                Err(error) => last_error = error.message,
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

fn launch_agent(binary: &Path, state_dir: &Path, socket: &Path) -> Result<(), AgentError> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
    validate_agent_binary(binary)?;
    let log = OpenOptions::new()
        .create(true)
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
    std::thread::Builder::new()
        .name("foks-agent-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
            let mut guard = MANAGED_AGENT_PID
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *guard == Some(pid) {
                *guard = None;
            }
        })
        .map_err(|error| {
            AgentError::new(
                "agent-start-failed",
                format!("Failed to supervise local agent: {error}"),
                false,
            )
        })?;
    Ok(())
}

static MANAGED_AGENT_PID: Mutex<Option<u32>> = Mutex::new(None);

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
fn stop_incompatible_agent(socket: &Path) -> Result<(), AgentError> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
    use std::os::unix::net::UnixStream;

    let metadata = match std::fs::symlink_metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
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
            return Ok(());
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
fn process_has_exited(pid: u32) -> bool {
    let alive = unsafe { libc::kill(pid as i32, 0) };
    alive != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
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

    let mut pid: libc::pid_t = 0;
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
        ResponseResult::Error { code, message, .. } => Err(AgentError::from_agent(code, message)),
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
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    return Some(home.join("Library/Application Support/foks-rs"));
    #[cfg(not(target_os = "macos"))]
    return Some(
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("foks-rs"),
    );
}

pub fn default_socket() -> Option<PathBuf> {
    Some(default_state_directory()?.join(DEFAULT_SOCKET_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

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

    #[test]
    fn classifications_preserve_ambiguity_and_fatality() {
        let timeout = AgentError::from_agent(ErrorCode::DeadlineExceeded, "slow".to_owned());
        assert!(timeout.retryable && timeout.ambiguous);
        assert!(AgentError::from_agent(ErrorCode::VersionMismatch, "old".to_owned()).fatal);
        let rate_limited = AgentError::from_agent(ErrorCode::RateLimited, "slow down".to_owned());
        assert_eq!(rate_limited.code, "rate-limited");
        assert!(rate_limited.retryable);
        let quota = AgentError::from_agent(ErrorCode::QuotaExceeded, "full".to_owned());
        assert_eq!(quota.code, "quota-exceeded");
        assert!(!quota.retryable);
        let ambiguous =
            AgentError::from_client(&foks_agent_client::Error::Ambiguous("write".to_owned()));
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
    fn stop_incompatible_agent_terminates_a_named_listener() {
        use std::os::unix::process::ExitStatusExt as _;
        use std::process::{Command, Stdio};

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
        let mut child = Command::new(&binary)
            .args([&socket, &ready])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
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
        stop_incompatible_agent(&socket).unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }
}
