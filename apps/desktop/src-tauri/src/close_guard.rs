//! One-way application teardown. Only verified agent exit permits desktop exit.

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
        pid: Option<u32>,
        unsent: usize,
    },
    Stopping {
        pid: Option<u32>,
        force: bool,
    },
    Finalizing,
    Failed {
        pid: Option<u32>,
        error: String,
    },
    ForceConfirmation {
        pid: Option<u32>,
        error: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExitAction {
    Cancel,
    StopAgent,
    Retry,
    ShowForce,
    CancelForce,
    ForceStop,
}

#[derive(Default)]
pub struct CloseGuard {
    unsent: AtomicUsize,
    asking: AtomicBool,
    renderer_ready: AtomicBool,
    state: Mutex<ExitState>,
}

impl CloseGuard {
    pub fn set_unsent(&self, count: usize) {
        self.unsent.store(count, Ordering::Relaxed);
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
        self.emit(app, state);
    }

    fn emit(&self, app: &tauri::AppHandle, state: ExitState) {
        if let Err(error) = app.emit(EXIT_STATE_EVENT, state) {
            self.renderer_ready.store(false, Ordering::Release);
            tracing::error!(%error, "Failed to publish application exit state");
        }
    }

    fn begin(self: &Arc<Self>, app: &tauri::AppHandle, agent: &Arc<AgentHandle>) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *state == ExitState::Finalizing {
            return false;
        }
        let repeated = *state != ExitState::Idle;
        if !repeated {
            // Missing endpoint/ownership information never authorizes exit.
            *state = ExitState::Decision {
                pid: agent.process_info().pid,
                unsent: self.unsent.load(Ordering::Relaxed),
            };
        }
        let snapshot = state.clone();
        drop(state);
        if !repeated {
            self.emit(app, snapshot);
        }
        // Repeating Cmd-Q supplies a native escape from a failed renderer.
        // Startup quits work before a renderer has subscribed at all.
        if repeated || !self.renderer_ready.load(Ordering::Acquire) {
            native_question(app, self, agent);
        }
        true
    }
}

fn transition(state: &ExitState, action: ExitAction) -> Result<ExitState, AgentError> {
    match (state, action) {
        (ExitState::Decision { .. }, ExitAction::Cancel) => Ok(ExitState::Idle),
        (ExitState::Decision { pid, .. }, ExitAction::StopAgent)
        | (ExitState::Failed { pid, .. }, ExitAction::Retry) => Ok(ExitState::Stopping {
            pid: *pid,
            force: false,
        }),
        (ExitState::Failed { pid, error }, ExitAction::ShowForce) => {
            Ok(ExitState::ForceConfirmation {
                pid: *pid,
                error: error.clone(),
            })
        }
        (ExitState::ForceConfirmation { pid, error }, ExitAction::CancelForce) => {
            Ok(ExitState::Failed {
                pid: *pid,
                error: error.clone(),
            })
        }
        (ExitState::ForceConfirmation { pid, .. }, ExitAction::ForceStop) => {
            Ok(ExitState::Stopping {
                pid: *pid,
                force: true,
            })
        }
        _ => Err(AgentError::new(
            "exit-state",
            "That exit action is no longer available. Shutdown cannot be cancelled once it starts.",
            false,
        )),
    }
}

fn apply_action(
    app: &tauri::AppHandle,
    guard: &Arc<CloseGuard>,
    agent: &Arc<AgentHandle>,
    action: ExitAction,
) -> Result<(), AgentError> {
    let mut state = guard
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let next = transition(&state, action)?;
    if matches!(next, ExitState::Stopping { .. }) {
        agent.begin_exit();
    }
    *state = next.clone();
    drop(state);
    guard.emit(app, next.clone());
    if let ExitState::Stopping { pid, force } = next {
        let app = app.clone();
        let guard = Arc::clone(guard);
        let agent = Arc::clone(agent);
        // Retain the native coordinator independently of renderer lifetime.
        tauri::async_runtime::spawn(async move {
            let worker = Arc::clone(&agent);
            let result = tauri::async_runtime::spawn_blocking(move || {
                if force {
                    worker.force_stop_for_exit()
                } else {
                    worker.stop_for_exit()
                }
            })
            .await
            .unwrap_or_else(|_| {
                Err(AgentError::unknown(
                    "The shutdown worker was interrupted. Try again.",
                ))
            });
            match result {
                Ok(()) => {
                    guard.publish(&app, ExitState::Finalizing);
                    app.exit(0);
                }
                Err(error) => {
                    guard.publish(
                        &app,
                        ExitState::Failed {
                            pid,
                            error: error.message,
                        },
                    );
                    if !guard.renderer_ready.load(Ordering::Acquire) {
                        native_question(&app, &guard, &agent);
                    }
                }
            }
        });
    }
    Ok(())
}

/// Native fallback shares the exact same transitions; it cannot bypass the
/// termination wait, even if the webview is unavailable or quits again.
fn native_question(app: &tauri::AppHandle, guard: &Arc<CloseGuard>, agent: &Arc<AgentHandle>) {
    let snapshot = guard.state();
    let (message, yes_label, no_label, yes, no) = match &snapshot {
        ExitState::Decision { unsent, .. } => (
            format!("Stop the FOKS agents using this app's local state and quit? Shutdown cannot be cancelled once it starts. {unsent} unsent messages will be discarded."),
            "Stop agent and quit", "Keep FOKS open", ExitAction::StopAgent, ExitAction::Cancel,
        ),
        ExitState::Failed { error, .. } => (
            format!("FOKS has not quit. {error}\nTry again, or review force-stop options."),
            "Try again", "Force stop…", ExitAction::Retry, ExitAction::ShowForce,
        ),
        ExitState::ForceConfirmation { .. } => (
            "Force stop FOKS Agent? Interrupted operations may need to be checked after restarting FOKS.".into(),
            "Force stop and quit", "Back", ExitAction::ForceStop, ExitAction::CancelForce,
        ),
        _ => return,
    };
    if guard.asking.swap(true, Ordering::AcqRel) {
        return;
    }
    let handle = app.clone();
    let guard = Arc::clone(guard);
    let agent = Arc::clone(agent);
    app.dialog()
        .message(message)
        .title("Quit FOKS")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            yes_label.into(),
            no_label.into(),
        ))
        .show(move |accepted| {
            guard.asking.store(false, Ordering::Release);
            // Ignore a stale native answer if the renderer already acted.
            if guard.state() != snapshot {
                return;
            }
            if apply_action(&handle, &guard, &agent, if accepted { yes } else { no }).is_ok()
                && matches!(
                    guard.state(),
                    ExitState::Failed { .. } | ExitState::ForceConfirmation { .. }
                )
            {
                native_question(&handle, &guard, &agent);
            }
        });
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
    guard.renderer_ready.store(true, Ordering::Release);
    Ok(guard.state())
}

#[tauri::command]
pub fn handle_exit_action(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    guard: State<'_, Arc<CloseGuard>>,
    state: State<'_, AppState>,
    action: ExitAction,
) -> Result<(), AgentError> {
    require_main_window(&webview)?;
    apply_action(&app, &guard, &state.agent, action)
}

pub fn observe(window: &tauri::WebviewWindow, guard: Arc<CloseGuard>) {
    let observed = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let app = observed.app_handle();
            if guard.begin(app, &app.state::<AppState>().agent) {
                api.prevent_close();
            }
        }
    });
}

pub fn request_app_exit(app: &tauri::AppHandle, guard: &Arc<CloseGuard>, agent: &Arc<AgentHandle>) {
    if !guard.begin(app, agent) {
        app.exit(0);
    }
}

pub fn intercept_exit_request(
    app: &tauri::AppHandle,
    guard: &Arc<CloseGuard>,
    agent: &Arc<AgentHandle>,
    _code: Option<i32>,
) -> bool {
    // Explicit exit codes, Cmd-Q and window closes all obey the same gate.
    guard.begin(app, agent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_is_irreversible_and_duplicate_actions_are_rejected() {
        let decision = ExitState::Decision {
            pid: None,
            unsent: 2,
        };
        assert_eq!(
            transition(&decision, ExitAction::Cancel).unwrap(),
            ExitState::Idle
        );
        let stopping = transition(&decision, ExitAction::StopAgent).unwrap();
        for action in [
            ExitAction::Cancel,
            ExitAction::StopAgent,
            ExitAction::Retry,
            ExitAction::ForceStop,
        ] {
            assert!(transition(&stopping, action).is_err());
        }
        let failed = ExitState::Failed {
            pid: None,
            error: "timeout".into(),
        };
        assert!(transition(&failed, ExitAction::Cancel).is_err());
        assert!(transition(&failed, ExitAction::ForceStop).is_err());
        let confirm = transition(&failed, ExitAction::ShowForce).unwrap();
        assert_eq!(
            transition(&confirm, ExitAction::CancelForce).unwrap(),
            failed
        );
        assert!(matches!(
            transition(&confirm, ExitAction::ForceStop).unwrap(),
            ExitState::Stopping { force: true, .. }
        ));
        assert!(transition(&ExitState::Finalizing, ExitAction::Cancel).is_err());
    }

    #[test]
    fn missing_process_information_is_explicit_in_renderer_contract() {
        assert_eq!(
            serde_json::to_value(ExitState::Decision {
                pid: None,
                unsent: 2
            })
            .unwrap(),
            serde_json::json!({"state":"decision", "pid":null, "unsent":2})
        );
    }
}
