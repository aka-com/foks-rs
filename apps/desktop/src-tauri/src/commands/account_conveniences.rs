//! Native account rename boundary; PINs are transient and handles stay account scoped.
use super::{
    context::AppState,
    validation::{invalid_request, invalid_response, require_main_window, valid_local_name},
};
use crate::agent::AgentError;
use foks_agent_proto::{
    account::{RenameAction, RenameProgress},
    Operation,
};
use tauri::{Manager as _, State};
#[tauri::command]
pub async fn rename_account_request(
    webview: tauri::Webview,
    state: State<'_, AppState>,
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
    let _mutation = state.begin_mutation()?;
    let transport = state.agent.transport();
    let changed = action.is_some();
    if changed {
        state.invalidate_catalog();
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|_| invalid_request("Rename interrupted; refresh its original operation."))??;
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
    state: State<'_, AppState>,
    account_store_id: String,
    label: String,
) -> Result<LocalAliasDto, AgentError> {
    require_main_window(&webview)?;
    let generation = crate::applock::unlocked_generation(webview.app_handle())?;
    foks_client_app::validate_local_alias(&label).map_err(|_| invalid_request("Enter a local alias of 1–64 UTF-8 bytes without surrounding whitespace or control characters."))?;
    let _mutation = state.begin_mutation()?;
    let account = state.local_account(&account_store_id)?;
    let transport = state.agent.transport();
    state.invalidate_catalog();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let value = transport
            .call(Operation::SetLocalAccountAlias {
                profile: account.profile.clone(),
                account_alias: account.account_alias.clone(),
                label: label.clone(),
            })
            .map_err(AgentError::from_desktop)?;
        local_alias_response(value, &account, &label, account_store_id)
    })
    .await
    .map_err(|_| {
        invalid_request("Local alias update interrupted. Refresh the account before trying again.")
    })?;
    state.invalidate_catalog();
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
