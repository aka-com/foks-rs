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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_after: Option<bool>,
}

impl From<foks_agent_proto::AgentStatus> for AgentStatusDto {
    fn from(status: foks_agent_proto::AgentStatus) -> Self {
        match status {
            foks_agent_proto::AgentStatus::Ready { history_after, .. } => Self {
                state: "ready".to_owned(),
                step: None,
                history_after,
            },
            foks_agent_proto::AgentStatus::Bootstrap { step } => Self {
                state: "bootstrap".to_owned(),
                step: Some(step),
                history_after: None,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
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
    let status =
        serde_json::from_value::<foks_agent_proto::AgentStatus>(value).map_err(|error| {
            AgentError::unknown(format!(
                "Could not parse background service status: {error}"
            ))
        })?;
    // The reply carries the agent's background loops. Recording them here is
    // what puts them in a readout beside the requests they competed with.
    state.agent.note_agent_timers(&status);
    Ok(AgentStatusDto::from(status))
}

#[tauri::command]
pub async fn auto_recover_agent(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AgentStatusDto, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let access = crate::applock::unlocked_generation(&app)?;
    let agent = Arc::clone(&state.agent);
    let response = tauri::async_runtime::spawn_blocking(move || {
        crate::applock::require_unlocked_generation(&app, access)?;
        agent.auto_recover_blocking()
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Agent recovery worker failed: {error}")))??;
    crate::applock::require_unlocked_generation(webview.app_handle(), access)?;
    let value = success_value(response)?;
    let status = serde_json::from_value::<foks_agent_proto::AgentStatus>(value)
        .map(AgentStatusDto::from)
        .map_err(|error| {
            AgentError::new("protocol", format!("Invalid agent status: {error}"), false)
        })?;
    state.invalidate_catalog();
    Ok(status)
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

/// The short account name, which seeds the first-run username field.
///
/// `NSUserName` is the login name; `NSFullUserName` is the display name, which
/// is usually the person's real name. A FOKS username is published to the
/// server and shared with the people they talk to, so the short name is the
/// only one offered here: a full name reaches that field only if the person
/// types it.
fn macos_user_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let name = objc2_foundation::NSUserName().to_string();
        let name = name.trim();
        if name.is_empty() || name.len() > 256 {
            return None;
        }
        Some(name.to_owned())
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
        user_name: macos_user_name(),
    })
}

/// The backend's timing log from a cursor: every agent operation issued
/// since, under the command that issued it. Read by Copy diagnostics.
#[tauri::command]
pub fn diagnostic_timings(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    since: u64,
) -> Result<crate::diagnostics::TimingBatch, AgentError> {
    require_main_window(&webview)?;
    Ok(state.agent.timings().since(since))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_incremental_history_only_when_advertised_by_the_agent() {
        let legacy = AgentStatusDto::from(foks_agent_proto::AgentStatus::ready());
        assert_eq!(
            serde_json::to_value(legacy).unwrap(),
            serde_json::json!({"state":"ready"})
        );
        let capable = AgentStatusDto::from(foks_agent_proto::AgentStatus::Ready {
            timers: vec![],
            history_after: Some(true),
        });
        assert_eq!(
            serde_json::to_value(capable).unwrap(),
            serde_json::json!({"state":"ready", "historyAfter":true})
        );
    }
}
