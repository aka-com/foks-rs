//! One-time bot secrets cross native IPC and a private file, never the webview.
use super::{
    context::AppState,
    validation::{invalid_request, invalid_response, require_main_window, valid_local_name},
};
use crate::agent::AgentError;
use foks_agent_proto::{bot::BotAction, KvRole, Operation, SecretString};
use serde::{Deserialize, Serialize};
use tauri::{Manager as _, State};
use tauri_plugin_dialog::DialogExt;
use zeroize::{Zeroize as _, Zeroizing};
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    List,
    LoadFile,
    Unload,
    Prepare {
        role: KvRole,
        pin: Option<SecretString>,
    },
    Attempt {
        operation_id: String,
        pin: Option<SecretString>,
    },
    Status {
        operation_id: String,
        pin: Option<SecretString>,
    },
    Cancel {
        operation_id: String,
    },
    ExportFile {
        operation_id: String,
    },
    Revoke {
        device_id: String,
        pin: Option<SecretString>,
    },
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    operation_id: String,
    account_alias: String,
    name: String,
    device_id: String,
    role: String,
    state: String,
    hardware_required: bool,
    export_available: bool,
}
#[derive(Serialize)]
pub struct Reply {
    rows: Vec<Enrollment>,
    message: &'static str,
}
fn project(
    value: serde_json::Value,
    alias: &str,
    id: Option<&str>,
    list: bool,
) -> Result<Vec<Enrollment>, AgentError> {
    let rows: Vec<Enrollment> = serde_json::from_value(if list {
        value
    } else {
        serde_json::json!([value])
    })
    .map_err(|_| invalid_response("Invalid bot enrollment response."))?;
    if rows.len() > 160
        || rows.iter().any(|p| {
            p.account_alias != alias
                || id.is_some_and(|id| p.operation_id != id)
                || !(BotAction::Cancel {
                    operation_id: p.operation_id.clone(),
                })
                .validate()
                || !(BotAction::Revoke {
                    device_id: p.device_id.clone(),
                    pin: None,
                })
                .validate()
                || p.name.len() != 5
                || !p.name.bytes().all(|b| b.is_ascii_alphanumeric())
                || p.role.len() > 64
                || !matches!(
                    p.state.as_str(),
                    "prepared"
                        | "submitting"
                        | "submission-unknown"
                        | "remote-verified"
                        | "complete"
                        | "rejected"
                )
        })
    {
        return Err(invalid_response(
            "Bot enrollment changed account or operation.",
        ));
    }
    Ok(rows)
}
#[tauri::command]
pub async fn bot_account_request(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    action: Action,
) -> Result<Reply, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let generation = crate::applock::unlocked_generation(&app)?;
    if !valid_local_name(&profile) || !valid_local_name(&account_alias) {
        return Err(invalid_request("Invalid bot account."));
    }
    let changes_catalog = !matches!(action, Action::List);
    let _mutation = if changes_catalog {
        Some(state.begin_mutation()?)
    } else {
        None
    };
    let mut output = None;
    let (action, message) = match action {
        Action::LoadFile => {
            let picker = app.clone();
            let path = tauri::async_runtime::spawn_blocking(move || {
                picker.dialog().file().blocking_pick_file()
            })
            .await
            .map_err(|_| invalid_request("File selection failed."))?;
            let Some(path) = path else {
                return Ok(Reply {
                    rows: vec![],
                    message: "File selection cancelled.",
                });
            };
            crate::applock::require_unlocked_generation(&app, generation)?;
            let path = path
                .into_path()
                .map_err(|_| invalid_request("Invalid private file."))?;
            let token = foks_agent_client::secret_file::import_token(&path)
                .map_err(|_| invalid_request("Cannot read private bot token file."))?;
            (
                BotAction::Load { token },
                "Bot loaded for this agent session.",
            )
        }
        Action::ExportFile { operation_id } => {
            if !(BotAction::Export {
                operation_id: operation_id.clone(),
            })
            .validate()
            {
                return Err(invalid_request("Invalid bot enrollment."));
            }
            let picker = app.clone();
            let path = tauri::async_runtime::spawn_blocking(move || {
                picker
                    .dialog()
                    .file()
                    .set_file_name("bot-token.txt")
                    .blocking_save_file()
            })
            .await
            .map_err(|_| invalid_request("File selection failed."))?;
            let Some(path) = path else {
                return Ok(Reply {
                    rows: vec![],
                    message: "File selection cancelled.",
                });
            };
            crate::applock::require_unlocked_generation(&app, generation)?;
            let path = path
                .into_path()
                .map_err(|_| invalid_request("Invalid export file."))?;
            output = Some(
                foks_agent_client::secret_file::reserve_export(&path)
                    .map_err(|_| invalid_request("Choose a new private file for export."))?,
            );
            (
                BotAction::Export { operation_id },
                "Token exported once to the selected private file.",
            )
        }
        Action::List => (BotAction::List, "Enrollment history loaded."),
        Action::Unload => (
            BotAction::Unload,
            "Bot unloaded. The original token is required to load it again.",
        ),
        Action::Prepare { role, pin } => (
            BotAction::Prepare { role, pin },
            "Review the role before confirming enrollment.",
        ),
        Action::Attempt { operation_id, pin } => (
            BotAction::Attempt { operation_id, pin },
            "Enrollment status updated.",
        ),
        Action::Status { operation_id, pin } => (
            BotAction::Status { operation_id, pin },
            "Original enrollment checked.",
        ),
        Action::Cancel { operation_id } => {
            (BotAction::Cancel { operation_id }, "Enrollment cancelled.")
        }
        Action::Revoke { device_id, pin } => (
            BotAction::Revoke { device_id, pin },
            "Revocation status updated.",
        ),
    };
    if !action.validate() {
        return Err(invalid_request("Invalid bot request."));
    }
    let transport = state.agent.transport();
    let guard_app = app.clone();
    if changes_catalog {
        state.invalidate_catalog();
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        super::servers::require_transport_profile(transport.as_ref(), &profile)?;
        crate::applock::require_unlocked_generation(&guard_app, generation)?;
        let id = match &action {
            BotAction::Attempt { operation_id, .. }
            | BotAction::Status { operation_id, .. }
            | BotAction::Cancel { operation_id }
            | BotAction::Export { operation_id } => Some(operation_id.clone()),
            _ => None,
        };
        let list = matches!(&action, BotAction::List);
        let loaded = match &action {
            BotAction::Load { .. } => Some(true),
            BotAction::Unload => Some(false),
            _ => None,
        };
        let target = if let BotAction::Revoke { device_id, .. } = &action {
            Some(device_id.clone())
        } else {
            None
        };
        let mut value = transport
            .call(Operation::BotAccount {
                profile,
                account_alias: account_alias.clone(),
                action,
            })
            .map_err(AgentError::from_desktop)?;
        if let Some(mut file) = output {
            let secret = match value.get_mut("token") {
                Some(serde_json::Value::String(s)) => {
                    let secret = Zeroizing::new(s.clone());
                    s.zeroize();
                    secret
                }
                _ => return Err(invalid_response("Missing one-time export.")),
            };
            let rows = project(value["report"].take(), &account_alias, id.as_deref(), false)?;
            // Finish an authorized export even if the app locks while IPC is in flight;
            // this avoids discarding the sole secret after the agent erased its copy.
            foks_agent_client::secret_file::write_export(&mut file, &secret).map_err(|_| {
                invalid_request(
                    "Export could not be saved; revoke this token if the file is incomplete.",
                )
            })?;
            return Ok(Reply { rows, message });
        }
        if let Some(loaded) = loaded {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Selection {
                account_alias: String,
                loaded: bool,
            }
            let p: Selection = serde_json::from_value(value)
                .map_err(|_| invalid_response("Invalid bot selection."))?;
            if p.account_alias != account_alias || p.loaded != loaded {
                return Err(invalid_response("Bot selection changed account."));
            }
            return Ok(Reply {
                rows: vec![],
                message,
            });
        }
        if let Some(target) = target {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Revoke {
                operation_id: Option<String>,
                device_id: String,
                state: String,
                currently_active: bool,
            }
            let p: Revoke = serde_json::from_value(value)
                .map_err(|_| invalid_response("Invalid revocation response."))?;
            if p.device_id != target
                || p.operation_id.as_ref().is_some_and(|id| {
                    !(BotAction::Cancel {
                        operation_id: id.clone(),
                    })
                    .validate()
                })
                || !matches!(
                    p.state.as_str(),
                    "absent" | "complete" | "rejected" | "submission-unknown"
                )
            {
                return Err(invalid_response("Revocation changed credential."));
            }
            return Ok(Reply {
                rows: vec![],
                message: if p.currently_active {
                    "Bot is still active; check revocation again."
                } else {
                    "Bot is revoked."
                },
            });
        }
        Ok(Reply {
            rows: project(value, &account_alias, id.as_deref(), list)?,
            message,
        })
    })
    .await
    .map_err(|_| invalid_request("Bot request interrupted; recover the original enrollment."))??;
    crate::applock::require_unlocked_generation(&app, generation)?;
    if changes_catalog {
        state.invalidate_catalog();
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn response_binding_and_secret_projection() {
        let p = serde_json::json!({"operation_id":"1".repeat(32),"account_alias":"work","name":"abcde","device_id":format!("13{}","0".repeat(64)),"role":"OWNER","state":"complete","hardware_required":false,"export_available":true});
        assert!(project(p.clone(), "work", None, false).is_ok());
        assert!(project(p.clone(), "other", None, false).is_err());
        assert!(project(p.clone(), "work", Some(&"2".repeat(32)), false).is_err());
        let mut p = p;
        p["token"] = serde_json::json!("secret");
        assert!(project(p, "work", None, false).is_err());
    }
}
