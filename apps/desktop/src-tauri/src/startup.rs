//! Displays blocking native dialogs and exits on fatal startup errors.

use std::path::Path;

use tauri_plugin_dialog::{DialogExt as _, MessageDialogKind};

use crate::agent::{AgentError, AgentHandle};

/// Shows a blocking dialog and exits non-zero.
///
/// Invoked during setup to display a fatal error dialog before non-zero exit,
/// avoiding an unhandled setup abort.
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
/// Verifies that the agent socket exists and the agent responds to requests.
pub fn require_agent(app: &tauri::App, agent: &AgentHandle) {
    let socket = agent.socket();
    if let Err(error) = agent.ensure_started_blocking() {
        let body = if !socket.exists() {
            missing_socket(socket)
        } else {
            unreachable_agent(socket, &error)
        };
        fatal_startup(app, "Agent Connection Failed", &body);
    }
    if let Err(error) = agent.call_blocking(foks_agent_proto::Operation::Ping) {
        fatal_startup(
            app,
            "Agent Connection Failed",
            &unreachable_agent(socket, &error),
        );
    }
}

fn missing_socket(socket: &Path) -> String {
    format!(
        "FOKS could not connect to the local background service at {}.\n\n\
         Verify that foks-agent is running and relaunch the application.",
        socket.display()
    )
}

fn unreachable_agent(socket: &Path, error: &AgentError) -> String {
    if error.code == "version-mismatch" {
        return format!(
            "Failed to connect to agent at {}.\n\n{}\n\nThe local background \
             service is from a different FOKS version. Stop that foks-agent \
             process and relaunch this application.",
            socket.display(),
            error.message
        );
    }
    format!(
        "Failed to connect to agent at {}.\n\n{}\n\nVerify that the \
         agent is running and try again.",
        socket.display(),
        error.message
    )
}

/// Displays an error dialog when on-disk state fails integrity verification.
#[allow(dead_code)]
pub fn fatal_state_tampered(app: &tauri::App, file: &Path) -> ! {
    fatal_startup(
        app,
        "FOKS state has been altered",
        &format!(
            "FOKS detected unexpected changes to {}.\n\nThe application state \
             may be corrupted or tampered with and cannot be used safely. \
             Reinstall FOKS or restore state from a trusted backup.",
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
        assert!(message.contains(
            "FOKS could not connect to the local background service at /run/user/1000/foks-rs/agent.sock"
        ));
        assert!(message.contains("verify that foks-agent is running"));
    }

    #[test]
    fn the_unreachable_message_carries_the_underlying_reason() {
        let error = AgentError::new("io", "connection refused", true);
        let message = unreachable_agent(Path::new("/tmp/agent.sock"), &error);
        assert!(message.contains("/tmp/agent.sock"));
        assert!(message.contains("connection refused"));
    }

    #[test]
    fn version_mismatch_names_the_stale_background_service() {
        let error = AgentError::from_agent(
            foks_agent_proto::ErrorCode::VersionMismatch,
            "Desktop and agent protocol versions do not match".to_owned(),
        );
        let message = unreachable_agent(Path::new("/tmp/agent.sock"), &error);
        assert!(message.contains("/tmp/agent.sock"));
        assert!(message.contains("Desktop and agent protocol versions do not match"));
        assert!(message.contains("different FOKS version"));
        assert!(message.contains("foks-agent"));
    }
}
