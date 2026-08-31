//! Startup failures a person can act on are presented as a dialog, not a crash.
//!
//! Ported from `src-tauri/src/lib.rs`'s `fatal_startup`. The shape is the same
//! — a blocking native dialog, then a non-zero exit through the standard path
//! so no crash report is filed — but the failure set is FOKS's: there is one
//! thing this app cannot start without, and that is a reachable agent.

use std::path::Path;

use tauri_plugin_dialog::{DialogExt as _, MessageDialogKind};

use crate::agent::{AgentError, AgentHandle};

/// Shows a blocking dialog and exits non-zero.
///
/// Called from the setup hook, which is a nounwind context: returning an `Err`
/// there aborts the process with no explanation, which is precisely the outcome
/// this function exists to avoid.
pub fn fatal_startup(app: &tauri::App, title: &str, body: &str) -> ! {
    app.dialog()
        .message(body)
        .kind(MessageDialogKind::Error)
        .title(title)
        .blocking_show();
    std::process::exit(1);
}

/// Confirms the agent is reachable before the first request.
///
/// Two failures, one message each. A missing socket is the common one — the
/// agent is not running — and is named separately from a socket that exists but
/// does not answer, because the remedies differ.
pub fn require_agent(app: &tauri::App, agent: &AgentHandle) {
    let socket = agent.socket();
    if let Err(error) = agent.ensure_started_blocking() {
        let body = if !socket.exists() {
            missing_socket(socket)
        } else {
            unreachable_agent(socket, &error)
        };
        fatal_startup(app, "FOKS could not reach its agent", &body);
    }
    if let Err(error) = agent.call_blocking(foks_agent_proto::Operation::Ping) {
        fatal_startup(
            app,
            "FOKS could not reach its agent",
            &unreachable_agent(socket, &error),
        );
    }
}

fn missing_socket(socket: &Path) -> String {
    format!(
        "FOKS could not reach its agent at {}.\n\nThe socket does not exist, \
         which usually means foks-agent is not running. Start the agent, then \
         relaunch FOKS.",
        socket.display()
    )
}

fn unreachable_agent(socket: &Path, error: &AgentError) -> String {
    format!(
        "FOKS could not reach its agent at {}.\n\n{}\n\nFix the underlying \
         problem, then relaunch FOKS.",
        socket.display(),
        error.message
    )
}

/// The state-tampered variant of the same dialog.
///
/// AKA raises this when its on-disk state fails an integrity check, because an
/// app-identity change is security-relevant and must be reported rather than
/// crashed through. FOKS has no such check yet — the agent owns its own state —
/// so the copy is written and the call site is wired in Phase 5, alongside the
/// app lock. It is kept here so that work adds a caller rather than a policy.
#[allow(dead_code)]
pub fn fatal_state_tampered(app: &tauri::App, file: &Path) -> ! {
    fatal_startup(
        app,
        "FOKS state has been altered",
        &format!(
            "FOKS found unexpected changes to {}.\n\nThis is what an \
             app-identity change looks like, and it is also what tampering \
             looks like. FOKS will not use this state. Reinstall FOKS, or set \
             it up again from a device you trust.",
            file.display()
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_missing_socket_message_names_the_path_and_the_remedy() {
        let message = missing_socket(Path::new("/run/user/1000/foks-rs/agent.sock"));
        assert!(
            message.contains("FOKS could not reach its agent at /run/user/1000/foks-rs/agent.sock")
        );
        assert!(message.contains("foks-agent is not running"));
    }

    #[test]
    fn the_unreachable_message_carries_the_underlying_reason() {
        let error = AgentError::new("io", "connection refused", true);
        let message = unreachable_agent(Path::new("/tmp/agent.sock"), &error);
        assert!(message.contains("/tmp/agent.sock"));
        assert!(message.contains("connection refused"));
    }
}
