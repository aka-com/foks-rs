//! One-time bot secrets cross native IPC and a private file, never the webview.
use super::validation::{invalid_request, invalid_response, require_main_window, valid_local_name};
use crate::agent::AgentError;
use foks_agent_proto::{bot::BotAction, KvRole, Operation, SecretString};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::Manager as _;
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
fn save_export_until(
    file: &mut foks_agent_client::secret_file::ExportReservation,
    secret: &str,
    interval: &crate::applock::UnlockedInterval,
    deadline: &foks_local_deadline::Deadline,
    published: &AtomicBool,
) -> Result<(), AgentError> {
    let export_error = |_| {
        invalid_request(
        "Export was not acknowledged. The original enrollment retains its token; check the selected file before retrying.",
    )
    };
    deadline.check().map_err(super::deadline::clock_error)?;
    interval.check()?;
    foks_agent_client::secret_file::prepare_export(file, secret).map_err(export_error)?;
    interval.publish(|| {
        deadline.check().map_err(super::deadline::clock_error)?;
        foks_agent_client::secret_file::publish_export(file).map_err(export_error)?;
        published.store(true, Ordering::Release);
        Ok(())
    })?;
    // Directory sync stays outside the publication gate. A timeout after the
    // link leaves the published file plus the agent's recoverable retained offer.
    foks_agent_client::secret_file::finish_export(file).map_err(export_error)?;
    deadline.check().map_err(super::deadline::clock_error)
}

#[tauri::command]
pub async fn bot_account_request(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    profile: String,
    account_alias: String,
    action: Action,
) -> Result<Reply, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let generation = crate::applock::unlocked_generation(&app)?;
    let interval = crate::applock::unlocked_interval(&app)?;
    if !valid_local_name(&profile) || !valid_local_name(&account_alias) {
        return Err(invalid_request("Invalid bot account."));
    }
    let exporting = matches!(&action, Action::ExportFile { .. });
    let published = Arc::new(AtomicBool::new(false));
    let annotate_export = |mut error: AgentError| {
        if exporting {
            error.message = if published.load(Ordering::Acquire) {
                "The token file was saved, but export completion was interrupted. Verify the file before retrying acknowledgment of this enrollment."
            } else {
                "Export was interrupted. Check the selected file before retrying this enrollment; publication or acknowledgment may be incomplete."
            }.into();
            error.ambiguous = true;
            error.retryable = false;
        }
        error
    };
    let dialog = matches!(&action, Action::LoadFile | Action::ExportFile { .. });
    let mut state = if dialog {
        state.for_profile(&profile)?
    } else {
        state
            .begin_operation(foks_agent_proto::DeadlineClass::Ordinary)?
            .for_profile(&profile)?
    };
    let changes_catalog = !matches!(action, Action::List);
    let _mutation = if changes_catalog {
        Some(state.begin_mutation()?)
    } else {
        None
    };
    let mut output = None;
    let mut input = None;
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
            state = state.begin_operation(foks_agent_proto::DeadlineClass::Ordinary)?;
            input = Some(path);
            // Replaced with the imported token inside bounded worker admission.
            (BotAction::List, "Bot loaded for this agent session.")
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
            state = state.begin_operation(foks_agent_proto::DeadlineClass::Ordinary)?;
            output = Some(path);
            (
                BotAction::Export { operation_id },
                "Token saved to the selected private file.",
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
    let transport = state.transport()?;
    let guard_app = app.clone();
    if changes_catalog {
        state.invalidate_catalog();
    }
    let result = {
        let deadline = state.deadline()?.clone();
        let worker_deadline = deadline.clone();
        let worker_published = published.clone();
        super::deadline::run(
            &deadline,
            true,
            move || {
                super::servers::require_transport_profile(transport.as_ref(), &profile)?;
                crate::applock::require_unlocked_generation(&guard_app, generation)?;
                let action = if let Some(path) = input {
                    let token = foks_agent_client::secret_file::import_token(&path)
                        .map_err(|_| invalid_request("Cannot read private bot token file."))?;
                    worker_deadline
                        .check()
                        .map_err(super::deadline::clock_error)?;
                    interval.check()?;
                    BotAction::Load { token }
                } else {
                    action
                };
                let output = output
                    .map(|path| {
                        foks_agent_client::secret_file::reserve_export(&path)
                            .map_err(|_| invalid_request("Choose a new private file for export."))
                    })
                    .transpose()?;
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
                        profile: profile.clone(),
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
                    project(value["report"].take(), &account_alias, id.as_deref(), false)?;
                    save_export_until(
                        &mut file,
                        &secret,
                        &interval,
                        &worker_deadline,
                        &worker_published,
                    )?;
                    let completed = transport
                        .call(Operation::BotAccount {
                            profile,
                            account_alias: account_alias.clone(),
                            action: BotAction::CompleteExport {
                                operation_id: id
                                    .clone()
                                    .ok_or_else(|| invalid_response("Missing export identity."))?,
                            },
                        })
                        .map_err(AgentError::from_desktop)?;
                    let rows = project(completed, &account_alias, id.as_deref(), false)?;
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
            },
            |_| {},
        )
        .await
    }
    .map_err(|error| {
        let error = annotate_export(error);
        if changes_catalog {
            super::execution::observe_mutation_error(&state, error)
        } else {
            error
        }
    })?;
    crate::applock::require_unlocked_generation(&app, generation).map_err(annotate_export)?;
    if changes_catalog {
        state.invalidate_catalog();
    }
    state
        .deadline()?
        .check()
        .map_err(super::deadline::clock_error)
        .map_err(annotate_export)?;
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

#[cfg(test)]
mod export_publication_tests {
    use super::*;
    use std::{
        sync::{mpsc, Mutex},
        time::{Duration, Instant},
    };

    #[test]
    fn lock_before_export_publication_preserves_destination_and_removes_temp() {
        for unlock in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("bot-token.txt");
            let reservation = foks_agent_client::secret_file::reserve_export(&destination).unwrap();
            let lock = crate::applock::AppLock::unlocked_for_test();
            let (reached_tx, reached_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            let resume_rx = Mutex::new(resume_rx);
            let interval = lock.interval_for_test().with_checkpoint(move |phase| {
                if phase == "before-publication" {
                    reached_tx.send(()).unwrap();
                    resume_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
            });
            let worker = std::thread::spawn(move || {
                let mut reservation = reservation;
                let deadline = foks_local_deadline::Deadline::capture(
                    Instant::now() + Duration::from_secs(30),
                )
                .unwrap();
                save_export_until(
                    &mut reservation,
                    &"x".repeat(37),
                    &interval,
                    &deadline,
                    &AtomicBool::new(false),
                )
            });
            reached_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            lock.invalidate_for_test(unlock);
            resume_tx.send(()).unwrap();
            assert_eq!(worker.join().unwrap().unwrap_err().code, "app-locked");
            assert!(!destination.exists());
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }
}
