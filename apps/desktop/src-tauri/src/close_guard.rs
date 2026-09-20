//! Confirmation before the application discards chat messages it is the only
//! copy of.
//!
//! The renderer's chat send queue is held in memory: a message waiting in it
//! has not been written anywhere the next session could read, so ending the
//! process loses it. The renderer reports how many such submissions it holds,
//! and this module turns closing the window — and quitting the application —
//! into a question for as long as that count is above zero. It is a prompt,
//! not an access control: the count is the renderer's own statement about its
//! queue, and nothing here reads or writes chat state.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tauri::{Manager as _, State, WindowEvent};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::agent::AgentError;

/// The unsent-message count a departure is answered against.
#[derive(Default)]
pub struct CloseGuard {
    unsent: AtomicUsize,
    /// Set once the user accepted that the messages reported so far may be
    /// lost, so the close that answer permits is not asked about again on its
    /// way out. A later report of its own replaces it.
    accepted: AtomicBool,
    /// Whether a question is already on screen, so a repeated close request
    /// does not stack a second dialog on the first.
    asking: AtomicBool,
}

impl CloseGuard {
    /// Records what the renderer says it is holding. A window still reporting
    /// messages is a departure that has not been answered yet, whatever an
    /// earlier window's answer was.
    pub fn set_unsent(&self, count: usize) {
        self.unsent.store(count, Ordering::Relaxed);
        if count > 0 {
            self.accepted.store(false, Ordering::Relaxed);
        }
    }

    /// How many messages closing would lose, or none once that was accepted.
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

    /// Claims the right to put a question on screen.
    fn begin(&self) -> bool {
        self.asking
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn end(&self) {
        self.asking.store(false, Ordering::Relaxed);
    }
}

#[tauri::command]
pub fn set_unsent_messages(
    webview: tauri::Webview,
    guard: State<'_, Arc<CloseGuard>>,
    count: u32,
) -> Result<(), AgentError> {
    crate::commands::require_main_window(&webview)?;
    guard.set_unsent(count as usize);
    Ok(())
}

fn question(unsent: usize) -> String {
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

/// Asks whether the `unsent` messages the caller found may be lost, and runs
/// `proceed` if they may. The caller has already held its departure back; a
/// refusal simply leaves it held. The count is the caller's, so a report that
/// lands while the question is open cannot turn the question into silence.
fn ask(
    app: &tauri::AppHandle,
    guard: &Arc<CloseGuard>,
    unsent: usize,
    proceed: impl FnOnce() + Send + 'static,
) {
    if !guard.begin() {
        return;
    }
    let guard = Arc::clone(guard);
    app.dialog()
        .message(question(unsent))
        .kind(MessageDialogKind::Warning)
        .title("Unsent messages")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Close anyway".to_owned(),
            "Keep FOKS open".to_owned(),
        ))
        .show(move |discard| {
            guard.end();
            if discard {
                guard.accept();
                proceed();
            }
        });
}

/// Holds the window's close while the renderer's queue has messages in it.
pub fn observe(window: &tauri::WebviewWindow, guard: Arc<CloseGuard>) {
    let observed = window.clone();
    window.on_window_event(move |event| {
        let WindowEvent::CloseRequested { api, .. } = event else {
            return;
        };
        let unsent = guard.unsent();
        if unsent == 0 {
            return;
        }
        api.prevent_close();
        let closing = observed.clone();
        ask(observed.app_handle(), &guard, unsent, move || {
            if let Err(error) = closing.close() {
                tracing::error!(%error, "Failed to close the window after the unsent-message confirmation");
            }
        });
    });
}

/// How many unsent messages this exit would discard. An exit the application
/// itself requested carries its code and is never turned into a question.
pub fn unsent_at_exit(guard: &Arc<CloseGuard>, code: Option<i32>) -> usize {
    if code.is_some() {
        0
    } else {
        guard.unsent()
    }
}

/// Asks about an exit the caller held back, and exits if it is allowed to.
pub fn ask_before_exit(app: &tauri::AppHandle, guard: &Arc<CloseGuard>, unsent: usize) {
    let exiting = app.clone();
    ask(app, guard, unsent, move || exiting.exit(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accepted_departure_is_not_asked_about_again() {
        let guard = Arc::new(CloseGuard::default());
        assert_eq!(guard.unsent(), 0);
        assert_eq!(unsent_at_exit(&guard, None), 0);
        guard.set_unsent(3);
        assert_eq!(unsent_at_exit(&guard, None), 3);
        // An exit the application itself asked for is never a question.
        assert_eq!(unsent_at_exit(&guard, Some(0)), 0);
        guard.accept();
        assert_eq!(guard.unsent(), 0);
        assert_eq!(unsent_at_exit(&guard, None), 0);
    }

    #[test]
    fn a_window_still_holding_messages_is_asked_about_again() {
        let guard = CloseGuard::default();
        guard.set_unsent(2);
        guard.accept();
        assert_eq!(guard.unsent(), 0);
        // An emptied queue does not revive the answer; a filled one does, so
        // a second window's messages are not covered by the first's answer.
        guard.set_unsent(0);
        assert_eq!(guard.unsent(), 0);
        guard.set_unsent(1);
        assert_eq!(guard.unsent(), 1);
    }

    #[test]
    fn only_one_question_is_on_screen_at_a_time() {
        let guard = CloseGuard::default();
        assert!(guard.begin());
        assert!(!guard.begin());
        guard.end();
        assert!(guard.begin());
    }

    #[test]
    fn the_question_counts_the_messages_it_names() {
        assert!(question(1).starts_with("1 message has not been sent"));
        assert!(question(4).starts_with("4 messages have not been sent"));
    }
}
