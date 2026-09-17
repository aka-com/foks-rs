//! First-run account enrollment, recovery, device pairing, and Go CLI handoff.

use crate::agent::{success_value, AgentError};
use crate::commands::accounts::{account_sync_response, passphrase_response};
use crate::commands::application::AgentStatusDto;
use crate::commands::context::AppState;
use crate::commands::execution::{
    ambiguous_mutation_response, ambiguous_worker_failure, apply_operation_value,
    apply_pending_operation_value, apply_profile_operation_value, MutationKind,
};
use crate::commands::groups::GoProfileCandidateResponse;
use crate::commands::servers::{
    check_existing_or_add_profile, require_transport_profile, valid_probe_target, CheckedProfileDto,
};
use crate::commands::types::MutationDto;
use crate::commands::validation::{
    bounded_field, bounded_local_name, bounded_secret, confirmed_passphrase, deserialize_secret,
    invalid_request, invalid_response, optional_bounded_field, optional_confirmed_passphrase,
    pairing_phrase, positive_recovery_serial, require_main_window, require_response_row_cap,
    serialize_secret, valid_device_member_id_hex, valid_go_candidate_id, valid_local_name,
    valid_response_text, valid_typed_entity_id_hex, BACKUP_ID_PREFIX, DEVICE_ID_PREFIX,
    HOST_ID_PREFIX, MAXIMUM_FIRST_RUN_ROWS, MAXIMUM_INVITE_BYTES, MAXIMUM_RECOVERY_PHRASE_BYTES,
    USER_ID_PREFIX,
};
use foks_agent_proto::{
    CredentialBackend, Operation, PendingOperationKind, PendingOperationSummary, SecretString,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::State;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOperationDto {
    pub kind: &'static str,
    pub alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl TryFrom<PendingOperationSummary> for PendingOperationDto {
    type Error = AgentError;

    fn try_from(summary: PendingOperationSummary) -> Result<Self, Self::Error> {
        if !valid_local_name(&summary.alias)
            || summary
                .target
                .as_ref()
                .is_some_and(|target| !valid_response_text(target, 256))
        {
            return Err(invalid_response(
                "The agent returned an invalid pending operation.",
            ));
        }
        let kind = match summary.kind {
            PendingOperationKind::AccountSignup => "account-signup",
            PendingOperationKind::DeviceProvision => "device-provision",
            PendingOperationKind::PairingOffer => "pairing-offer",
            PendingOperationKind::PairingAcceptance => "pairing-acceptance",
            PendingOperationKind::AccountRecovery => "account-recovery",
            PendingOperationKind::YubiEnrollment => "yubi-enrollment",
            PendingOperationKind::TeamCreation => "team-creation",
            PendingOperationKind::TeamMemberAddition => "team-member-addition",
            PendingOperationKind::TeamMemberEdit => "team-member-edit",
            PendingOperationKind::FederationExpulsion => "federation-expulsion",
            PendingOperationKind::TeamRekey => "team-rekey",
        };
        Ok(Self {
            kind,
            alias: summary.alias,
            target: summary.target,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryResponse {
    alias: String,
    device_id_hex: String,
    user_chain_sequence: u64,
}

pub(super) fn pairing_offer_response(
    value: serde_json::Value,
    expected_account: &str,
) -> Result<PairingOfferDto, AgentError> {
    let response: PairingOfferResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.account_alias != expected_account
        || response.phrase.len() > MAXIMUM_RECOVERY_PHRASE_BYTES
        || foks_crypto::KexSecret::from_phrase(response.phrase.as_str()).is_err()
    {
        return Err(invalid_response(
            "The received device-pairing phrase is invalid.",
        ));
    }
    Ok(PairingOfferDto {
        account_alias: response.account_alias,
        phrase: response.phrase,
    })
}

pub(super) fn device_provision_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<DeviceProvisionDto, AgentError> {
    let response: DeviceProvisionResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias
        || !valid_typed_entity_id_hex(&response.device_id_hex, DEVICE_ID_PREFIX)
    {
        return Err(invalid_response(
            "The agent returned an invalid paired-device response.",
        ));
    }
    Ok(DeviceProvisionDto {
        alias: response.alias,
        device_id: response.device_id_hex,
        user_chain_sequence: response.user_chain_sequence,
    })
}

pub(super) fn recovery_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<MutationDto, AgentError> {
    let report: RecoveryResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if report.alias != expected_alias
        || !valid_typed_entity_id_hex(&report.device_id_hex, DEVICE_ID_PREFIX)
    {
        return Err(invalid_response(
            "The agent returned an invalid account recovery response.",
        ));
    }
    let _sequence = report.user_chain_sequence;
    Ok(MutationDto { applied: true })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoProfilePairingRequest {
    candidate_id: String,
    profile: String,
    target_alias: String,
    device_name: String,
    phrase: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoProfileDiscoveryResponse {
    installed: bool,
    candidates: Vec<GoProfileCandidateResponse>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoProfileCandidateDto {
    pub candidate_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_hint: Option<String>,
    pub host_id: String,
    pub user_id: String,
    pub device_id: String,
    pub role: String,
    pub storage_kind: String,
    pub hidden: bool,
    pub provisional: bool,
    pub pairable: bool,
    pub copyable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoProfileDiscoveryDto {
    pub installed: bool,
    pub candidates: Vec<GoProfileCandidateDto>,
}

pub(super) fn go_profile_discovery_response(
    value: serde_json::Value,
) -> Result<GoProfileDiscoveryDto, AgentError> {
    let response: GoProfileDiscoveryResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.candidates.len() > MAXIMUM_FIRST_RUN_ROWS
        || (!response.installed && !response.candidates.is_empty())
    {
        return Err(invalid_response(
            "The agent returned an inconsistent legacy installation status.",
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    let candidates = response
        .candidates
        .into_iter()
        .map(|candidate| {
            let candidate_id_valid = valid_go_candidate_id(&candidate.candidate_id);
            let device_id_valid = valid_device_member_id_hex(&candidate.device_id_hex)
                || valid_typed_entity_id_hex(&candidate.device_id_hex, BACKUP_ID_PREFIX)
                || valid_typed_entity_id_hex(&candidate.device_id_hex, "13");
            let role_valid = matches!(candidate.role.as_str(), "none" | "admin" | "owner")
                || candidate
                    .role
                    .strip_prefix("member(")
                    .and_then(|value| value.strip_suffix(')'))
                    .and_then(|value| value.parse::<i16>().ok())
                    .is_some_and(|visibility| (-16384..=16384).contains(&visibility));
            let storage_valid = matches!(
                candidate.storage_kind.as_str(),
                "plaintext" | "passphrase" | "macos-keychain" | "noise-file" | "generic-keychain"
            );
            if !candidate_id_valid
                || !ids.insert(candidate.candidate_id.clone())
                || !valid_typed_entity_id_hex(&candidate.host_id_hex, HOST_ID_PREFIX)
                || !valid_typed_entity_id_hex(&candidate.user_id_hex, USER_ID_PREFIX)
                || !device_id_valid
                || !role_valid
                || !storage_valid
                || candidate
                    .username
                    .as_ref()
                    .is_some_and(|value| !valid_response_text(value, 256))
                || candidate
                    .server_hint
                    .as_ref()
                    .is_some_and(|value| !valid_response_text(value, 2048))
                || ((candidate.hidden || candidate.provisional)
                    && (candidate.pairable || candidate.copyable))
                || (candidate.copyable
                    && (!candidate.pairable
                        || !candidate.device_id_hex.starts_with(DEVICE_ID_PREFIX)
                        || candidate.storage_kind != "macos-keychain"))
            {
                return Err(invalid_response(
                    "The agent returned an invalid legacy profile candidate.",
                ));
            }
            Ok(GoProfileCandidateDto {
                candidate_id: candidate.candidate_id,
                username: candidate.username,
                server_hint: candidate.server_hint,
                host_id: candidate.host_id_hex,
                user_id: candidate.user_id_hex,
                device_id: candidate.device_id_hex,
                role: candidate.role,
                storage_kind: candidate.storage_kind,
                hidden: candidate.hidden,
                provisional: candidate.provisional,
                pairable: candidate.pairable,
                copyable: candidate.copyable,
            })
        })
        .collect::<Result<Vec<_>, AgentError>>()?;
    Ok(GoProfileDiscoveryDto {
        installed: response.installed,
        candidates,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PendingOperationResponse {
    pub(super) kind: PendingOperationKind,
    pub(super) alias: String,
    pub(super) target: Option<String>,
}

impl From<PendingOperationResponse> for PendingOperationSummary {
    fn from(response: PendingOperationResponse) -> Self {
        Self {
            kind: response.kind,
            alias: response.alias,
            target: response.target,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitializationResponse {
    backend: CredentialBackend,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOfferDto {
    pub account_alias: String,
    #[serde(serialize_with = "serialize_secret")]
    pub phrase: Zeroizing<String>,
}

impl std::fmt::Debug for PairingOfferDto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PairingOfferDto")
            .field("account_alias", &self.account_alias)
            .field("phrase", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProvisionDto {
    pub alias: String,
    pub device_id: String,
    pub user_chain_sequence: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingOfferResponse {
    account_alias: String,
    #[serde(deserialize_with = "deserialize_secret")]
    phrase: Zeroizing<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceProvisionResponse {
    alias: String,
    device_id_hex: String,
    user_chain_sequence: u64,
}

pub(super) fn pending_operations(
    transport: &dyn foks_desktop::AgentTransport,
    profile: &str,
) -> Result<Vec<PendingOperationSummary>, AgentError> {
    require_transport_profile(transport, profile)?;
    let value = transport
        .call(Operation::ListPendingOperations {
            profile: profile.to_owned(),
        })
        .map_err(AgentError::from_desktop)?;
    require_response_row_cap(&value, "pending operations")?;
    let rows: Vec<PendingOperationResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub(super) fn validated_pending_dtos(
    pending: &[PendingOperationSummary],
) -> Result<Vec<PendingOperationDto>, AgentError> {
    if pending.len() > MAXIMUM_FIRST_RUN_ROWS {
        return Err(invalid_response(
            "The agent returned too many pending operations.",
        ));
    }
    let projected = pending
        .iter()
        .cloned()
        .map(PendingOperationDto::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let mut identities = std::collections::HashSet::new();
    if projected.iter().any(|operation| {
        !identities.insert((
            operation.kind,
            operation.alias.as_str(),
            operation.target.as_deref(),
        ))
    }) {
        return Err(invalid_response(
            "The agent returned duplicate pending operations.",
        ));
    }
    Ok(projected)
}

#[tauri::command]
pub async fn initialize_client_state(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AgentStatusDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let value = apply_operation_value(
        &state,
        Operation::InitializeState {
            backend: CredentialBackend::Native,
        },
        MutationKind::Create,
    )
    .await?;
    let response: InitializationResponse = serde_json::from_value(value).map_err(|error| {
        ambiguous_mutation_response(&state, format!("invalid initialization response: {error}"))
    })?;
    if response.backend != CredentialBackend::Native {
        return Err(ambiguous_mutation_response(
            &state,
            "The background service initialized an unexpected credential backend.",
        ));
    }
    let value = success_value(state.agent.call(Operation::AgentStatus).await?)?;
    let status: foks_agent_proto::AgentStatus = serde_json::from_value(value).map_err(|error| {
        ambiguous_mutation_response(&state, format!("invalid initialized status: {error}"))
    })?;
    if status != foks_agent_proto::AgentStatus::Ready {
        return Err(ambiguous_mutation_response(
            &state,
            "The background service did not become ready after initialization.",
        ));
    }
    Ok(status.into())
}

#[tauri::command]
pub async fn discover_go_profiles(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<GoProfileDiscoveryDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let value = success_value(state.agent.call(Operation::DiscoverGoProfiles).await?)?;
    go_profile_discovery_response(value)
}

#[tauri::command]
pub async fn check_and_add_go_profile(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    candidate_id: String,
    host_id: String,
    profile_name: String,
    probe: String,
) -> Result<CheckedProfileDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    if !valid_go_candidate_id(&candidate_id) || !valid_typed_entity_id_hex(&host_id, HOST_ID_PREFIX)
    {
        return Err(invalid_request("Select a valid legacy profile."));
    }
    let profile_name = bounded_local_name(
        &profile_name,
        "Server profile name must be 1 to 64 letters, numbers, hyphens, or underscores.",
    )?;
    let probe = bounded_field(
        &probe,
        2 * 1024,
        "Server address must be 2,048 bytes or fewer.",
    )?;
    if !valid_probe_target(&probe) {
        return Err(invalid_request(
            "Enter a valid DNS name, IP address, or host and port.",
        ));
    }
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        check_existing_or_add_profile(
            transport.as_ref(),
            profile_name,
            probe,
            Some((candidate_id, host_id)),
        )
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            &state,
            format!("Server verification was interrupted before completion: {error}"),
        )
    })?;
    match result {
        Ok(checked) => Ok(checked),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn list_pending_operations(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<Vec<PendingOperationDto>, AgentError> {
    require_main_window(&webview)?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        let pending = pending_operations(transport.as_ref(), &profile)?;
        validated_pending_dtos(&pending)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("failed to load pending operations: {error}")))?
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_first_run_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    username: String,
    device_name: String,
    email: String,
    invite: String,
    passphrase: Option<String>,
    passphrase_confirmation: Option<String>,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Enter a valid local account alias.")?;
    let username = bounded_field(&username, 256, "Enter a username.")?;
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let email = optional_bounded_field(&email, 320, "Enter a valid email address.")?;
    let invite = Zeroizing::new(invite);
    let invite = Zeroizing::new(optional_bounded_field(
        invite.as_str(),
        MAXIMUM_INVITE_BYTES,
        "Invitation code must be a single line of at most 4,096 bytes.",
    )?);
    let passphrase = optional_confirmed_passphrase(passphrase, passphrase_confirmation)?;
    let operation = Operation::CreateAccount {
        profile: profile.clone(),
        alias,
        username,
        device_name,
        email,
        invite: SecretString::new(invite.as_str()),
        passphrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Create).await?;
    account_sync_response(value).map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_first_run_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid pending account alias.")?;
    let operation = Operation::ResumeAccount {
        profile: profile.clone(),
        alias: alias.clone(),
    };
    let value = apply_pending_operation_value(
        &state,
        profile,
        PendingOperationKind::AccountSignup,
        alias,
        None,
        operation,
    )
    .await?;
    account_sync_response(value).map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn set_first_run_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    passphrase: String,
    confirmation: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid account alias.")?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    let operation = Operation::SetPassphrase {
        profile: profile.clone(),
        alias,
        passphrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Guarded).await?;
    passphrase_response(value).map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn recover_owner_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    target_alias: String,
    phrase: String,
    device_name: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let target_alias = bounded_local_name(&target_alias, "Enter a valid local account alias.")?;
    let expected_alias = target_alias.clone();
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let phrase = bounded_secret(
        phrase,
        MAXIMUM_RECOVERY_PHRASE_BYTES,
        "Recovery phrase must be a single line of at most 4,096 bytes.",
    )?;
    let serial = positive_recovery_serial()?;
    let operation = Operation::RecoverOwnerAccount {
        profile: profile.clone(),
        target_alias,
        phrase,
        device_name,
        serial,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Create).await?;
    recovery_response(value, &expected_alias)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_owner_recovery(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    target_alias: String,
    phrase: String,
    device_name: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let target_alias = bounded_local_name(&target_alias, "Choose a valid pending recovery alias.")?;
    let expected_alias = target_alias.clone();
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let phrase = bounded_secret(
        phrase,
        MAXIMUM_RECOVERY_PHRASE_BYTES,
        "Recovery phrase must be a single line of at most 4,096 bytes.",
    )?;
    let operation = Operation::ResumeOwnerRecovery {
        profile: profile.clone(),
        target_alias: target_alias.clone(),
        phrase,
        device_name,
    };
    let value = apply_pending_operation_value(
        &state,
        profile,
        PendingOperationKind::AccountRecovery,
        target_alias,
        None,
        operation,
    )
    .await?;
    recovery_response(value, &expected_alias)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

async fn pairing_offer_operation(
    state: &AppState,
    account: foks_agent_proto::AccountStoreRef,
    resume: bool,
) -> Result<PairingOfferDto, AgentError> {
    let expected = account.account_alias.clone();
    let operation = if resume {
        Operation::RepublishDevicePairing {
            profile: account.profile.clone(),
            account_alias: account.account_alias.clone(),
        }
    } else {
        Operation::StartDevicePairing {
            profile: account.profile.clone(),
            account_alias: account.account_alias.clone(),
        }
    };
    let value = if resume {
        apply_pending_operation_value(
            state,
            account.profile,
            PendingOperationKind::PairingOffer,
            account.account_alias,
            None,
            operation,
        )
        .await?
    } else {
        apply_profile_operation_value(state, account.profile, operation, MutationKind::Create)
            .await?
    };
    pairing_offer_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(state, error.message))
}

#[tauri::command]
pub async fn start_device_pairing(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<PairingOfferDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    pairing_offer_operation(&state, account, false).await
}

#[tauri::command]
pub async fn resume_device_pairing_offer(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<PairingOfferDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    pairing_offer_operation(&state, account, true).await
}

#[tauri::command]
pub async fn finish_device_pairing(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let expected = account.account_alias.clone();
    let operation = Operation::FinishDevicePairing {
        profile: account.profile.clone(),
        account_alias: account.account_alias.clone(),
    };
    let value = apply_pending_operation_value(
        &state,
        account.profile,
        PendingOperationKind::PairingOffer,
        account.account_alias,
        None,
        operation,
    )
    .await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn accept_device_pairing(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    target_alias: String,
    device_name: String,
    phrase: String,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let target_alias = bounded_local_name(&target_alias, "Enter a valid local account alias.")?;
    let expected = target_alias.clone();
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let phrase = pairing_phrase(phrase)?;
    let serial = positive_recovery_serial()?;
    let operation = Operation::AcceptDevicePairing {
        profile: profile.clone(),
        target_alias,
        device_name,
        serial,
        phrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Create).await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn accept_go_profile_pairing(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    request: GoProfilePairingRequest,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    let GoProfilePairingRequest {
        candidate_id,
        profile,
        target_alias,
        device_name,
        phrase,
    } = request;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    if !valid_go_candidate_id(&candidate_id) {
        return Err(invalid_request("Select a valid profile to import."));
    }
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let target_alias = bounded_local_name(&target_alias, "Enter a valid local account alias.")?;
    let expected = target_alias.clone();
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let phrase = pairing_phrase(phrase)?;
    let serial = positive_recovery_serial()?;
    let operation = Operation::AcceptGoProfilePairing {
        candidate_id,
        profile: profile.clone(),
        target_alias,
        device_name,
        serial,
        phrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Create).await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_device_pairing_acceptance(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    target_alias: String,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let target_alias = bounded_local_name(
        &target_alias,
        "Choose a valid pending device-pairing alias.",
    )?;
    let expected = target_alias.clone();
    let operation = Operation::ResumeDevicePairingAcceptance {
        profile: profile.clone(),
        target_alias: target_alias.clone(),
    };
    let value = apply_pending_operation_value(
        &state,
        profile,
        PendingOperationKind::PairingAcceptance,
        target_alias,
        None,
        operation,
    )
    .await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_go_profile_pairing(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    candidate_id: String,
    profile: String,
    target_alias: String,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    if !valid_go_candidate_id(&candidate_id) {
        return Err(invalid_request("Select a valid profile to import."));
    }
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let target_alias = bounded_local_name(
        &target_alias,
        "Choose a valid pending device-pairing alias.",
    )?;
    let expected = target_alias.clone();
    let operation = Operation::ResumeGoProfilePairing {
        candidate_id,
        profile: profile.clone(),
        target_alias: target_alias.clone(),
    };
    let value = apply_pending_operation_value(
        &state,
        profile,
        PendingOperationKind::PairingAcceptance,
        target_alias,
        None,
        operation,
    )
    .await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn copy_go_profile_device(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    candidate_id: String,
    profile: String,
    target_alias: String,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    if !valid_go_candidate_id(&candidate_id) {
        return Err(invalid_request("Select a valid profile to import."));
    }
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let target_alias = bounded_local_name(&target_alias, "Enter a valid local account alias.")?;
    let expected = target_alias.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::CopyGoProfileDevice {
            candidate_id,
            profile,
            target_alias,
        },
        MutationKind::Create,
    )
    .await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}
