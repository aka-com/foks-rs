//! IPC transport, startup gating, connection observation, and error mapping.

use super::{
    AgentError, AgentErrorDetails, AgentHandle, LabelledTransport, MaintenanceDrain,
    ObservedTransport, StartupGate, TransportDisposition,
};
use crate::diagnostics::{agent_operation, outcome_of, Label, TimingLog};
use foks_agent_proto::{ErrorCode, ErrorFields, Operation, Response, ResponseResult};
use foks_desktop::{AgentError as DesktopAgentError, AgentTransport, IpcErrorCode};
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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

    pub(super) fn from_ipc(
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

impl Drop for MaintenanceDrain<'_> {
    fn drop(&mut self) {
        self.0.maintenance_pending.store(false, Ordering::Release);
    }
}

impl StartupGate {
    pub(super) fn new_open() -> Self {
        Self {
            open: Mutex::new(true),
            ready: Condvar::new(),
        }
    }

    pub(super) fn hold(&self) {
        *self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
    }

    pub(super) fn release(&self) {
        *self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.ready.notify_all();
    }

    pub(super) fn wait(&self) {
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

impl ObservedTransport {
    #[allow(clippy::result_large_err)] // Uses the existing transport trait error without allocating on success.
    pub(super) fn reserve_use(
        &self,
    ) -> Result<std::sync::RwLockReadGuard<'_, ()>, DesktopAgentError> {
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
    pub(super) fn preempted(&self, preemptible: bool) -> bool {
        preemptible
            && (self.maintenance_pending.load(Ordering::Acquire)
                || self.shutting_down.load(Ordering::Acquire))
    }

    /// Takes the exclusive reservation that maintenance runs under, asking
    /// in-flight preemptible calls to stop first. Returns `None` if calls were
    /// still outstanding when the wait ran out.
    pub(super) fn reserve_for_maintenance(
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
    pub(super) fn require_not_exiting(&self) -> Result<(), DesktopAgentError> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(DesktopAgentError::Local(
                foks_desktop::LocalAgentCondition::RestartRequired,
            ));
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    pub(super) fn require_current(&self) -> Result<(), DesktopAgentError> {
        self.require_not_exiting()?;
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

    pub(super) fn record<T>(&self, result: &Result<T, DesktopAgentError>) {
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
    pub(super) fn observe(
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
    pub(super) fn call_observed(
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
    pub(super) fn stream_observed(
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

pub(super) fn client_to_desktop(error: foks_agent_client::Error) -> DesktopAgentError {
    foks_desktop::agent_client_error(error)
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

impl AgentHandle {
    /// Holds ordinary commands until `release_startup` runs. Call before the
    /// webview can issue its first command.
    pub fn hold_commands_for_startup(&self) {
        self.startup_gate.hold();
    }

    /// Opens the startup gate and wakes every command waiting on it.
    pub fn release_startup(&self) {
        self.startup_gate.release();
    }

    pub(super) fn wait_for_startup(&self) {
        self.startup_gate.wait();
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

    pub(super) fn call_unreserved(&self, operation: Operation) -> Result<Response, AgentError> {
        self.transport
            .client
            .call(operation)
            .map_err(|error| AgentError::from_client(&error))
    }

    pub(super) fn probe_status(&self) -> Result<Response, AgentError> {
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

    pub(super) fn clear_connection_failure(&self) {
        *self
            .connection_failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}
