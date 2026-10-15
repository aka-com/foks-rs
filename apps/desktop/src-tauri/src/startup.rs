//! Displays blocking native dialogs and exits on fatal startup errors.

use std::path::{Path, PathBuf};

use tauri::Manager as _;
use tauri_plugin_dialog::{
    DialogExt as _, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

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
            let (title, kind, body) = if error.code == "agent-start-timeout" {
                (
                    "Agent Still Starting",
                    MessageDialogKind::Warning,
                    starting_agent(socket, &error),
                )
            } else if !socket.exists() {
                (
                    "Agent Connection Failed",
                    MessageDialogKind::Error,
                    missing_socket(socket, &error),
                )
            } else {
                (
                    "Agent Connection Failed",
                    MessageDialogKind::Error,
                    unreachable_agent(socket, &error),
                )
            };
            let body = format!("{body}{}", agent_log_line(agent));
            let root = match startup_recovery(&error, agent.startup_reset_directory()) {
                StartupRecovery::Retry => {
                    let retry = app
                        .dialog()
                        .message(body)
                        .parent(&window)
                        .kind(kind)
                        .title(title)
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            TRY_AGAIN.into(),
                            QUIT.into(),
                        ))
                        .blocking_show();
                    if !retry {
                        quit_after_agent_startup_error(app, agent);
                    }
                    continue;
                }
                StartupRecovery::RetryOrReset(root) => root,
            };
            let choice = app
                .dialog()
                .message(format!(
                    "{body}\n\nIf the agent output reports a locked or denied system credential store, \
                     resolve that and choose Try Again. Otherwise you can move this device's local FOKS \
                     state aside and start again."
                ))
                .parent(&window)
                .kind(MessageDialogKind::Error)
                .title(title)
                .buttons(MessageDialogButtons::YesNoCancelCustom(
                    TRY_AGAIN.into(),
                    REVIEW_RESET.into(),
                    QUIT.into(),
                ))
                .blocking_show_with_result();
            match startup_choice(&choice) {
                StartupChoice::Retry => continue,
                StartupChoice::Reset => {}
                StartupChoice::Quit => quit_after_agent_startup_error(app, agent),
            }
            let confirmed = app
                .dialog()
                .message(reset_warning(&root))
                .parent(&window)
                .kind(MessageDialogKind::Warning)
                .title("Move Local FOKS State Aside?")
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "Move State and Continue".into(),
                    "Cancel".into(),
                ))
                .blocking_show();
            if !confirmed {
                quit_after_agent_startup_error(app, agent);
            }
            match agent.reset_startup_state(&root) {
                Ok(moved) => {
                    app.dialog()
                        .message(moved_state_message(&root, &moved))
                        .parent(&window)
                        .kind(MessageDialogKind::Info)
                        .title("Local FOKS State Moved")
                        .blocking_show();
                }
                Err(error) => {
                    fatal_agent_startup(
                        app,
                        agent,
                        "Local State Reset Failed",
                        &reset_failure_message(&root, &error),
                    );
                }
            }
            continue;
        }
        break;
    }
    // AgentStatus already completed a version-checked round trip. Do not add
    // a second startup gate that bypasses takeover recovery if the owner changes.
    offer_additional_agent_cleanup(app, agent);
}

const TRY_AGAIN: &str = "Try Again";
const REVIEW_RESET: &str = "Review Reset…";
const QUIT: &str = "Quit";

/// The recovery a startup failure dialog offers besides Quit.
#[derive(Debug, Eq, PartialEq)]
enum StartupRecovery {
    /// Try Again only. The failure does not show that the local state is
    /// unreadable: the agent may still be starting, or never ran.
    Retry,
    /// Try Again, or move the state in this root aside. The launched agent
    /// exited before it created its socket.
    RetryOrReset(PathBuf),
}

fn startup_recovery(error: &AgentError, reset_root: Option<PathBuf>) -> StartupRecovery {
    match reset_root {
        Some(root) if error.code == "agent-start-failed" => StartupRecovery::RetryOrReset(root),
        _ => StartupRecovery::Retry,
    }
}

#[derive(Debug, Eq, PartialEq)]
enum StartupChoice {
    Retry,
    Reset,
    Quit,
}

/// Maps the three-button dialog's result. Platforms report either the custom
/// label or the underlying button; anything else, including closing the
/// dialog, quits.
fn startup_choice(result: &MessageDialogResult) -> StartupChoice {
    match result {
        MessageDialogResult::Yes => StartupChoice::Retry,
        MessageDialogResult::No => StartupChoice::Reset,
        MessageDialogResult::Custom(label) if label == TRY_AGAIN => StartupChoice::Retry,
        MessageDialogResult::Custom(label) if label == REVIEW_RESET => StartupChoice::Reset,
        _ => StartupChoice::Quit,
    }
}

/// Names the managed agent's log, which holds the full output of every launch.
fn agent_log_line(agent: &AgentHandle) -> String {
    let log = agent_state_directory(agent).join("agent.log");
    if log.is_file() {
        format!("\n\nAgent log: {}", log.display())
    } else {
        String::new()
    }
}

fn reset_warning(root: &Path) -> String {
    format!(
        "Move all local FOKS state in {} into a new directory beside it and start again?\n\n\
         FOKS will restart its background service and open setup with empty local state. \
         The moved directory keeps the local account keys, profiles, trust history, cached data, unfinished operations, and agent log for every server on this device. \
         Nothing is deleted, system credential-store entries are left in place, and server data is not affected.\n\n\
         To return to the previous state, quit FOKS and replace the contents of {} with the contents of the moved directory.\n\n\
         The moved directory still contains local keys. If you delete it, you can permanently lose access to your accounts without another enrolled device, \
         a recovery phrase for an enrolled backup, or a usable external backup. Your account passphrase alone cannot restore deleted keys.",
        root.display(),
        root.display(),
    )
}

fn moved_state_message(root: &Path, moved: &Path) -> String {
    format!(
        "The local FOKS state from {} was moved to {}.\n\n\
         FOKS will now start with empty local state and open setup. \
         To return to the previous state, quit FOKS and replace the contents of {} with the contents of {}.",
        root.display(),
        moved.display(),
        root.display(),
        moved.display(),
    )
}

fn reset_failure_message(root: &Path, error: &AgentError) -> String {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(
        "Could not move the local state at {} aside.\n\n{}\n\n\
         Nothing was deleted. Entries already moved are in a directory beside it named {}.reset- followed by a timestamp. \
         Close other FOKS clients and agents before trying again.",
        root.display(),
        error.message,
        name,
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
         {}\n\nVerify the client state and foks-agent configuration, then try again.",
        socket.display(),
        error.message
    )
}

fn starting_agent(socket: &Path, error: &AgentError) -> String {
    format!(
        "FOKS is still waiting for the local background service at {} to start.\n\n\
         {}\n\nAfter responding to any credential prompt, choose Try Again to keep waiting.",
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
    fn reset_warning_explains_scope_recovery_and_continuation() {
        let warning = reset_warning(Path::new("/private/foks-state"));
        for text in [
            "/private/foks-state",
            "new directory beside it",
            "every server",
            "Nothing is deleted",
            "credential-store entries are left in place",
            "replace the contents of /private/foks-state",
            "permanently lose access",
            "passphrase alone cannot restore",
            "open setup",
            "server data is not affected",
        ] {
            assert!(warning.contains(text), "missing warning: {text}");
        }
        assert!(!warning.contains("cannot be undone"));
        let moved = moved_state_message(
            Path::new("/private/foks-state"),
            Path::new("/private/foks-state.reset-1700000000"),
        );
        assert!(moved.contains("moved to /private/foks-state.reset-1700000000"));
        assert!(moved.contains("replace the contents of /private/foks-state with"));
        let failure = reset_failure_message(
            Path::new("/private/foks-state"),
            &AgentError::new("agent-reset", "state is busy", false),
        );
        assert!(failure.contains("state is busy"));
        assert!(failure.contains("Nothing was deleted"));
        assert!(failure.contains("named foks-state.reset-"));
    }

    #[test]
    fn startup_reset_is_offered_only_after_the_launched_agent_exited() {
        let root = PathBuf::from("/private/foks-state");
        let exited = AgentError::new("agent-start-failed", "exited (exit status: 1)", true);
        let waiting = AgentError::new("agent-start-timeout", "still running", true);
        let busy = AgentError::new("agent-state", "state is busy", false);
        assert_eq!(
            startup_recovery(&exited, Some(root.clone())),
            StartupRecovery::RetryOrReset(root.clone())
        );
        assert_eq!(startup_recovery(&exited, None), StartupRecovery::Retry);
        assert_eq!(
            startup_recovery(&waiting, Some(root.clone())),
            StartupRecovery::Retry
        );
        assert_eq!(startup_recovery(&busy, Some(root)), StartupRecovery::Retry);
    }

    #[test]
    fn startup_choice_maps_labels_and_quits_on_anything_else() {
        let custom = |label: &str| MessageDialogResult::Custom(label.to_owned());
        assert_eq!(startup_choice(&custom(TRY_AGAIN)), StartupChoice::Retry);
        assert_eq!(startup_choice(&custom(REVIEW_RESET)), StartupChoice::Reset);
        assert_eq!(startup_choice(&custom(QUIT)), StartupChoice::Quit);
        assert_eq!(
            startup_choice(&MessageDialogResult::Yes),
            StartupChoice::Retry
        );
        assert_eq!(
            startup_choice(&MessageDialogResult::No),
            StartupChoice::Reset
        );
        assert_eq!(
            startup_choice(&MessageDialogResult::Cancel),
            StartupChoice::Quit
        );
        assert_eq!(startup_choice(&custom("Other")), StartupChoice::Quit);
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
    fn the_still_starting_message_asks_for_the_prompt_and_a_retry() {
        let error = AgentError::new("agent-start-timeout", "process 42 is still running", true);
        let message = starting_agent(Path::new("/run/user/1000/foks-rs/agent.sock"), &error);
        assert!(message.contains(
            "still waiting for the local background service at /run/user/1000/foks-rs/agent.sock"
        ));
        assert!(message.contains("process 42 is still running"));
        assert!(message.contains("choose Try Again"));
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
