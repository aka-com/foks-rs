//! Browser credentials stay native; the webview can only open its account's stored flow.
use super::{
    context::AppState,
    validation::{invalid_request, invalid_response, require_main_window, valid_local_name},
};
use crate::agent::AgentError;
use foks_agent_proto::{
    sso::{SsoAction, SsoProgress},
    Operation,
};
use serde::Serialize;
use tauri::{Manager as _, State};
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    operation_id: Option<String>,
    account_alias: String,
    purpose: foks_agent_proto::sso::SsoPurpose,
    account_status: Option<foks_agent_proto::sso::SsoAccountStatusView>,
    state: String,
    browser_available: bool,
    expires_at_ms: u64,
    service_access: bool,
}
fn validate(p: &SsoProgress, alias: &str) -> Result<(), AgentError> {
    if p.account_alias != alias
        || p.operation_id.as_ref().is_some_and(|id| {
            !(SsoAction::Status {
                operation_id: id.clone(),
            })
            .validate()
        })
        || p.operation_id.is_none() != p.account_status.is_some()
        || (p.operation_id.is_none() && p.browser_url.is_some())
        || !matches!(
            p.state.as_str(),
            "prepared"
                | "waiting"
                | "ready"
                | "submitting"
                | "complete"
                | "cancelled"
                | "expired"
                | "submission-unknown"
                | "rejected"
                | "denied"
                | "provider-unavailable"
                | "reauthentication-required"
                | "hardware-verification-required"
                | "service-unavailable"
                | "device-only"
                | "link-needed"
                | "locked-out"
                | "linked"
                | "not-eligible"
        )
        || p.browser_url.as_ref().is_some_and(|s| s.len() > 8192)
    {
        return Err(invalid_response("Invalid account authentication progress."));
    }
    Ok(())
}
fn public_progress(result: SsoProgress) -> Progress {
    Progress {
        operation_id: result.operation_id,
        account_alias: result.account_alias,
        purpose: result.purpose,
        account_status: result.account_status,
        state: result.state,
        browser_available: result.browser_url.is_some(),
        expires_at_ms: result.expires_at_ms,
        service_access: result.service_access,
    }
}
#[tauri::command]
pub async fn sso_request(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    action: SsoAction,
) -> Result<Progress, AgentError> {
    require_main_window(&webview)?;
    let generation = crate::applock::unlocked_generation(webview.app_handle())?;
    if !valid_local_name(&profile) || !valid_local_name(&account_alias) || !action.validate() {
        return Err(invalid_request("Invalid account authentication request."));
    }
    let state = state.for_profile(&profile)?;
    let inspecting = matches!(
        action,
        SsoAction::Status { .. } | SsoAction::AccountStatus { .. }
    );
    let _mutation = if inspecting {
        None
    } else {
        Some(state.begin_mutation()?)
    };
    let transport = state.agent.transport();
    let expected = account_alias.clone();
    let changes_account = matches!(
        action,
        SsoAction::FinishSignup { .. } | SsoAction::FinishYubiSignup { .. }
    );
    if changes_account {
        state.invalidate_catalog();
    }
    let expected_id = match &action {
        SsoAction::Begin { .. }
        | SsoAction::AccountStatus { .. }
        | SsoAction::BeginYubiSignup { .. } => None,
        SsoAction::FinishYubiSignup { operation_id, .. }
        | SsoAction::Status { operation_id }
        | SsoAction::Poll { operation_id }
        | SsoAction::Cancel { operation_id }
        | SsoAction::FinishLogin { operation_id, .. }
        | SsoAction::FinishSignup { operation_id, .. } => Some(operation_id.clone()),
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        super::servers::require_transport_profile(transport.as_ref(), &profile)?;
        let value = transport
            .call(Operation::Sso {
                profile,
                account_alias,
                action,
            })
            .map_err(AgentError::from_desktop)?;
        let p: SsoProgress = serde_json::from_value(value)
            .map_err(|_| invalid_response("Invalid authentication response."))?;
        validate(&p, &expected)?;
        if expected_id.is_some_and(|id| Some(id) != p.operation_id) {
            return Err(invalid_response("Authentication handle changed."));
        }
        Ok::<_, AgentError>(p)
    })
    .await
    .map_err(|_| {
        if !inspecting {
            super::execution::ambiguous_worker_failure(
                &state,
                "Authentication interrupted; resume its existing flow.",
            )
        } else {
            invalid_request("Authentication interrupted; resume its existing flow.")
        }
    })?
    .map_err(|error| {
        if !inspecting {
            super::execution::observe_mutation_error(&state, error)
        } else {
            error
        }
    })?;
    crate::applock::require_unlocked_generation(webview.app_handle(), generation)?;
    if changes_account {
        state.invalidate_catalog();
    }
    Ok(public_progress(result))
}
#[tauri::command]
pub async fn open_sso_browser(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    operation_id: String,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    let generation = crate::applock::unlocked_generation(webview.app_handle())?;
    let action = SsoAction::Status {
        operation_id: operation_id.clone(),
    };
    if !valid_local_name(&profile) || !valid_local_name(&account_alias) || !action.validate() {
        return Err(invalid_request("Invalid authentication handle."));
    }
    let expected = account_alias.clone();
    let value = crate::agent::success_value(
        state
            .agent
            .call(Operation::Sso {
                profile,
                account_alias,
                action,
            })
            .await?,
    )?;
    let p: SsoProgress = serde_json::from_value(value)
        .map_err(|_| invalid_response("Invalid authentication response."))?;
    validate(&p, &expected)?;
    if p.operation_id.as_deref() != Some(operation_id.as_str()) || p.state != "waiting" {
        return Err(invalid_request("This browser flow is no longer waiting."));
    }
    let url = p.browser_url.ok_or_else(|| {
        invalid_request("The start response was interrupted. Cancel this flow and begin again.")
    })?;
    crate::applock::require_unlocked_generation(webview.app_handle(), generation)?;
    super::chat::open_chat_link(webview, url).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_progress_removes_browser_bearer_and_rejects_wrong_account() {
        let p = SsoProgress {
            operation_id: Some("a".repeat(32)),
            account_alias: "work".into(),
            purpose: foks_agent_proto::sso::SsoPurpose::Reauthenticate,
            account_status: None,
            state: "waiting".into(),
            browser_url: Some("https://host.example/oauth2/start?state=secret".into()),
            expires_at_ms: 100,
            service_access: false,
        };
        assert!(validate(&p, "other").is_err());
        assert!(validate(&p, "work").is_ok());
        let public = serde_json::to_string(&public_progress(p)).unwrap();
        assert!(!public.contains("secret"));
        assert!(!public.contains("https"));
        assert!(public.contains("browserAvailable"));
    }
}
