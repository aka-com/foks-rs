//! Managed binary validation, launch, process identity, and owned-process exit.

use super::maintenance::{inspect_takeover_target, stop_incompatible_agent};
use super::transport::success_value;
use super::{
    AgentEndpoint, AgentError, AgentHandle, AgentProcessInfo, AgentTakeover, StaleAgentProcess,
    AGENT_BINARY_ENV, DEFAULT_SOCKET_NAME, MAINTENANCE_RESERVATION_WAIT, MANAGED_AGENT_PID,
    MAX_AGENT_STARTUP_DIAGNOSTIC_BYTES, MAX_STARTUP_TAKEOVERS, SOCKET_ARG, SOCKET_ENV,
};
use foks_agent_proto::{Operation, Response};
use fs2::FileExt as _;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub(super) fn managed_agent_binary(socket: &Path) -> Option<PathBuf> {
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

pub(super) fn packaged_agent_candidate(executable: &Path) -> Option<PathBuf> {
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

pub(super) fn acquire_spawn_lock(socket: &Path) -> Result<File, AgentError> {
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

pub(super) fn validate_agent_binary(binary: &Path) -> Result<(), AgentError> {
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

pub(super) fn prepare_state_directory(directory: &Path) -> Result<(), AgentError> {
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

pub(super) fn managed_agent_arguments(state_dir: &Path, socket: &Path) -> [std::ffi::OsString; 6] {
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

pub(super) struct ManagedAgentLaunch {
    pid: u32,
    exit: mpsc::Receiver<std::io::Result<ExitStatus>>,
    log: File,
    log_offset: u64,
}

impl ManagedAgentLaunch {
    pub(super) fn early_exit_error(&mut self) -> Option<AgentError> {
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

pub(super) fn launch_agent(
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

/// Verify the captured process even after its listener has closed. Neither a
/// missing socket nor a successful signal is evidence of process termination.
pub(super) fn stop_exit_process(
    target: &StaleAgentProcess,
    force: bool,
    wait: Duration,
) -> Result<(), AgentError> {
    if process_has_exited(target.pid) {
        return Ok(());
    }
    if !agent_process_matches(target) {
        return Err(AgentError::new(
            "agent-exit-changed",
            "The agent process identity changed or could not be verified. It was not terminated.",
            true,
        ));
    }
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    if unsafe { libc::kill(target.pid as i32, signal) } != 0
        && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        return Err(AgentError::new(
            "agent-stop-failed",
            "Could not signal the FOKS agent process.",
            true,
        ));
    }
    let deadline = Instant::now() + wait;
    loop {
        if process_has_exited(target.pid) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AgentError::new(
                "agent-stop-failed",
                "FOKS Agent has not exited. Try again or force it to stop.",
                true,
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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
pub(super) fn agent_state_dir(arguments: &[OsString]) -> Option<PathBuf> {
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
pub(super) fn process_ids() -> std::io::Result<Vec<u32>> {
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
pub(super) fn process_ids() -> std::io::Result<Vec<u32>> {
    Ok(std::fs::read_dir("/proc")?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
pub(super) fn process_ids() -> std::io::Result<Vec<u32>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process enumeration is unavailable on this platform",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn process_owner_uid(pid: u32) -> std::io::Result<libc::uid_t> {
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
pub(super) fn process_owner_uid(pid: u32) -> std::io::Result<libc::uid_t> {
    use std::os::unix::fs::MetadataExt as _;
    Ok(std::fs::metadata(format!("/proc/{pid}"))?.uid())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
pub(super) fn process_owner_uid(_pid: u32) -> std::io::Result<libc::uid_t> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process ownership is unavailable on this platform",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn process_arguments(pid: u32) -> std::io::Result<Vec<OsString>> {
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
pub(super) fn process_arguments(pid: u32) -> std::io::Result<Vec<OsString>> {
    use std::os::unix::ffi::OsStringExt as _;

    Ok(std::fs::read(format!("/proc/{pid}/cmdline"))?
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| OsString::from_vec(argument.to_vec()))
        .collect())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
pub(super) fn process_arguments(_pid: u32) -> std::io::Result<Vec<OsString>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process arguments are unavailable on this platform",
    ))
}

#[cfg(unix)]
pub(super) fn process_has_exited(pid: u32) -> bool {
    let alive = unsafe { libc::kill(pid as i32, 0) };
    alive != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Whether two paths name the same file, by device and inode, so a bundle
/// reached through a symlink or a different prefix still matches its agent.
#[cfg(unix)]
pub(super) fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Whether `pid` has been reparented to init: the process that launched it
/// has exited, so nothing else supervises it.
#[cfg(unix)]
pub(super) fn process_is_orphaned(pid: u32) -> bool {
    process_parent_pid(pid).is_ok_and(|parent| parent == 1)
}

/// When `pid` started, in seconds since the Unix epoch.
#[cfg(unix)]
pub(super) fn process_start_time(pid: u32) -> std::io::Result<u64> {
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
pub(super) fn process_parent_pid(pid: u32) -> std::io::Result<u32> {
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
pub(super) fn incompatible_listener_is_gone(socket: &Path, pid: u32) -> bool {
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
pub(super) fn unix_peer_pid(stream: &std::os::unix::net::UnixStream) -> std::io::Result<u32> {
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
pub(super) fn process_executable_path(pid: u32) -> std::io::Result<PathBuf> {
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
pub(super) fn is_foks_agent_executable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "foks-agent" || name.starts_with("foks-agent (deleted)")
}

#[cfg(unix)]
pub(super) fn is_replaceable_agent_process(pid: u32) -> bool {
    if pid <= 1 || pid == std::process::id() {
        return false;
    }
    process_executable_path(pid).is_ok_and(|path| is_foks_agent_executable(&path))
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

pub(super) fn default_state_directory() -> Option<PathBuf> {
    foks_client_app::portability::selected_desktop_state_root().ok()
}

pub fn default_socket() -> Option<PathBuf> {
    Some(default_state_directory()?.join(DEFAULT_SOCKET_NAME))
}

impl AgentHandle {
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

    /// Starts and verifies the managed agent while the caller owns the
    /// exclusive maintenance reservation. This must never acquire the shared
    /// reservation again.
    pub(super) fn ensure_started_already_reserved(&self) -> Result<Response, AgentError> {
        self.start_already_reserved(&|_| {
            Err(AgentError::new(
            "agent-takeover-required",
            "Another FOKS version owns the agent socket. Relaunch FOKS to confirm replacing it.",
            false,
        ))
        })
    }

    pub(super) fn start_already_reserved(
        &self,
        confirm: &dyn Fn(&AgentTakeover) -> Result<bool, AgentError>,
    ) -> Result<Response, AgentError> {
        self.transport
            .require_not_exiting()
            .map_err(AgentError::from_desktop)?;
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

    pub(super) fn start_with_binary(
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
            let mut launch = self.launch_unless_exiting(binary, state_dir)?;
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

    pub(super) fn require_managed_endpoint(&self) -> Result<(), AgentError> {
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

    pub(super) fn stop_owned_managed_agent(&self) -> Result<(), AgentError> {
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

    /// Irreversible admission barrier. Existing mutations may settle, but no
    /// new transport call or recovery/startup may begin after confirmation.
    pub fn begin_exit(&self) {
        let _launch = self
            .exit_launch_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.transport.shutting_down.store(true, Ordering::Release);
        self.startup_gate.release();
    }

    pub(super) fn launch_unless_exiting(
        &self,
        binary: &Path,
        root: &Path,
    ) -> Result<ManagedAgentLaunch, AgentError> {
        let _launch = self
            .exit_launch_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.transport
            .require_not_exiting()
            .map_err(AgentError::from_desktop)?;
        launch_agent(binary, root, &self.socket)
    }

    pub fn stop_for_exit(&self) -> Result<(), AgentError> {
        self.stop_exit_targets(false)
    }

    pub fn force_stop_for_exit(&self) -> Result<(), AgentError> {
        self.stop_exit_targets(true)
    }

    pub(super) fn stop_exit_targets(&self, force: bool) -> Result<(), AgentError> {
        self.begin_exit();
        let mut targets = self
            .exit_targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if targets.is_none() {
            // Graceful shutdown lets active mutations settle first. Explicit
            // force-stop may interrupt them; the permanent launch barrier still
            // prevents startup/recovery from spawning after target capture.
            let _reservation = if force {
                None
            } else {
                Some(self.transport.reserve_for_maintenance(MAINTENANCE_RESERVATION_WAIT)
                    .ok_or_else(|| AgentError::new("agent-busy", "Outstanding agent operations are still settling. Try again or force the agent to stop.", true))?)
            };
            *targets = Some(self.discover_exit_targets()?);
        }
        for target in targets.as_ref().unwrap() {
            stop_exit_process(target, force, Duration::from_secs(5))?;
        }
        // A new process must never inherit consent intended for the captured
        // targets. Keep the desktop open if somebody else rebound the endpoint.
        if !self.discover_exit_targets()?.is_empty() {
            return Err(AgentError::new("agent-exit-replaced", "Another FOKS agent is using this app's local state. Stop it before retrying shutdown.", true));
        }
        self.clear_connection_failure();
        Ok(())
    }

    pub(super) fn discover_exit_targets(&self) -> Result<Vec<StaleAgentProcess>, AgentError> {
        let root = self
            .socket
            .parent()
            .ok_or_else(|| AgentError::unknown("The agent state directory is unavailable."))?;
        let targets = if root.exists() {
            matching_agent_processes(root).map_err(|error| {
                AgentError::new(
                    "agent-exit-inspection",
                    format!("Could not inspect FOKS agent processes: {error}"),
                    true,
                )
            })?
        } else {
            Vec::new()
        };
        let owned = *MANAGED_AGENT_PID
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if owned.is_some_and(|pid| {
            !process_has_exited(pid) && !targets.iter().any(|target| target.pid == pid)
        }) {
            return Err(AgentError::new("agent-exit-inspection", "The managed agent is still running but its identity could not be verified. Stop it before retrying shutdown.", true));
        }
        if let Some(peer) = inspect_takeover_target(&self.socket)? {
            if !targets.iter().any(|target| target.pid == peer.pid) {
                return Err(AgentError::new("agent-exit-inspection", "The agent endpoint belongs to a process outside this app's local state. Stop it before retrying shutdown.", true));
            }
        }
        Ok(targets)
    }

    pub(super) fn require_owned_managed_agent(&self) -> Result<(), AgentError> {
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
    pub(super) fn adopt_orphaned_agent(&self) {
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
    pub(super) fn claim_external_agent(&self) -> Result<(), AgentError> {
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

    pub(super) fn require_maintenance_stop_settled(&self) -> Result<(), AgentError> {
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
}
