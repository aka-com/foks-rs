//! Native invitation boundary: opaque handles and verified identity, no capabilities.
use super::{
    context::AppState,
    validation::{invalid_request, invalid_response, require_main_window, valid_local_name},
};
use crate::agent::AgentError;
use foks_agent_proto::{invitations::InvitationAction, Operation, SecretString};
use serde::{Deserialize, Serialize};
use tauri::{Manager as _, State};
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRole {
    kind: u8,
    visibility: i16,
}
#[derive(Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
struct Row {
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    joiner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invite: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    joiner_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<StoredRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_role: Option<StoredRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_generations: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    membership: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    membership_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery_acknowledged: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hardware_required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    possibly_truncated: Option<bool>,
    /// The number of pending inbox rows, from the count-only action. Bounded
    /// by the same page the full inbox reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<Vec<Row>>,
}
fn decode(value: serde_json::Value) -> Result<serde_json::Value, AgentError> {
    if serde_json::to_vec(&value)
        .map_err(|_| invalid_response("Invalid invitation response."))?
        .len()
        > 2 * 1024 * 1024
    {
        return Err(invalid_response("Invitation response too large."));
    }
    let rows: Vec<Row> = if value.is_array() {
        serde_json::from_value(value.clone())
    } else {
        serde_json::from_value(value.clone()).map(|r| vec![r])
    }
    .map_err(|_| invalid_response("Invalid invitation response."))?;
    fn valid(r: &Row, depth: u8) -> bool {
        depth <= 1
            && [&r.operation_id, &r.request_id].iter().all(|id| {
                id.as_ref()
                    .is_none_or(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            })
            && [&r.team_id, &r.host_id, &r.joiner_id].iter().all(|id| {
                id.as_ref()
                    .is_none_or(|s| s.len() == 66 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            })
            && [&r.name, &r.username, &r.error]
                .iter()
                .all(|s| s.as_ref().is_none_or(|s| s.len() <= 1024))
            && r.invite
                .as_ref()
                .is_none_or(|s| s.len() <= 256 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
            && r.count.is_none_or(|count| count <= 2000)
            && r.rows
                .as_ref()
                .is_none_or(|rows| rows.len() <= 2000 && rows.iter().all(|r| valid(r, depth + 1)))
    }
    if rows.len() > 2000 || !rows.iter().all(|r| valid(r, 0)) {
        return Err(invalid_response("Invalid invitation bounds."));
    }
    if value.is_array() {
        serde_json::to_value(rows)
    } else {
        serde_json::to_value(rows.into_iter().next().unwrap())
    }
    .map_err(|_| invalid_response("Invalid invitation response."))
}
#[tauri::command]
pub async fn invitation_request(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    action: InvitationAction,
    pin: Option<SecretString>,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    let generation = crate::applock::unlocked_generation(webview.app_handle())?;
    if !valid_local_name(&profile)
        || !valid_local_name(&account_alias)
        || !action.validate()
        || pin
            .as_ref()
            .is_some_and(|p| p.expose().is_empty() || p.expose().len() > 32)
    {
        return Err(invalid_request("Invalid invitation action."));
    }
    let state = if action.remote_profile().is_some() {
        state.inner().clone()
    } else {
        state.for_profile(&profile)?
    };
    let changes_catalog = action.changes_catalog();
    let _mutation = state.begin_catalog_action(changes_catalog)?;
    let transport = state.agent.transport();
    let value = tauri::async_runtime::spawn_blocking(move || {
        super::servers::require_transport_profile(transport.as_ref(), &profile)?;
        if let Some(remote) = action.remote_profile() {
            super::servers::require_transport_profile(transport.as_ref(), remote)?;
        }
        let value = transport
            .call(Operation::Invitations {
                profile,
                account_alias,
                action,
                pin,
            })
            .map_err(AgentError::from_desktop)?;
        decode(value)
    })
    .await
    .map_err(|_| {
        if changes_catalog {
            super::execution::ambiguous_worker_failure(
                &state,
                "Invitation interrupted; check its original operation.",
            )
        } else {
            invalid_request("Invitation interrupted; check its original operation.")
        }
    })?
    .map_err(|error| {
        if changes_catalog {
            super::execution::observe_mutation_error(&state, error)
        } else {
            error
        }
    })?;
    crate::applock::require_unlocked_generation(webview.app_handle(), generation)?;
    if changes_catalog {
        state.invalidate_catalog();
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invitation_boundary_rejects_capabilities_and_nested_overflow() {
        assert!(decode(serde_json::json!({"permission":[1,2,3]})).is_err());
        assert!(decode(serde_json::json!({"rows":[{"receipt":[1,2,3]}]})).is_err());
        assert!(decode(serde_json::json!({"rows":[{"rows":[]}]})).is_ok());
        assert!(decode(serde_json::json!({"rows":[{"rows":[{"state":"pending"}]}]})).is_err());
        assert!(decode(
            serde_json::json!({"operation_id":"a".repeat(32),"state":"submission-unknown"})
        )
        .is_ok());
    }
}
