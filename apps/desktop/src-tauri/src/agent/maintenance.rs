//! Exclusive maintenance, recovery, restoration, and confirmed agent takeover.

use super::process::{
    acquire_spawn_lock, incompatible_listener_is_gone, is_replaceable_agent_process,
    managed_agent_binary, prepare_managed_crash_directory, process_executable_path, unix_peer_pid,
    validate_agent_binary,
};
use super::{
    AgentError, AgentHandle, AgentTakeover, MaintenanceAdmission, MaintenanceCompletion,
    MaintenanceDisposition, MaintenanceKind, MaintenanceOperationOutcome, MaintenancePhase,
    MaintenanceProcess, MaintenanceSnapshot, MaintenanceWorker, NativeMaintenanceProcess,
    SafeRootDisposition, TransportDisposition, MAINTENANCE_RESERVATION_WAIT, MANAGED_AGENT_PID,
    MAX_STARTUP_TAKEOVERS,
};
use foks_agent_proto::Response;
use foks_desktop::AgentError as DesktopAgentError;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

impl MaintenanceSnapshot {
    pub(super) fn generation(&self) -> u64 {
        match self {
            Self::Idle { generation, .. }
            | Self::Active { generation, .. }
            | Self::Complete { generation, .. } => *generation,
        }
    }

    pub(super) fn set_revision(&mut self, revision: u64) {
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

    pub(super) fn transition_rank(&self) -> u8 {
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

impl Drop for MaintenanceAdmission<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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

pub(super) fn safe_selected_root(source: &Path, affected_roots: &[PathBuf]) -> SafeRootDisposition {
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

#[cfg(unix)]
pub(super) fn inspect_takeover_target(socket: &Path) -> Result<Option<AgentTakeover>, AgentError> {
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
pub(super) fn stop_incompatible_agent(
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

impl AgentHandle {
    pub fn auto_recover_blocking(&self) -> Result<Response, AgentError> {
        self.auto_recover_with_reservation_timeout(MAINTENANCE_RESERVATION_WAIT)
    }

    pub(super) fn auto_recover_with_reservation_timeout(
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
        let mut launch = self.launch_unless_exiting(&binary, root)?;
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

    pub(super) fn require_missing_agent_endpoint(&self) -> Result<(), AgentError> {
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

    pub(super) fn check_recovery_credentials(&self, interactive: bool) -> Result<(), AgentError> {
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

    pub(super) fn require_auto_recovery_root(&self) -> Result<(), AgentError> {
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

    /// Retries agent restoration while excluding ordinary commands. A failed
    /// maintenance restoration remains a typed local transport condition until
    /// this method has verified an authoritative agent status response.
    pub fn retry_started_blocking(&self) -> Result<Response, AgentError> {
        let _reservation = self
            .transport
            .maintenance
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.transport
            .require_not_exiting()
            .map_err(AgentError::from_desktop)?;
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

    pub fn maintenance_snapshot(&self) -> MaintenanceSnapshot {
        self.maintenance_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn publish_maintenance(
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
        self.transport
            .require_not_exiting()
            .map_err(AgentError::from_desktop)?;
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
}
