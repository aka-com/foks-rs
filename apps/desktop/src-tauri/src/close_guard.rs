//! Coordinates application exit with the renderer's volatile chat queue and
//! the local agent process owned by this desktop launch.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter as _, Manager as _, State, WindowEvent};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::agent::{AgentError, AgentHandle};
use crate::commands::{require_main_window, AppState};

pub const EXIT_STATE_EVENT: &str = "foks://exit-state";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ExitState {
    #[default]
    Idle,
    Decision {
        pid: u32,
        unsent: usize,
    },
    Stopping {
        pid: u32,
        force: bool,
    },
    Finalizing,
    Failed {
        pid: u32,
        error: String,
    },
    ForceConfirmation {
        pid: u32,
        error: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExitAction {
    Cancel,
    LeaveRunning,
    StopAgent,
    Retry,
    ShowForce,
    CancelForce,
    ForceStop,
}

/// The renderer's volatile-message report and the single application-wide
/// exit workflow. Multiple close/quit signals share this state, preventing
/// stacked prompts and concurrent attempts to stop the same process.
pub struct CloseGuard {
    unsent: AtomicUsize,
    accepted: AtomicBool,
    asking: AtomicBool,
    leave_agent_running: AtomicBool,
    state: Mutex<ExitState>,
}

impl Default for CloseGuard {
    fn default() -> Self {
        Self {
            unsent: AtomicUsize::new(0),
            accepted: AtomicBool::new(false),
            asking: AtomicBool::new(false),
            leave_agent_running: AtomicBool::new(false),
            state: Mutex::new(ExitState::Idle),
        }
    }
}

impl CloseGuard {
    pub fn set_unsent(&self, count: usize) {
        self.unsent.store(count, Ordering::Relaxed);
        if count > 0 {
            self.accepted.store(false, Ordering::Relaxed);
        }
    }

    pub fn unsent(&self) -> usize {
        if self.accepted.load(Ordering::Relaxed) {
            0
        } else {
            self.unsent.load(Ordering::Relaxed)
        }
    }

    fn accept(&self) {
        self.accepted.store(true, Ordering::Relaxed);
    }

    fn begin_native_question(&self) -> bool {
        self.asking
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn end_native_question(&self) {
        self.asking.store(false, Ordering::Relaxed);
    }

    pub fn state(&self) -> ExitState {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn publish(&self, app: &tauri::AppHandle, state: ExitState) {
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = state.clone();
        if let Err(error) = app.emit(EXIT_STATE_EVENT, state) {
            tracing::error!(%error, "Failed to publish the application exit state");
        }
    }

    /// Preserve historical agent cleanup for restarts and fatal exits. The
    /// exception is the reader's explicit choice to leave it running.
    pub fn terminate_agent_on_exit(&self) -> bool {
        !self.leave_agent_running.load(Ordering::Acquire)
    }

    fn begin_owned_agent_exit(&self, app: &tauri::AppHandle, agent: &AgentHandle) -> bool {
        if self.state() != ExitState::Idle {
            return true;
        }
        let process = agent.process_info();
        let Some(pid) = process.pid.filter(|_| process.owned) else {
            return false;
        };
        self.publish(
            app,
            ExitState::Decision {
                pid,
                unsent: self.unsent(),
            },
        );
        true
    }
}

#[tauri::command]
pub fn set_unsent_messages(
    webview: tauri::Webview,
    guard: State<'_, Arc<CloseGuard>>,
    count: u32,
) -> Result<(), AgentError> {
    require_main_window(&webview)?;
    guard.set_unsent(count as usize);
    Ok(())
}

#[tauri::command]
pub fn exit_state(
    webview: tauri::Webview,
    guard: State<'_, Arc<CloseGuard>>,
) -> Result<ExitState, AgentError> {
    require_main_window(&webview)?;
    Ok(guard.state())
}

fn unsent_question(unsent: usize) -> String {
    if unsent == 1 {
        "1 message has not been sent yet. It is only on this device, and closing FOKS discards it."
            .to_owned()
    } else {
        format!(
            "{unsent} messages have not been sent yet. They are only on this device, and closing \
             FOKS discards them."
        )
    }
}

/// The unsent-only fallback remains native. The richer in-app decision is
/// reserved for a process this launch can actually stop.
fn ask_about_unsent(
    app: &tauri::AppHandle,
    guard: &Arc<CloseGuard>,
    unsent: usize,
    proceed: impl FnOnce() + Send + 'static,
) {
    if !guard.begin_native_question() {
        return;
    }
    let guard = Arc::clone(guard);
    app.dialog()
        .message(unsent_question(unsent))
        .kind(MessageDialogKind::Warning)
        .title("Unsent messages")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Close anyway".to_owned(),
            "Keep FOKS open".to_owned(),
        ))
        .show(move |discard| {
            guard.end_native_question();
            if discard {
                guard.accept();
                proceed();
            }
        });
}

fn finish_exit(app: &tauri::AppHandle, guard: &Arc<CloseGuard>, leave_agent_running: bool) {
    guard.accept();
    guard
        .leave_agent_running
        .store(leave_agent_running, Ordering::Release);
    guard.publish(app, ExitState::Finalizing);
    app.exit(0);
}

fn current_pid_for_stop(state: &ExitState, force: bool) -> Option<u32> {
    match (state, force) {
        (ExitState::Decision { pid, .. }, false) | (ExitState::Failed { pid, .. }, false) => {
            Some(*pid)
        }
        (ExitState::ForceConfirmation { pid, .. }, true) => Some(*pid),
        _ => None,
    }
}

async fn stop_and_exit(
    app: tauri::AppHandle,
    guard: Arc<CloseGuard>,
    agent: Arc<AgentHandle>,
    force: bool,
) -> Result<(), AgentError> {
    let pid = current_pid_for_stop(&guard.state(), force).ok_or_else(|| {
        AgentError::new(
            "exit-state",
            "That exit action is no longer available.",
            true,
        )
    })?;
    guard.publish(&app, ExitState::Stopping { pid, force });
    let result = tauri::async_runtime::spawn_blocking(move || {
        if force {
            agent.force_stop_for_exit()
        } else {
            agent.stop_for_exit()
        }
    })
    .await
    .map_err(|_| AgentError::unknown("The agent shutdown worker was interrupted."))?;
    match result {
        Ok(()) => finish_exit(&app, &guard, false),
        Err(error) => guard.publish(
            &app,
            ExitState::Failed {
                pid,
                error: error.message,
            },
        ),
    }
    Ok(())
}

#[tauri::command]
pub async fn handle_exit_action(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    guard: State<'_, Arc<CloseGuard>>,
    state: State<'_, AppState>,
    action: ExitAction,
) -> Result<(), AgentError> {
    require_main_window(&webview)?;
    let guard = Arc::clone(&guard);
    match action {
        ExitAction::Cancel => {
            guard.publish(&app, ExitState::Idle);
            Ok(())
        }
        ExitAction::LeaveRunning => {
            if !matches!(
                guard.state(),
                ExitState::Decision { .. } | ExitState::Failed { .. }
            ) {
                return Err(AgentError::new(
                    "exit-state",
                    "That exit action is no longer available.",
                    true,
                ));
            }
            finish_exit(&app, &guard, true);
            Ok(())
        }
        ExitAction::StopAgent | ExitAction::Retry => {
            stop_and_exit(app, guard, Arc::clone(&state.agent), false).await
        }
        ExitAction::ShowForce => {
            let ExitState::Failed { pid, error } = guard.state() else {
                return Err(AgentError::new(
                    "exit-state",
                    "That exit action is no longer available.",
                    true,
                ));
            };
            guard.publish(&app, ExitState::ForceConfirmation { pid, error });
            Ok(())
        }
        ExitAction::CancelForce => {
            let ExitState::ForceConfirmation { pid, error } = guard.state() else {
                return Err(AgentError::new(
                    "exit-state",
                    "That exit action is no longer available.",
                    true,
                ));
            };
            guard.publish(&app, ExitState::Failed { pid, error });
            Ok(())
        }
        ExitAction::ForceStop => stop_and_exit(app, guard, Arc::clone(&state.agent), true).await,
    }
}

/// Holds a window close while either the owned-agent decision or the existing
/// unsent-message question is active.
pub fn observe(window: &tauri::WebviewWindow, guard: Arc<CloseGuard>) {
    let observed = window.clone();
    window.on_window_event(move |event| {
        let WindowEvent::CloseRequested { api, .. } = event else {
            return;
        };
        let app = observed.app_handle();
        let agent = Arc::clone(&app.state::<AppState>().agent);
        if guard.begin_owned_agent_exit(app, &agent) {
            api.prevent_close();
            return;
        }
        let unsent = guard.unsent();
        if unsent == 0 {
            return;
        }
        api.prevent_close();
        let closing = observed.clone();
        ask_about_unsent(app, &guard, unsent, move || {
            if let Err(error) = closing.close() {
                tracing::error!(%error, "Failed to close the window after the unsent-message confirmation");
            }
        });
    });
}

/// Handles an application-wide user quit, including commands from the webview.
pub fn request_app_exit(app: &tauri::AppHandle, guard: &Arc<CloseGuard>, agent: &AgentHandle) {
    if guard.begin_owned_agent_exit(app, agent) {
        return;
    }
    let unsent = guard.unsent();
    if unsent > 0 {
        let exiting = app.clone();
        ask_about_unsent(app, guard, unsent, move || exiting.exit(0));
    } else {
        app.exit(0);
    }
}

/// Returns whether an OS/menu quit was intercepted for a decision.
pub fn intercept_exit_request(
    app: &tauri::AppHandle,
    guard: &Arc<CloseGuard>,
    agent: &AgentHandle,
    code: Option<i32>,
) -> bool {
    if code.is_some() {
        return false;
    }
    if guard.begin_owned_agent_exit(app, agent) {
        return true;
    }
    let unsent = guard.unsent();
    if unsent == 0 {
        return false;
    }
    let exiting = app.clone();
    ask_about_unsent(app, guard, unsent, move || exiting.exit(0));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accepted_departure_no_longer_reports_unsent_messages() {
        let guard = CloseGuard::default();
        guard.set_unsent(3);
        assert_eq!(guard.unsent(), 3);
        guard.accept();
        assert_eq!(guard.unsent(), 0);
    }

    #[test]
    fn a_later_queue_report_requires_a_new_answer() {
        let guard = CloseGuard::default();
        guard.set_unsent(2);
        guard.accept();
        guard.set_unsent(0);
        assert_eq!(guard.unsent(), 0);
        guard.set_unsent(1);
        assert_eq!(guard.unsent(), 1);
    }

    #[test]
    fn only_one_native_question_is_on_screen_at_a_time() {
        let guard = CloseGuard::default();
        assert!(guard.begin_native_question());
        assert!(!guard.begin_native_question());
        guard.end_native_question();
        assert!(guard.begin_native_question());
    }

    #[test]
    fn the_unsent_question_counts_the_messages_it_names() {
        assert!(unsent_question(1).starts_with("1 message has not been sent"));
        assert!(unsent_question(4).starts_with("4 messages have not been sent"));
    }

    #[test]
    fn only_valid_states_supply_a_pid_for_stopping() {
        let decision = ExitState::Decision { pid: 42, unsent: 0 };
        let force = ExitState::ForceConfirmation {
            pid: 42,
            error: "timed out".into(),
        };
        assert_eq!(current_pid_for_stop(&decision, false), Some(42));
        assert_eq!(current_pid_for_stop(&decision, true), None);
        assert_eq!(current_pid_for_stop(&force, true), Some(42));
    }

    #[test]
    fn exit_states_have_the_renderer_contract_shape() {
        assert_eq!(
            serde_json::to_value(ExitState::Decision { pid: 42, unsent: 2 }).unwrap(),
            serde_json::json!({"state":"decision", "pid":42, "unsent":2})
        );
        assert_eq!(
            serde_json::to_value(ExitState::Stopping {
                pid: 42,
                force: true,
            })
            .unwrap(),
            serde_json::json!({"state":"stopping", "pid":42, "force":true})
        );
    }
}
