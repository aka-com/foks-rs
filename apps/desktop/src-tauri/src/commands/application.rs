//! Agent connectivity and desktop application information.

use crate::agent::{success_value, AgentError, AgentHandle};
use crate::commands::context::AppState;
use crate::commands::validation::require_main_window;
use foks_agent_proto::hello::capability::HISTORY_AFTER;
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

impl AgentStatusDto {
    /// `history_after` is whether the agent's Hello advertised
    /// [`HISTORY_AFTER`]; it is reported only for a ready agent.
    pub fn new(status: foks_agent_proto::AgentStatus, history_after: bool) -> Self {
        match status {
            foks_agent_proto::AgentStatus::Ready { .. } => Self {
                state: "ready".to_owned(),
                step: None,
                history_after: history_after.then_some(true),
            },
            foks_agent_proto::AgentStatus::Bootstrap { step } => Self {
                state: "bootstrap".to_owned(),
                step: Some(step),
                history_after: None,
            },
        }
    }

    /// The status, with the capabilities the agent's last Hello advertised.
    pub fn from_agent(status: foks_agent_proto::AgentStatus, agent: &AgentHandle) -> Self {
        Self::new(status, agent.advertises(HISTORY_AFTER))
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
    state.agent.ensure_hello().await?;
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
    Ok(AgentStatusDto::from_agent(status, &state.agent))
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
        .map(|status| AgentStatusDto::from_agent(status, &state.agent))
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
        .map(|status| AgentStatusDto::from_agent(status, &state.agent))
        .map_err(|error| {
            AgentError::unknown(format!(
                "Failed to parse background service status: {error}"
            ))
        })
}

/// Upper bound on the account name offered as a username suggestion.
const MAXIMUM_USER_NAME_BYTES: usize = 256;

/// `raw` without surrounding whitespace, or `None` when that is empty or longer
/// than `maximum` bytes.
fn bounded_name(raw: &str, maximum: usize) -> Option<String> {
    let name = raw.trim();
    (!name.is_empty() && name.len() <= maximum).then(|| name.to_owned())
}

/// The name this computer goes by, which seeds the first-run device name.
///
/// macOS reports the Sharing pane's computer name. Elsewhere on Unix this is
/// systemd-hostnamed's pretty hostname when one is set, and otherwise the
/// kernel hostname.
fn computer_name() -> Option<String> {
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
        bounded_name(&name, foks_agent_proto::MAXIMUM_DEVICE_NAME_BYTES)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        unix_names::pretty_hostname()
            .and_then(|name| bounded_name(&name, foks_agent_proto::MAXIMUM_DEVICE_NAME_BYTES))
            .or_else(|| {
                unix_names::kernel_hostname().and_then(|name| {
                    bounded_name(&name, foks_agent_proto::MAXIMUM_DEVICE_NAME_BYTES)
                })
            })
    }
    #[cfg(not(unix))]
    None
}

/// The short account name, which seeds the first-run username field.
///
/// On macOS, `NSUserName` is the login name; `NSFullUserName` is the display
/// name, which is usually the person's real name. Elsewhere on Unix the login
/// name is the password database entry's `pw_name`, not its GECOS full name. A
/// FOKS username is published to the server and shared with the people they
/// talk to, so the short name is the only one offered here: a full name reaches
/// that field only if the person types it.
fn user_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let name = objc2_foundation::NSUserName().to_string();
        bounded_name(&name, MAXIMUM_USER_NAME_BYTES)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        unix_names::login_name().and_then(|name| bounded_name(&name, MAXIMUM_USER_NAME_BYTES))
    }
    #[cfg(not(unix))]
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
mod unix_names {
    use std::ffi::CStr;
    use std::time::Duration;

    const HOSTNAMED: &str = "org.freedesktop.hostname1";
    const HOSTNAMED_PATH: &str = "/org/freedesktop/hostname1";
    /// hostnamed is started on demand by the system bus; a reply slower than
    /// this falls back to the kernel hostname.
    const HOSTNAMED_TIMEOUT: Duration = Duration::from_secs(2);
    /// Upper bound on the buffer `getpwuid_r` may ask for.
    const MAXIMUM_PASSWD_BUFFER: usize = 1 << 20;

    /// `PrettyHostname` from systemd-hostnamed, or `None` when the system bus
    /// or the service is unavailable or the name is unset.
    pub(super) fn pretty_hostname() -> Option<String> {
        let connection = zbus::blocking::connection::Builder::system()
            .and_then(|builder| builder.method_timeout(HOSTNAMED_TIMEOUT).build())
            .ok()?;
        let reply = connection
            .call_method(
                Some(HOSTNAMED),
                HOSTNAMED_PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(HOSTNAMED, "PrettyHostname"),
            )
            .ok()?;
        let value: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
        String::try_from(value).ok()
    }

    /// The kernel hostname from `gethostname`.
    pub(super) fn kernel_hostname() -> Option<String> {
        let mut buffer = [0u8; 256];
        // SAFETY: the pointer and length describe `buffer`, which outlives the call.
        if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
            return None;
        }
        // A name that fills the buffer may be truncated without a terminator.
        let name = CStr::from_bytes_until_nul(&buffer).ok()?;
        name.to_str().ok().map(str::to_owned)
    }

    /// The login name of the effective user, from the password database.
    pub(super) fn login_name() -> Option<String> {
        let mut buffer = vec![0u8; 1024];
        loop {
            // SAFETY: `passwd` is plain data that `getpwuid_r` fills in.
            let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
            let mut entry: *mut libc::passwd = std::ptr::null_mut();
            // SAFETY: every pointer refers to a live local, and the length
            // describes `buffer`, which the strings in `passwd` point into.
            let status = unsafe {
                libc::getpwuid_r(
                    libc::geteuid(),
                    &mut passwd,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut entry,
                )
            };
            if status == libc::ERANGE && buffer.len() < MAXIMUM_PASSWD_BUFFER {
                buffer.resize(buffer.len() * 2, 0);
                continue;
            }
            if status != 0 || entry.is_null() || passwd.pw_name.is_null() {
                return None;
            }
            // SAFETY: a successful lookup sets `pw_name` to a NUL-terminated
            // string inside `buffer`, which is still borrowed here.
            let name = unsafe { CStr::from_ptr(passwd.pw_name) };
            return name.to_str().ok().map(str::to_owned);
        }
    }
}

#[tauri::command]
pub async fn app_info(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AppInfo, AgentError> {
    require_main_window(&webview)?;
    // The name lookups can start a system service or a subprocess, so they run
    // off the main thread.
    let (computer, user) = tauri::async_runtime::spawn_blocking(|| (computer_name(), user_name()))
        .await
        .unwrap_or((None, None));
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
        computer_name: computer,
        user_name: user,
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
pub fn quit_app(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    closing: State<'_, std::sync::Arc<crate::close_guard::CloseGuard>>,
) -> Result<(), AgentError> {
    require_main_window(&webview)?;
    crate::close_guard::request_app_exit(&app, &closing, &state.agent);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_incremental_history_only_when_advertised_by_the_agent() {
        let legacy = AgentStatusDto::new(foks_agent_proto::AgentStatus::ready(), false);
        assert_eq!(
            serde_json::to_value(legacy).unwrap(),
            serde_json::json!({"state":"ready"})
        );
        let capable = AgentStatusDto::new(foks_agent_proto::AgentStatus::ready(), true);
        assert_eq!(
            serde_json::to_value(capable).unwrap(),
            serde_json::json!({"state":"ready", "historyAfter":true})
        );
        let bootstrap = AgentStatusDto::new(
            foks_agent_proto::AgentStatus::Bootstrap {
                step: "initialize-state".to_owned(),
            },
            true,
        );
        assert_eq!(
            serde_json::to_value(bootstrap).unwrap(),
            serde_json::json!({"state":"bootstrap", "step":"initialize-state"})
        );
    }

    #[test]
    fn suggested_names_are_trimmed_and_bounded() {
        assert_eq!(bounded_name("  host \n", 8).as_deref(), Some("host"));
        assert_eq!(bounded_name(" \t\n", 8), None);
        assert_eq!(bounded_name("12345678", 8).as_deref(), Some("12345678"));
        assert_eq!(bounded_name("123456789", 8), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_kernel_hostname_matches_procfs() {
        let expected = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
        assert_eq!(
            unix_names::kernel_hostname().as_deref(),
            Some(expected.trim_end_matches('\n'))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_suggests_a_computer_and_user_name() {
        assert!(computer_name().is_some());
        // `id -un` fails when the effective UID has no password database
        // entry, as in a container started with an arbitrary UID.
        let output = std::process::Command::new("id")
            .arg("-un")
            .output()
            .unwrap();
        let expected = String::from_utf8(output.stdout).unwrap();
        let expected = expected.trim();
        if !output.status.success() || expected.is_empty() {
            assert_eq!(user_name(), None);
            return;
        }
        let user = user_name().expect("the effective user has a password database entry");
        // The login name is a single token; the GECOS full name often is not.
        assert!(!user.contains(char::is_whitespace), "{user:?}");
        assert_eq!(expected, user);
    }
}
