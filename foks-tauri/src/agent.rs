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

/// The socket selected for this process and any state that this desktop owns.
///
/// An explicit argument or environment socket is an external trust boundary:
/// it may live beside files owned by another launcher, so it never grants this
/// process a directory in which to write crash markers.
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
                "FOKS cannot talk to a local agent on this platform.",
                false,
            ),
            Error::UnsafeSocket => Self::new(
                "unsafe-socket",
                "The agent socket is not private to this account; FOKS will not use it.",
                false,
            ),
            Error::Io(error) => Self::new(
                "io",
                format!("The agent could not be reached: {error}"),
                true,
            ),
            Error::Protocol(error) => Self::new(
                "protocol",
                format!("The agent spoke an unexpected protocol: {error}"),
                false,
            ),
            Error::ResponseBinding => Self::new(
                "response-binding",
                "The agent reply did not match the request; FOKS discarded it.",
                true,
            ),
            Error::Ambiguous(detail) => {
                let mut mapped = Self::new(
                    "ambiguous",
                    format!("The change may or may not have been applied: {detail}"),
                    false,
                );
                mapped.ambiguous = true;
                mapped
            }
        }
    }

    pub fn from_agent(code: ErrorCode, message: String) -> Self {
        let (slug, retryable) = match code {
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
            ErrorCode::OperationFailed => ("operation-failed", false),
        };
        let mut mapped = Self::new(slug, message, retryable);
        mapped.ambiguous = code == ErrorCode::DeadlineExceeded;
        mapped.fatal = code == ErrorCode::VersionMismatch;
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
                message: "the desktop and local agent protocol versions do not match".to_owned(),
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
                AgentError::unknown(format!("the agent call did not finish: {error}"))
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
        if let Ok(response) = self.call_blocking(Operation::AgentStatus) {
            self.clear_connection_failure();
            return Ok(response);
        }
        let Some(binary) = managed_agent_binary(&self.socket) else {
            return self.call_blocking(Operation::AgentStatus);
        };
        let state_dir = self.socket.parent().ok_or_else(|| {
            AgentError::unknown("the configured agent socket has no parent directory")
        })?;
        prepare_state_directory(state_dir)?;
        let spawn_lock = acquire_spawn_lock(&self.socket)?;
        if let Ok(response) = self.call_blocking(Operation::AgentStatus) {
            drop(spawn_lock);
            self.clear_connection_failure();
            return Ok(response);
        }
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
                "The managed agent did not create its socket.",
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
    #[cfg(target_os = "macos")]
    let candidate = executable.parent()?.parent()?.join("Helpers/foks-agent");
    #[cfg(not(target_os = "macos"))]
    let candidate = executable.parent()?.join("foks-agent");
    Some(candidate)
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
                format!("could not open {}: {error}", path.display()),
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
            "The desktop agent lock is not a private regular file owned by this account.",
            false,
        ));
    }
    file.lock_exclusive().map_err(|error| {
        AgentError::new(
            "agent-lock",
            format!("could not lock {}: {error}", path.display()),
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
                "the packaged local agent {} is unavailable: {error}",
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
        return Err(AgentError::new("agent-binary", "The packaged local agent must be an executable, non-writable regular file owned like the desktop.", false));
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
            "The FOKS state path must be a real directory owned by this account.",
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
            "The managed crash directory has no private state parent.",
            false,
        )
    })?;
    prepare_state_directory(state)?;
    prepare_state_directory(directory).map_err(|error| AgentError {
        code: "crash-state".to_owned(),
        ..error
    })
}

fn managed_agent_arguments(state_dir: &Path, socket: &Path) -> [std::ffi::OsString; 4] {
    [
        "--state-dir".into(),
        state_dir.as_os_str().to_owned(),
        "--socket".into(),
        socket.as_os_str().to_owned(),
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
                format!("could not open agent log: {error}"),
                false,
            )
        })?;
    let log_metadata = log
        .metadata()
        .map_err(|error| AgentError::new("agent-log", error.to_string(), false))?;
    if !log_metadata.is_file() || log_metadata.uid() != unsafe { libc::geteuid() } {
        return Err(AgentError::new(
            "agent-log",
            "The agent log must be a regular file owned by this account.",
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
                format!("could not launch local agent: {error}"),
                true,
            )
        })?;
    std::thread::Builder::new()
        .name("foks-agent-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        })
        .map_err(|error| {
            AgentError::new(
                "agent-start-failed",
                format!("could not supervise local agent: {error}"),
                false,
            )
        })?;
    Ok(())
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
        socket: state.join("agent.sock"),
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
    return Some(home.join("Library/Application Support/FOKS"));
    #[cfg(not(target_os = "macos"))]
    return Some(
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("foks-rs"),
    );
}

pub fn default_socket() -> Option<PathBuf> {
    Some(default_state_directory()?.join("agent.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

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
    fn managed_launch_passes_only_the_named_state_and_socket_arguments() {
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
                "/Applications/FOKS.app/Contents/Helpers/foks-agent"
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
        let ambiguous =
            AgentError::from_client(&foks_agent_client::Error::Ambiguous("write".to_owned()));
        assert!(ambiguous.ambiguous && !ambiguous.retryable);
    }
}
