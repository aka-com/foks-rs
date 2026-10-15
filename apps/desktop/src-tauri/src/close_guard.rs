//! One-way application teardown. Only verified agent exit permits desktop exit.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::{Emitter as _, Manager as _, State, WindowEvent};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::agent::{AgentError, AgentHandle, MAINTENANCE_RESERVATION_WAIT};
use crate::commands::{require_main_window, AppState};

pub const EXIT_STATE_EVENT: &str = "foks://exit-state";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ExitState {
    #[default]
    Idle,
    /// The first close or quit request, with what it found. `unsent`,
    /// `mutation`, `maintenance` and `owned == false` are each a reason to ask
    /// before stopping the agent; see [`QuitReasons::asks_before_stopping`].
    Decision {
        pid: Option<u32>,
        /// Chat messages queued in the renderer and not yet sent.
        unsent: usize,
        /// A command holds a desktop mutation reservation.
        mutation: bool,
        /// State maintenance or automatic recovery holds admission.
        maintenance: bool,
        /// This app launched or adopted the agent answering on the socket,
        /// and no other agent process uses this app's state directory. False
        /// when the socket owner cannot be identified.
        owned: bool,
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

/// The device-local Ask before quitting preference, which the renderer
/// stores and reports here. Until it reports one, every request asks.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum QuitConfirmation {
    Always,
    /// Ask only when a request finds a reason in its decision; otherwise stop
    /// the agent and quit without asking.
    WhenPending,
}

#[derive(Default)]
pub struct CloseGuard {
    unsent: AtomicUsize,
    when_pending: AtomicBool,
    asking: AtomicBool,
    renderer_ready: AtomicBool,
    state: Mutex<ExitState>,
}

/// What a first close or quit request finds; the fields are those of
/// [`ExitState::Decision`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QuitReasons {
    pid: Option<u32>,
    unsent: usize,
    mutation: bool,
    maintenance: bool,
    owned: bool,
}

impl QuitReasons {
    /// Whether the decision must be shown before shutdown starts. A request
    /// that finds no reason skips it only when the preference allows that.
    fn asks_before_stopping(&self, confirmation: QuitConfirmation) -> bool {
        confirmation == QuitConfirmation::Always
            || self.unsent > 0
            || self.mutation
            || self.maintenance
            || !self.owned
    }
}

impl From<QuitReasons> for ExitState {
    fn from(reasons: QuitReasons) -> Self {
        ExitState::Decision {
            pid: reasons.pid,
            unsent: reasons.unsent,
            mutation: reasons.mutation,
            maintenance: reasons.maintenance,
            owned: reasons.owned,
        }
    }
}

impl CloseGuard {
    pub fn set_unsent(&self, count: usize) {
        self.unsent.store(count, Ordering::Relaxed);
    }

    pub fn set_confirmation(&self, confirmation: QuitConfirmation) {
        self.when_pending.store(
            confirmation == QuitConfirmation::WhenPending,
            Ordering::Relaxed,
        );
    }

    fn confirmation(&self) -> QuitConfirmation {
        if self.when_pending.load(Ordering::Relaxed) {
            QuitConfirmation::WhenPending
        } else {
            QuitConfirmation::Always
        }
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
        // decision() may read the whole process table, so it runs before the
        // state lock is taken. A request that then finds the state no longer
        // Idle discards the result.
        let found = (self.state() == ExitState::Idle)
            .then(|| decision(app, agent, self.unsent.load(Ordering::Relaxed)));
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *state == ExitState::Finalizing {
            return false;
        }
        let repeated = *state != ExitState::Idle;
        if !repeated {
            let reasons =
                found.unwrap_or_else(|| decision(app, agent, self.unsent.load(Ordering::Relaxed)));
            // A request that finds nothing to interrupt starts shutdown at
            // once when the preference allows it. The same Stopping state
            // follows, so a failure still reaches the Failed questions.
            if !reasons.asks_before_stopping(self.confirmation()) {
                enter(
                    app,
                    self,
                    agent,
                    state,
                    ExitState::Stopping {
                        pid: reasons.pid,
                        force: false,
                    },
                );
                return true;
            }
            *state = reasons.into();
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

/// What a first request finds. Missing endpoint or ownership information,
/// and a failed process scan, read as an agent this app does not own, so they
/// never skip the question. The scan runs only for an owned socket peer,
/// since it cannot make any other agent count as owned.
fn decision(app: &tauri::AppHandle, agent: &AgentHandle, unsent: usize) -> QuitReasons {
    let info = agent.process_info();
    let owned = info.owned
        && agent.state_dir_agent_processes().is_ok_and(|processes| {
            processes
                .iter()
                .all(|process| Some(process.pid) == info.pid)
        });
    QuitReasons {
        pid: info.pid,
        unsent,
        mutation: app
            .try_state::<AppState>()
            .is_none_or(|state| state.mutation_in_progress()),
        maintenance: agent.maintenance_in_progress(),
        owned,
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
    let state = guard
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let next = transition(&state, action)?;
    enter(app, guard, agent, state, next);
    Ok(())
}

/// Store and publish `next` under the lock that admitted it, and start the
/// shutdown worker when `next` is [`ExitState::Stopping`].
fn enter(
    app: &tauri::AppHandle,
    guard: &Arc<CloseGuard>,
    agent: &Arc<AgentHandle>,
    mut state: MutexGuard<'_, ExitState>,
    next: ExitState,
) {
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
}

/// Native fallback shares the exact same transitions; it cannot bypass the
/// termination wait, even if the webview is unavailable or quits again.
fn native_question(app: &tauri::AppHandle, guard: &Arc<CloseGuard>, agent: &Arc<AgentHandle>) {
    let snapshot = guard.state();
    let (message, yes_label, no_label, yes, no) = match &snapshot {
        ExitState::Decision { .. } => (
            decision_message(&snapshot),
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

/// The native fallback's decision text. It names only the reasons present.
fn decision_message(decision: &ExitState) -> String {
    let mut message = String::from("Stop the FOKS agents using this app's local state and quit? Shutdown cannot be cancelled once it starts.");
    let ExitState::Decision {
        pid,
        unsent,
        mutation,
        maintenance,
        owned,
    } = decision
    else {
        return message;
    };
    match unsent {
        0 => {}
        1 => message.push_str(" 1 unsent message will be discarded."),
        count => message.push_str(&format!(" {count} unsent messages will be discarded.")),
    }
    if *mutation {
        message.push_str(
            " A change is in progress. Shutdown starts none of its remaining agent requests.",
        );
    }
    if *maintenance {
        message.push_str(" Maintenance of this device's FOKS data is in progress.");
    }
    if *mutation || *maintenance {
        message.push_str(&format!(
            " Shutdown waits up to {} seconds for the current work to finish. If it has not finished by then, FOKS stays open and offers Try again or Force stop.",
            MAINTENANCE_RESERVATION_WAIT.as_secs()
        ));
    }
    if pid.is_none() {
        message.push_str(" FOKS could not identify the agent process.");
    } else if !*owned {
        message.push_str(" Quitting also stops an agent this app did not start, which other FOKS clients on this device may be using.");
    }
    message
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
pub fn set_quit_confirmation(
    webview: tauri::Webview,
    guard: State<'_, Arc<CloseGuard>>,
    confirmation: QuitConfirmation,
) -> Result<(), AgentError> {
    require_main_window(&webview)?;
    guard.set_confirmation(confirmation);
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

    fn quiet_reasons() -> QuitReasons {
        QuitReasons {
            pid: Some(4242),
            unsent: 0,
            mutation: false,
            maintenance: false,
            owned: true,
        }
    }

    #[test]
    fn teardown_is_irreversible_and_duplicate_actions_are_rejected() {
        let decision = ExitState::Decision {
            pid: None,
            unsent: 2,
            mutation: false,
            maintenance: false,
            owned: false,
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
                unsent: 2,
                mutation: true,
                maintenance: false,
                owned: false,
            })
            .unwrap(),
            serde_json::json!({
                "state": "decision",
                "pid": null,
                "unsent": 2,
                "mutation": true,
                "maintenance": false,
                "owned": false,
            })
        );
    }

    #[test]
    fn quit_confirmation_accepts_only_the_renderer_values() {
        assert_eq!(
            serde_json::from_value::<QuitConfirmation>(serde_json::json!("always")).unwrap(),
            QuitConfirmation::Always
        );
        assert_eq!(
            serde_json::from_value::<QuitConfirmation>(serde_json::json!("when-pending")).unwrap(),
            QuitConfirmation::WhenPending
        );
        assert!(serde_json::from_value::<QuitConfirmation>(serde_json::json!("never")).is_err());
        let guard = CloseGuard::default();
        assert_eq!(guard.confirmation(), QuitConfirmation::Always);
        guard.set_confirmation(QuitConfirmation::WhenPending);
        assert_eq!(guard.confirmation(), QuitConfirmation::WhenPending);
        guard.set_confirmation(QuitConfirmation::Always);
        assert_eq!(guard.confirmation(), QuitConfirmation::Always);
    }

    #[test]
    fn only_a_decision_without_reasons_is_skipped_and_only_when_allowed() {
        assert!(quiet_reasons().asks_before_stopping(QuitConfirmation::Always));
        assert!(!quiet_reasons().asks_before_stopping(QuitConfirmation::WhenPending));
        let reasons = [
            QuitReasons {
                unsent: 1,
                ..quiet_reasons()
            },
            QuitReasons {
                mutation: true,
                ..quiet_reasons()
            },
            QuitReasons {
                maintenance: true,
                ..quiet_reasons()
            },
            // An agent that a CLI or another launch started may be shared.
            QuitReasons {
                owned: false,
                ..quiet_reasons()
            },
            // An unidentified socket owner is never treated as this app's.
            QuitReasons {
                pid: None,
                owned: false,
                ..quiet_reasons()
            },
        ];
        for reasons in &reasons {
            assert!(
                reasons.asks_before_stopping(QuitConfirmation::WhenPending),
                "{reasons:?}"
            );
        }
    }

    #[test]
    fn native_decision_names_only_the_reasons_present() {
        let quiet = decision_message(&quiet_reasons().into());
        assert!(!quiet.contains("unsent"), "{quiet}");
        assert!(!quiet.contains("in progress"), "{quiet}");
        assert!(!quiet.contains("did not start"), "{quiet}");
        let one = decision_message(
            &QuitReasons {
                unsent: 1,
                ..quiet_reasons()
            }
            .into(),
        );
        assert!(
            one.contains(" 1 unsent message will be discarded."),
            "{one}"
        );
        let busy = decision_message(
            &QuitReasons {
                unsent: 3,
                mutation: true,
                maintenance: true,
                owned: false,
                ..quiet_reasons()
            }
            .into(),
        );
        for part in [
            " 3 unsent messages will be discarded.",
            " Maintenance of this device's FOKS data is in progress.",
            " A change is in progress. Shutdown starts none of its remaining agent requests.",
            " Shutdown waits up to 5 seconds for the current work to finish.",
            " FOKS stays open and offers Try again or Force stop.",
            " Quitting also stops an agent this app did not start",
        ] {
            assert!(busy.contains(part), "{busy}");
        }
        let unknown = decision_message(
            &QuitReasons {
                pid: None,
                owned: false,
                ..quiet_reasons()
            }
            .into(),
        );
        assert!(
            unknown.contains("could not identify the agent process"),
            "{unknown}"
        );
        assert!(!unknown.contains("did not start"), "{unknown}");
    }
}
