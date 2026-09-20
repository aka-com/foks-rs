//! Displays blocking native dialogs and exits on fatal startup errors.

use std::path::Path;

use tauri::Manager as _;
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::{
    agent::{AgentError, AgentHandle},
    commands::MAIN,
};

/// Shows a blocking dialog and exits non-zero.
///
/// Invoked from the startup thread to display a fatal error dialog before a
/// non-zero exit.
pub fn fatal_startup(app: &tauri::AppHandle, title: &str, body: &str) -> ! {
    show_fatal_startup(app, title, body);
    std::process::exit(1);
}

fn show_fatal_startup(app: &tauri::AppHandle, title: &str, body: &str) {
    let window = app
        .get_webview_window(MAIN)
        .expect("main window unavailable during startup");
    app.dialog()
        .message(body)
        .parent(&window)
        .kind(MessageDialogKind::Error)
        .title(title)
        .blocking_show();
}

fn quit_after_agent_startup_error(app: &tauri::AppHandle, agent: &AgentHandle) -> ! {
    let candidates = agent.stale_agent_processes();
    if !candidates.is_empty() {
        let window = app
            .get_webview_window(MAIN)
            .expect("main window unavailable during startup");
        let confirmed = app
            .dialog()
            .message(stale_agent_cleanup_message(agent, &candidates))
            .parent(&window)
            .kind(MessageDialogKind::Warning)
            .title("Terminate Existing FOKS Agents?")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Terminate Agents and Quit".into(),
                "Leave Running and Quit".into(),
            ))
            .blocking_show();
        if confirmed {
            // A confirmation that quietly did nothing is worse than no offer
            // at all: the reader was told these processes would be stopped, so
            // the ones that were not are named before the app goes away.
            let refused = candidates
                .iter()
                .filter_map(|candidate| {
                    agent
                        .terminate_stale_agent(candidate)
                        .err()
                        .map(|error| (candidate.pid, error))
                })
                .collect::<Vec<_>>();
            if !refused.is_empty() {
                show_fatal_startup(
                    app,
                    "Agents Could Not Be Terminated",
                    &stale_agent_cleanup_failure_message(agent, &refused),
                );
            }
        }
    }
    std::process::exit(1);
}

fn fatal_agent_startup(app: &tauri::AppHandle, agent: &AgentHandle, title: &str, body: &str) -> ! {
    show_fatal_startup(app, title, body);
    quit_after_agent_startup_error(app, agent)
}

fn agent_state_directory(agent: &AgentHandle) -> &Path {
    agent.socket().parent().unwrap_or_else(|| Path::new("."))
}

fn stale_agent_cleanup_message(
    agent: &AgentHandle,
    candidates: &[crate::agent::StaleAgentProcess],
) -> String {
    agent_cleanup_message(agent, candidates, false)
}

fn agent_cleanup_message(
    agent: &AgentHandle,
    candidates: &[crate::agent::StaleAgentProcess],
    continuing: bool,
) -> String {
    let processes = candidates
        .iter()
        .map(|candidate| {
            format!(
                "    Process: {}\n    Executable: {}\n    Started: {}",
                candidate.pid,
                candidate.executable.display(),
                candidate.started_at,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let action = if continuing {
        "These additional processes are not serving this application's current socket. They may still serve other clients; terminating them will disconnect those clients.\n\nTerminate these additional processes and continue?"
    } else {
        "Terminate these processes before quitting?"
    };
    format!(
        "FOKS found {} foks-agent process{} configured to use {}. They may continue accessing that data directory after this application quits.\n\n{}\n\n{}",
        candidates.len(),
        if candidates.len() == 1 { "" } else { "es" },
        agent_state_directory(agent).display(),
        processes,
        action,
    )
}

fn stale_agent_cleanup_failure_message(
    agent: &AgentHandle,
    refused: &[(u32, AgentError)],
) -> String {
    let processes = refused
        .iter()
        .map(|(pid, error)| format!("    Process {}: {}", pid, error.message))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (plural, pronoun) = if refused.len() == 1 {
        ("", "It is")
    } else {
        ("es", "They are")
    };
    format!(
        "FOKS could not terminate {} foks-agent process{}. {} still running and may continue accessing {}.\n\n{}\n\nQuit the remaining processes yourself before starting FOKS again.",
        refused.len(),
        plural,
        pronoun,
        agent_state_directory(agent).display(),
        processes,
    )
}

fn offer_additional_agent_cleanup(app: &tauri::AppHandle, agent: &AgentHandle) {
    let candidates = agent.additional_agent_processes();
    if candidates.is_empty() {
        return;
    }
    let window = app
        .get_webview_window(MAIN)
        .expect("main window unavailable during startup");
    if !app
        .dialog()
        .message(agent_cleanup_message(agent, &candidates, true))
        .parent(&window)
        .kind(MessageDialogKind::Warning)
        .title("Additional FOKS Agents Found")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Terminate Additional Agents".into(),
            "Leave Running".into(),
        ))
        .blocking_show()
    {
        return;
    }
    let failures = candidates
        .iter()
        .filter_map(|candidate| {
            agent
                .terminate_additional_agent(candidate)
                .err()
                .map(|error| format!("Process {}: {}", candidate.pid, error.message))
        })
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        app.dialog()
            .message(failures.join("\n\n"))
            .parent(&window)
            .kind(MessageDialogKind::Warning)
            .title("Agent Cleanup Incomplete")
            .blocking_show();
    }
}

/// Confirms the agent is reachable before the first request.
///
/// Verifies that the agent socket exists and the agent responds to requests.
/// Runs on a background thread so the main thread can paint the webview while
/// the agent starts; its blocking dialogs are dispatched to the main thread by
/// the dialog plugin. The caller holds ordinary agent commands until this
/// returns (see `AgentHandle::hold_commands_for_startup`).
pub fn require_agent(app: &tauri::AppHandle, agent: &AgentHandle) {
    let window = app
        .get_webview_window(MAIN)
        .expect("main window unavailable during startup");
    let socket = agent.socket();
    loop {
        if let Err(error) = agent.ensure_started_with_confirmation(&|target| {
            app.dialog()
                .message(takeover_message(
                    &target.socket,
                    target.pid,
                    &target.executable,
                ))
                .parent(&window)
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
            if let Some(root) = agent.startup_reset_directory() {
                let reset = app
                    .dialog()
                    .message(format!(
                        "{body}\n\nYou can delete this device's local FOKS state and start again."
                    ))
                    .parent(&window)
                    .kind(MessageDialogKind::Error)
                    .title("Agent Connection Failed")
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "Review Reset…".into(),
                        "Quit".into(),
                    ))
                    .blocking_show();
                if !reset {
                    quit_after_agent_startup_error(app, agent);
                }
                let confirmed = app
                    .dialog()
                    .message(reset_warning(&root))
                    .parent(&window)
                    .kind(MessageDialogKind::Warning)
                    .title("Delete Local FOKS State?")
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "Delete Local State and Continue".into(),
                        "Cancel".into(),
                    ))
                    .blocking_show();
                if !confirmed {
                    quit_after_agent_startup_error(app, agent);
                }
                if let Err(error) = agent.reset_startup_state(&root) {
                    let body = format!(
                        "Could not finish deleting local state at {}.\n\n{}\n\n\
                         Deletion may be incomplete. Close other FOKS clients and agents before trying again.",
                        root.display(), error.message,
                    );
                    fatal_agent_startup(app, agent, "Local State Reset Failed", &body);
                }
                continue;
            }
            fatal_agent_startup(app, agent, "Agent Connection Failed", &body);
        }
        break;
    }
    // AgentStatus already completed a version-checked round trip. Do not add
    // a second startup gate that bypasses takeover recovery if the owner changes.
    offer_additional_agent_cleanup(app, agent);
}

fn reset_warning(root: &Path) -> String {
    format!(
        "Permanently delete all local FOKS state in {}?\n\n\
         This deletes local account keys, profiles, trust history, cached data, and unfinished operations for every server on this device.\n\n\
         Server data is not deleted. You can permanently lose access to your accounts without another enrolled device, a recovery phrase for an enrolled backup, or a usable external backup. Your account passphrase alone cannot restore deleted keys.\n\n\
         This cannot be undone. FOKS will restart its background service and open setup. Old system credential-store entries may remain; they will not be reused by the new state.",
        root.display(),
    )
}

fn takeover_message(socket: &Path, pid: u32, executable: &Path) -> String {
    format!(
        "A different FOKS version is using {}.\n\n    Process: {}\n    Executable: {}\n\n\
         Terminate this agent and let this application take over the socket? \
         Other clients using this agent will be disconnected. Your accounts and vault data will not be deleted.",
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
pub fn fatal_state_tampered(app: &tauri::AppHandle, file: &Path) -> ! {
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
    fn reset_warning_explains_scope_loss_and_continuation() {
        let warning = reset_warning(Path::new("/private/foks-state"));
        for text in [
            "/private/foks-state",
            "every server",
            "permanently lose access",
            "passphrase alone cannot restore",
            "cannot be undone",
            "open setup",
            "Server data is not deleted",
        ] {
            assert!(warning.contains(text), "missing warning: {text}");
        }
    }

    #[test]
    fn stale_agent_prompt_identifies_every_confirmed_target() {
        let agent = AgentHandle::new(std::path::PathBuf::from("/private/foks/foks-rs.sock"));
        let message = stale_agent_cleanup_message(
            &agent,
            &[
                crate::agent::StaleAgentProcess {
                    pid: 123,
                    executable: "/tmp/old/foks-agent".into(),
                    started_at: 456,
                    state_dir: "/private/foks".into(),
                },
                crate::agent::StaleAgentProcess {
                    pid: 789,
                    executable: "/Applications/FOKS.app/Contents/MacOS/foks-agent".into(),
                    started_at: 999,
                    state_dir: "/private/foks".into(),
                },
            ],
        );
        assert!(message.contains("2 foks-agent processes"));
        assert!(message.contains("/private/foks"));
        assert!(message.contains("Process: 123"));
        assert!(message.contains("Executable: /tmp/old/foks-agent"));
        assert!(message.contains("Process: 789"));
        assert!(message.contains("Terminate these processes before quitting?"));
    }

    #[test]
    fn stale_agent_cleanup_reports_every_process_it_could_not_terminate() {
        let agent = AgentHandle::new(std::path::PathBuf::from("/private/foks/foks-rs.sock"));
        let message = stale_agent_cleanup_failure_message(
            &agent,
            &[
                (
                    123,
                    AgentError::new(
                        "agent-cleanup-changed",
                        "The agent process changed after confirmation and was not terminated.",
                        false,
                    ),
                ),
                (
                    789,
                    AgentError::new(
                        "agent-cleanup-failed",
                        "Failed to terminate foks-agent process 789: Operation not permitted",
                        false,
                    ),
                ),
            ],
        );
        assert!(message.contains("could not terminate 2 foks-agent processes"));
        assert!(message.contains("They are still running"));
        assert!(message.contains("/private/foks"));
        assert!(message.contains("Process 123: The agent process changed after confirmation"));
        assert!(message.contains("Process 789: Failed to terminate foks-agent process 789"));
        assert!(message.contains("before starting FOKS again"));

        let single = stale_agent_cleanup_failure_message(
            &agent,
            &[(
                123,
                AgentError::new("agent-cleanup-failed", "Operation not permitted", false),
            )],
        );
        assert!(single.contains("could not terminate 1 foks-agent process."));
        assert!(single.contains("It is still running"));
    }

    #[test]
    fn takeover_prompt_identifies_the_target_and_explains_continuation() {
        let message = takeover_message(
            Path::new("/tmp/foks/agent.sock"),
            12345,
            Path::new("/tmp/old/foks-agent"),
        );
        assert!(message.contains("/tmp/foks/agent.sock"));
        assert!(message.contains("    Process: 12345\n    Executable: /tmp/old/foks-agent"));
        assert!(message.contains("will be disconnected"));
        assert!(message.contains("vault data will not be deleted"));
        assert!(!message.contains("continue startup"));
        assert!(!message.contains("ask again"));
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
