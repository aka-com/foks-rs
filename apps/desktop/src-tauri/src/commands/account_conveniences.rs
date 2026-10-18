//! Native account rename boundary; PINs are transient and handles stay account scoped.
use super::validation::{invalid_request, invalid_response, require_main_window, valid_local_name};
use crate::agent::AgentError;
use foks_agent_proto::{
    account::{RenameAction, RenameProgress},
    Operation,
};
use tauri::Manager as _;
#[tauri::command]
pub async fn rename_account_request(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    profile: String,
    account_alias: String,
    action: Option<RenameAction>,
) -> Result<Vec<RenameProgress>, AgentError> {
    require_main_window(&webview)?;
    let generation = crate::applock::unlocked_generation(webview.app_handle())?;
    if !valid_local_name(&profile)
        || !valid_local_name(&account_alias)
        || action.as_ref().is_some_and(|a| !a.validate())
    {
        return Err(invalid_request("Invalid account rename request."));
    }
    let state = state.for_profile(&profile)?;
    let changed = action.is_some();
    let _mutation = if changed {
        Some(state.begin_mutation()?)
    } else {
        None
    };
    let transport = state.transport()?;
    if changed {
        state.invalidate_catalog();
    }
    let result = {
        let deadline = state.deadline()?.clone();
        super::deadline::run(
            &deadline,
            changed,
            move || {
                super::servers::require_transport_profile(transport.as_ref(), &profile)?;
                let expected = account_alias.clone();
                let expected_id = action
                    .as_ref()
                    .and_then(|a| a.operation_id())
                    .map(str::to_owned);
                let operation = match action {
                    Some(action) => Operation::RenameAccount {
                        profile,
                        account_alias,
                        action,
                    },
                    None => Operation::ListAccountRenames {
                        profile,
                        account_alias,
                    },
                };
                let value = transport
                    .call(operation)
                    .map_err(AgentError::from_desktop)?;
                decode_progress(value, changed, &expected, expected_id.as_deref())
            },
            |_| {},
        )
        .await
    }
    .map_err(|error| {
        if changed {
            super::execution::observe_mutation_error(&state, error)
        } else {
            error
        }
    })?;
    crate::applock::require_unlocked_generation(webview.app_handle(), generation)?;
    if changed {
        state.invalidate_catalog();
    }
    Ok(result)
}

fn decode_progress(
    value: serde_json::Value,
    changed: bool,
    expected: &str,
    expected_id: Option<&str>,
) -> Result<Vec<RenameProgress>, AgentError> {
    let progress: Vec<RenameProgress> = if changed {
        vec![serde_json::from_value(value)
            .map_err(|_| invalid_response("Invalid rename response."))?]
    } else {
        serde_json::from_value(value).map_err(|_| invalid_response("Invalid rename history."))?
    };
    if progress.len() > 160
        || progress.iter().any(|p| {
            !p.validate()
                || p.account_alias != expected
                || expected_id.is_some_and(|id| id != p.operation_id)
        })
    {
        return Err(invalid_response(
            "Rename response changed account or operation.",
        ));
    }
    Ok::<_, AgentError>(progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_response_is_strictly_bound_to_selected_account_and_handle() {
        let value = serde_json::json!({"operation_id":"1".repeat(32),"account_alias":"work","state":"prepared","target":"alice","current_username":"bob","hardware_required":false});
        assert!(decode_progress(value.clone(), true, "work", Some(&"1".repeat(32))).is_ok());
        assert!(decode_progress(value.clone(), true, "other", None).is_err());
        assert!(decode_progress(value.clone(), true, "work", Some(&"2".repeat(32))).is_err());
        let mut secret = value.clone();
        secret["pin"] = serde_json::json!("654321");
        assert!(decode_progress(secret, true, "work", None).is_err());
        assert!(decode_progress(serde_json::json!([value]), false, "work", None).is_ok());
    }
}

#[derive(serde::Serialize)]
pub struct LocalAliasDto {
    pub store: String,
    pub alias: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalAliasResponse {
    profile: String,
    account_alias: String,
    label: String,
}

fn local_alias_response(
    value: serde_json::Value,
    account: &foks_agent_proto::AccountStoreRef,
    label: &str,
    store: String,
) -> Result<LocalAliasDto, AgentError> {
    let response: LocalAliasResponse = serde_json::from_value(value)
        .map_err(|_| invalid_response("Invalid local alias response."))?;
    if response.profile != account.profile
        || response.account_alias != account.account_alias
        || response.label != label
    {
        return Err(invalid_response(
            "Local alias response belongs to a different account or label.",
        ));
    }
    Ok(LocalAliasDto {
        store,
        alias: response.label,
    })
}

#[tauri::command]
pub async fn set_local_account_alias(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    account_store_id: String,
    label: String,
) -> Result<LocalAliasDto, AgentError> {
    require_main_window(&webview)?;
    let generation = crate::applock::unlocked_generation(webview.app_handle())?;
    foks_client_app::validate_local_alias(&label).map_err(|_| invalid_request("Enter a local alias of 1–64 UTF-8 bytes without surrounding whitespace or control characters."))?;
    let state = state.for_local_aliases()?;
    let _mutation = state.begin_mutation()?;
    let account = state.local_account(&account_store_id)?;
    let profile = account.profile.clone();
    let transport = state.transport()?;
    state.invalidate_alias_metadata(&profile);
    let result = {
        let deadline = state.deadline()?.clone();
        super::deadline::run(
            &deadline,
            true,
            move || {
                let value = transport
                    .call(Operation::SetLocalAccountAlias {
                        profile: account.profile.clone(),
                        account_alias: account.account_alias.clone(),
                        label: label.clone(),
                    })
                    .map_err(AgentError::from_desktop)?;
                local_alias_response(value, &account, &label, account_store_id)
            },
            |_| {},
        )
        .await
    };
    state.invalidate_alias_metadata(&profile);
    crate::applock::require_unlocked_generation(webview.app_handle(), generation)?;
    result
}

#[cfg(test)]
mod local_alias_tests {
    use super::*;
    #[test]
    fn local_alias_response_is_bound_and_rejects_extra_fields() {
        let account = foks_agent_proto::AccountStoreRef {
            profile: "local".into(),
            account_alias: "work".into(),
        };
        let good = serde_json::json!({"profile":"local", "account_alias":"work", "label":"Office"});
        assert!(local_alias_response(good.clone(), &account, "Office", "store".into()).is_ok());
        assert!(local_alias_response(good.clone(), &account, "Other", "store".into()).is_err());
        for field in ["profile", "account_alias"] {
            let mut bad = good.clone();
            bad[field] = serde_json::json!("other");
            assert!(local_alias_response(bad, &account, "Office", "store".into()).is_err());
        }
        let mut bad = good;
        bad["pin"] = serde_json::json!("secret");
        assert!(local_alias_response(bad, &account, "Office", "store".into()).is_err());
    }
}

/// Whether the account's adapter clock admits new MCP data writes.
#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterClockDto {
    pub admission: foks_agent_proto::data::AdapterClockAdmission,
    /// The computer's wall time when the agent checked, in Unix seconds.
    pub local_time: u64,
    /// When admission is `behind`, the Unix time at which writes resume.
    pub resumes_at: Option<u64>,
}

impl From<foks_agent_proto::data::AdapterClockStatus> for AdapterClockDto {
    fn from(status: foks_agent_proto::data::AdapterClockStatus) -> Self {
        Self {
            admission: status.admission,
            local_time: status.local_time,
            resumes_at: (status.admission == foks_agent_proto::data::AdapterClockAdmission::Behind)
                .then_some(status.reject_issued_before)
                .flatten(),
        }
    }
}

fn adapter_clock_status(
    value: serde_json::Value,
) -> Result<foks_agent_proto::data::AdapterClockStatus, AgentError> {
    serde_json::from_value(value).map_err(|_| invalid_response("Invalid adapter clock status."))
}

/// Checks that the agent previewed and re-anchored exactly the time this
/// process proposed, and that the confirmation carried the preview's digest.
fn adapter_clock_repaired(
    preview: serde_json::Value,
    repair: impl FnOnce(String) -> Result<serde_json::Value, AgentError>,
    proposed: u64,
) -> Result<(), AgentError> {
    let preview: foks_agent_proto::data::AdapterClockPreview = serde_json::from_value(preview)
        .map_err(|_| invalid_response("Invalid adapter clock preview."))?;
    if preview.proposed_wall != proposed {
        return Err(invalid_response("Adapter clock preview changed the time."));
    }
    let repaired: foks_agent_proto::data::AdapterClockRepair =
        serde_json::from_value(repair(preview.confirmation_digest.clone())?)
            .map_err(|_| invalid_response("Invalid adapter clock repair."))?;
    if repaired.preview != preview || repaired.resulting_floor != proposed {
        return Err(invalid_response(
            "Adapter clock repair does not match its preview.",
        ));
    }
    Ok(())
}

/// Reads the adapter clock or, with `confirm`, re-anchors it at this
/// computer's current time. The renderer never supplies the time: the person
/// confirms that the clock this process reads is correct.
#[tauri::command]
pub async fn adapter_clock_request(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    profile: String,
    account_alias: String,
    confirm: bool,
) -> Result<AdapterClockDto, AgentError> {
    use foks_agent_proto::data::{AdapterClockRepairAction, ReceiptClockDomain};
    require_main_window(&webview)?;
    let generation = crate::applock::unlocked_generation(webview.app_handle())?;
    if !valid_local_name(&profile) || !valid_local_name(&account_alias) {
        return Err(invalid_request("Invalid adapter clock request."));
    }
    let state = state.for_profile(&profile)?;
    let _mutation = if confirm {
        Some(state.begin_mutation()?)
    } else {
        None
    };
    let transport = state.transport()?;
    let result = super::deadline::run(
        state.deadline()?,
        confirm,
        move || {
            super::servers::require_transport_profile(transport.as_ref(), &profile)?;
            if confirm {
                let proposed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| invalid_request("This computer's clock is before 1970."))?
                    .as_secs();
                let repair = |action| {
                    transport
                        .call(Operation::RepairAdapterClock {
                            profile: profile.clone(),
                            account_alias: account_alias.clone(),
                            domain: ReceiptClockDomain::Adapter,
                            action,
                        })
                        .map_err(AgentError::from_desktop)
                };
                adapter_clock_repaired(
                    repair(AdapterClockRepairAction::Preview {
                        unix_seconds: proposed,
                    })?,
                    |confirmation_digest| {
                        repair(AdapterClockRepairAction::Confirm {
                            unix_seconds: proposed,
                            confirmation_digest,
                        })
                    },
                    proposed,
                )?;
            }
            let status = adapter_clock_status(
                transport
                    .call(Operation::AdapterClockStatus {
                        profile,
                        account_alias,
                        domain: ReceiptClockDomain::Adapter,
                    })
                    .map_err(AgentError::from_desktop)?,
            )?;
            Ok::<_, AgentError>(AdapterClockDto::from(status))
        },
        |_| {},
    )
    .await
    .map_err(|error| {
        if confirm {
            super::execution::observe_mutation_error(&state, error)
        } else {
            error
        }
    });
    crate::applock::require_unlocked_generation(webview.app_handle(), generation)?;
    result
}

#[cfg(test)]
mod adapter_clock_tests {
    use super::*;
    use foks_agent_proto::data::AdapterClockAdmission;

    fn preview(proposed: u64) -> serde_json::Value {
        serde_json::json!({
            "host_id": "01", "user_id": "02", "anchor_wall": 10,
            "validated_time_floor": 10, "reject_issued_before": 0,
            "proposed_wall": proposed, "confirmation_digest": "ab".repeat(32),
        })
    }

    #[test]
    fn repair_confirms_only_the_previewed_digest_and_time() {
        let confirm = |digest: String, floor: u64| {
            assert_eq!(digest, "ab".repeat(32));
            Ok(serde_json::json!({
                "preview": preview(100), "resulting_floor": floor,
                "checkpoint_digest": "cd".repeat(32),
            }))
        };
        assert!(adapter_clock_repaired(preview(100), |d| confirm(d, 100), 100).is_ok());
        assert!(adapter_clock_repaired(preview(100), |d| confirm(d, 99), 100).is_err());
        assert!(adapter_clock_repaired(preview(99), |_| unreachable!(), 100).is_err());
        let mut extra = preview(100);
        extra["pin"] = serde_json::json!("secret");
        assert!(adapter_clock_repaired(extra, |_| unreachable!(), 100).is_err());
    }

    #[test]
    fn status_serializes_for_the_renderer() {
        let status = adapter_clock_status(serde_json::json!({
            "host_id": "01", "user_id": "02",
            "admission": "paused", "local_time": 100,
            "validated_time_floor": 90, "reject_issued_before": 50,
        }))
        .unwrap();
        assert_eq!(status.admission, AdapterClockAdmission::Paused);
        assert_eq!(
            serde_json::to_value(AdapterClockDto::from(status)).unwrap(),
            serde_json::json!({"admission": "paused", "localTime": 100, "resumesAt": null})
        );
        let behind = adapter_clock_status(serde_json::json!({
            "host_id": "01", "user_id": "02",
            "admission": "behind", "local_time": 100,
            "validated_time_floor": 100, "reject_issued_before": 3_700,
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(AdapterClockDto::from(behind)).unwrap(),
            serde_json::json!({"admission": "behind", "localTime": 100, "resumesAt": 3_700})
        );
        assert!(adapter_clock_status(serde_json::json!({"admission": "open"})).is_err());
    }
}
