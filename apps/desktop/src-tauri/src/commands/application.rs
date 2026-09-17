//! Agent connectivity and desktop application information.

use crate::agent::{success_value, AgentError};
use crate::commands::context::AppState;
use crate::commands::validation::require_main_window;
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter as _, Manager as _, State};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusDto {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}

impl From<foks_agent_proto::AgentStatus> for AgentStatusDto {
    fn from(status: foks_agent_proto::AgentStatus) -> Self {
        match status {
            foks_agent_proto::AgentStatus::Ready => Self {
                state: "ready".to_owned(),
                step: None,
            },
            foks_agent_proto::AgentStatus::Bootstrap { step } => Self {
                state: "bootstrap".to_owned(),
                step: Some(step),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub agent_socket: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer_name: Option<String>,
}

#[tauri::command]
pub async fn agent_status(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AgentStatusDto, AgentError> {
    require_main_window(&webview)?;
    let value = success_value(
        state
            .agent
            .call(foks_agent_proto::Operation::AgentStatus)
            .await?,
    )?;
    serde_json::from_value::<foks_agent_proto::AgentStatus>(value)
        .map(AgentStatusDto::from)
        .map_err(|error| {
            AgentError::unknown(format!(
                "Could not parse background service status: {error}"
            ))
        })
}

#[tauri::command]
pub async fn retry_agent_connection(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AgentStatusDto, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let agent = Arc::clone(&state.agent);
    let restored = Arc::clone(&agent);
    let response = tauri::async_runtime::spawn_blocking(move || {
        let response = agent.retry_started_blocking()?;
        let worker_state = app.state::<AppState>();
        restored.record_restoration_success(&|snapshot| {
            let _ = app.emit(crate::agent::MAINTENANCE_EVENT, snapshot);
        });
        worker_state.invalidate_catalog();
        Ok::<_, AgentError>(response)
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!("Failed to restart background service: {error}"))
    })??;
    let value = success_value(response)?;
    serde_json::from_value::<foks_agent_proto::AgentStatus>(value)
        .map(AgentStatusDto::from)
        .map_err(|error| {
            AgentError::unknown(format!(
                "Failed to parse background service status: {error}"
            ))
        })
}

fn macos_computer_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/sbin/scutil")
            .args(["--get", "ComputerName"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let name = String::from_utf8(output.stdout).ok()?;
        let name = name.trim();
        if name.is_empty() || name.len() > 256 {
            return None;
        }
        Some(name.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    None
}

#[tauri::command]
pub fn app_info(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AppInfo, AgentError> {
    require_main_window(&webview)?;
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        agent_socket: state.agent.socket().display().to_string(),
        managed_profile: std::env::var("FOKS_MANAGED_PROFILE")
            .ok()
            .filter(|profile| {
                !profile.is_empty()
                    && profile.len() <= 64
                    && profile
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            }),
        computer_name: macos_computer_name(),
    })
}

/// The process answering on the agent socket: Settings › This Mac shows it
/// before offering to stop it.
#[tauri::command]
pub fn agent_process_info(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<crate::agent::AgentProcessInfo, AgentError> {
    require_main_window(&webview)?;
    Ok(state.agent.process_info())
}

/// Stops the local agent and starts it again. `takeover` claims an agent this
/// app did not start, which the reader has confirmed in the sheet that asks.
#[tauri::command]
pub async fn restart_agent(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    takeover: bool,
) -> Result<crate::agent::MaintenanceSnapshot, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let generation = crate::applock::unlocked_generation(&app)?;
    let agent = state.agent.clone();
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let state = app.state::<AppState>();
        crate::applock::require_unlocked_generation(&app, generation)?;
        let _mutation = state.begin_mutation()?;
        agent.restart_agent(takeover, &|snapshot| {
            if matches!(snapshot, crate::agent::MaintenanceSnapshot::Complete { .. }) {
                state.invalidate_catalog();
            }
            let _ = app.emit(crate::agent::MAINTENANCE_EVENT, snapshot);
        })
    })
    .await
    .map_err(|_| AgentError::unknown("Agent restart worker interrupted."))?
}

#[tauri::command]
pub fn take_agent_connection_loss(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<Option<String>, AgentError> {
    require_main_window(&webview)?;
    Ok(state.agent.take_connection_failure())
}

#[tauri::command]
pub fn restart_app(app: tauri::AppHandle, webview: tauri::Webview) -> Result<(), AgentError> {
    require_main_window(&webview)?;
    app.restart()
}

#[tauri::command]
pub fn quit_app(app: tauri::AppHandle, webview: tauri::Webview) -> Result<(), AgentError> {
    require_main_window(&webview)?;
    app.exit(0);
    Ok(())
}
