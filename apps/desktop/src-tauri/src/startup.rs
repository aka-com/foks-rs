//! Displays blocking native dialogs and exits on fatal startup errors.

use std::path::Path;

use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

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
    if let Err(error) = agent.ensure_started_with_confirmation(&|target| {
        app.dialog()
            .message(takeover_message(
                &target.socket,
                target.pid,
                &target.executable,
            ))
            .kind(MessageDialogKind::Warning)
            .title("Replace Existing FOKS Agent?")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Terminate and Take Over".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    }) {
        if error.code == "agent-takeover-declined" {
            std::process::exit(0);
        }
        let body = if !socket.exists() {
            missing_socket(socket, &error)
        } else {
            unreachable_agent(socket, &error)
        };
        fatal_startup(app, "Agent Connection Failed", &body);
    }
    // AgentStatus already completed a version-checked round trip. Do not add
    // a second startup gate that bypasses takeover recovery if the owner changes.
}

fn takeover_message(socket: &Path, pid: u32, executable: &Path) -> String {
    format!(
        "A different FOKS version is using {}.\n\nProcess: {}\nExecutable: {}\n\n\
         Terminate this agent and let this application take over the socket? \
         Other clients using this agent will be disconnected. Your accounts and vault data will not be deleted.\n\n\
         FOKS will start its matching background service and continue startup. \
         If another process claims the socket, FOKS will ask again before terminating it.",
        socket.display(), pid, executable.display(),
    )
}

fn missing_socket(socket: &Path, error: &AgentError) -> String {
    format!(
        "FOKS could not connect to the local background service at {}.\n\n\
         {}\n\nVerify the client state and foks-agent configuration, then relaunch the application.",
        socket.display(),
        error.message
    )
}

fn unreachable_agent(socket: &Path, error: &AgentError) -> String {
    if error.code == "version-mismatch" {
        return format!(
            "Failed to connect to agent at {}.\n\n{}\n\nThe local background \
             service is from a different FOKS version. FOKS could not safely replace \
             the foks-agent process. No unconfirmed socket owner was terminated.",
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
    fn takeover_prompt_identifies_the_target_and_explains_continuation() {
        let message = takeover_message(
            Path::new("/tmp/foks/agent.sock"),
            12345,
            Path::new("/tmp/old/foks-agent"),
        );
        assert!(message.contains("/tmp/foks/agent.sock"));
        assert!(message.contains("Process: 12345"));
        assert!(message.contains("/tmp/old/foks-agent"));
        assert!(message.contains("will be disconnected"));
        assert!(message.contains("vault data will not be deleted"));
        assert!(message.contains("continue startup"));
        assert!(message.contains("ask again"));
    }

    #[test]
    fn the_missing_socket_message_names_the_path_reason_and_remedy() {
        let error = AgentError::new("agent-state", "unsupported client state version", false);
        let message = missing_socket(Path::new("/run/user/1000/foks-rs/agent.sock"), &error);
        assert!(message.contains(
            "FOKS could not connect to the local background service at /run/user/1000/foks-rs/agent.sock"
        ));
        assert!(message.contains("unsupported client state version"));
        assert!(message.contains("Verify the client state and foks-agent configuration"));
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
