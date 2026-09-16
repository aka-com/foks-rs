//! Hardware-backed enrollment, synchronization, and credential lifecycle.

use crate::agent::AgentError;
use crate::commands::accounts::{
    load_account_devices, passphrase_report_response, valid_sync_report, AccountSyncResponse,
    DeviceDto, PassphraseReportDto,
};
use crate::commands::context::AppState;
use crate::commands::execution::{
    ambiguous_mutation_response, apply_pending_operation_value, apply_profile_operation_value,
    read_profile_operation_value, MutationKind,
};
use crate::commands::validation::{
    bounded_field, bounded_local_name, bounded_secret, confirmed_passphrase, invalid_request,
    invalid_response, optional_bounded_field, optional_confirmed_passphrase,
    positive_recovery_serial, require_main_window, require_nested_response_row_cap,
    require_response_row_cap, valid_local_name, valid_response_text, valid_typed_entity_id_hex,
    valid_typed_yubi_id_hex, yubi_retry_configuration, yubi_slots, DEVICE_ID_PREFIX,
    MAXIMUM_FIRST_RUN_ROWS, MAXIMUM_INVITE_BYTES, MAXIMUM_PASSPHRASE_BYTES, SUBKEY_ID_PREFIX,
};
use foks_agent_proto::{Operation, PendingOperationKind, SecretString};
use serde::{Deserialize, Serialize};
use tauri::State;
use zeroize::Zeroizing;

pub(super) fn yubi_card_dtos(value: serde_json::Value) -> Result<Vec<YubiCardDto>, AgentError> {
    require_response_row_cap(&value, "connected security keys")?;
    let rows: Vec<YubiCardResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    let mut serials = std::collections::HashSet::new();
    rows.into_iter()
        .map(|row| {
            if row.serial == 0
                || !valid_response_text(&row.name, 256)
                || !serials.insert(row.serial)
            {
                return Err(invalid_response(
                    "The agent response contains an invalid or duplicate security key.",
                ));
            }
            Ok(YubiCardDto { serial: row.serial })
        })
        .collect()
}

pub(super) fn yubi_enrollment_dtos(
    value: serde_json::Value,
) -> Result<Vec<YubiEnrollmentDto>, AgentError> {
    require_response_row_cap(&value, "security-key enrollments")?;
    let rows: Vec<YubiEnrollmentResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    let mut enrollments = std::collections::HashSet::new();
    rows.into_iter()
        .map(|row| {
            let state = match row.state.as_str() {
                "pending" => "pending",
                "complete" => "complete",
                _ => {
                    return Err(invalid_response(
                        "The agent returned an unrecognized security-key enrollment state.",
                    ))
                }
            };
            if !valid_local_name(&row.alias) || !enrollments.insert((row.alias.clone(), state)) {
                return Err(invalid_response(
                    "The agent response contains an invalid or duplicate security key enrollment.",
                ));
            }
            if row
                .device_id_hex
                .as_ref()
                .is_some_and(|id| !valid_typed_yubi_id_hex(id))
                || row.card_serial == Some(0)
            {
                return Err(invalid_response(
                    "The enrollment has an invalid card identity.",
                ));
            }
            Ok(YubiEnrollmentDto {
                alias: row.alias,
                state,
                device_id: row.device_id_hex,
                card_serial: row.card_serial,
            })
        })
        .collect()
}

pub(super) fn require_yubi_enrollment(
    enrollments: &[YubiEnrollmentDto],
    alias: &str,
    expected_state: &str,
) -> Result<(), AgentError> {
    let has_pending = enrollments
        .iter()
        .any(|entry| entry.alias == alias && entry.state == "pending");
    let has_complete = enrollments
        .iter()
        .any(|entry| entry.alias == alias && entry.state == "complete");
    if !has_pending && !has_complete {
        return Err(AgentError::new(
            "security-key-not-found",
            "Refresh security keys before using this enrollment.",
            false,
        ));
    }
    match expected_state {
        "pending" if has_pending => Ok(()),
        "pending" => Err(AgentError::new(
            "security-key-state-changed",
            "This security key setup is already complete. Refresh the security keys list.",
            false,
        )),
        "complete" if has_complete && !has_pending => Ok(()),
        "complete" => Err(AgentError::new(
            "security-key-state-changed",
            "Resume the pending security key setup, then refresh security keys before continuing.",
            false,
        )),
        _ => Err(invalid_response(
            "Unsupported security key enrollment state requested.",
        )),
    }
}

pub(super) fn require_software_revocation_signer(devices: &[DeviceDto]) -> Result<(), AgentError> {
    match devices.iter().find(|device| device.current) {
        Some(device) if device.id.starts_with(DEVICE_ID_PREFIX) => Ok(()),
        Some(_) => Err(AgentError::new(
            "security-key-current",
            "A security key cannot sign its own revocation. Switch to a software credential on this device.",
            false,
        )),
        None => Err(invalid_response(
            "The current signing device could not be identified.",
        )),
    }
}

pub(super) fn yubi_account_response(
    value: serde_json::Value,
    expected_alias: &str,
    expected_username: Option<&str>,
) -> Result<YubiAccountDto, AgentError> {
    let response: YubiAccountResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias
        || !valid_response_text(&response.username, 256)
        || expected_username.is_some_and(|username| response.username != username)
        || !valid_typed_yubi_id_hex(&response.yubi_id_hex)
        || !valid_typed_entity_id_hex(&response.subkey_id_hex, SUBKEY_ID_PREFIX)
        || !response.management_enrolled
    {
        return Err(invalid_response(
            "The agent returned invalid security key account details.",
        ));
    }
    Ok(YubiAccountDto {
        alias: response.alias,
        username: response.username,
        yubi_id: response.yubi_id_hex,
        subkey_id: response.subkey_id_hex,
        user_chain_sequence: response.user_chain_sequence,
        management_enrolled: response.management_enrolled,
    })
}

pub(super) fn yubi_sync_response(
    value: serde_json::Value,
    expected_profile: &str,
    with_federation: bool,
) -> Result<YubiSyncDto, AgentError> {
    let (sync, federation) = if with_federation {
        require_nested_response_row_cap(&value, "federation", "federation refreshes")?;
        let response: YubiFederationSyncResponse =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        let mut teams = std::collections::HashSet::new();
        let federation = response
            .federation
            .into_iter()
            .map(|entry| {
                let deferred_valid = entry
                    .deferred
                    .as_deref()
                    .is_none_or(|reason| valid_response_text(reason, 512));
                if entry.local_profile != expected_profile
                    || !valid_local_name(&entry.local_team_alias)
                    || !teams.insert(entry.local_team_alias.clone())
                    || !deferred_valid
                    || entry.refreshed == entry.deferred.is_some()
                {
                    return Err(invalid_response(
                        "The agent returned an invalid federation refresh response.",
                    ));
                }
                Ok(YubiFederationRefreshDto {
                    local_profile: entry.local_profile,
                    local_team_alias: entry.local_team_alias,
                    refreshed: entry.refreshed,
                    deferred: entry.deferred,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        (response.sync, federation)
    } else {
        let response: AccountSyncResponse =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        (response, Vec::new())
    };
    if !valid_sync_report(&sync) {
        return Err(invalid_response(
            "The agent returned an invalid security-key synchronization response.",
        ));
    }
    Ok(YubiSyncDto {
        username: sync.username,
        user_chain_sequence: sync.user_chain_sequence,
        directories: sync.directories,
        entries: sync.entries,
        federation,
    })
}

pub(super) fn yubi_pin_status_response(
    value: serde_json::Value,
) -> Result<YubiPinStatusDto, AgentError> {
    let response: YubiPinStatusResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.blocked != (response.remaining == 0) {
        return Err(invalid_response(
            "The agent returned inconsistent security-key PIN retry data.",
        ));
    }
    Ok(YubiPinStatusDto {
        remaining: response.remaining,
        blocked: response.blocked,
    })
}

pub(super) fn yubi_lifecycle_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<YubiLifecycleDto, AgentError> {
    let response: YubiLifecycleResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias
        || !response.management_enrolled
        || response
            .management_generation
            .is_none_or(|generation| generation == 0)
    {
        return Err(invalid_response(
            "The agent returned an invalid security-key management response.",
        ));
    }
    Ok(YubiLifecycleDto {
        alias: response.alias,
        management_enrolled: response.management_enrolled,
        management_generation: response.management_generation,
    })
}

pub(super) fn yubi_subkey_recovery_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<YubiSubkeyRecoveryDto, AgentError> {
    let response: YubiSubkeyRecoveryResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias
        || !valid_typed_entity_id_hex(&response.subkey_id_hex, SUBKEY_ID_PREFIX)
        || response.certificate_count > MAXIMUM_FIRST_RUN_ROWS
    {
        return Err(invalid_response(
            "The agent returned an invalid security-key subkey recovery response.",
        ));
    }
    Ok(YubiSubkeyRecoveryDto {
        alias: response.alias,
        subkey_id: response.subkey_id_hex,
        certificate_count: response.certificate_count,
    })
}

pub(super) fn yubi_revocation_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<YubiRevocationDto, AgentError> {
    let response: YubiRevocationResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias || !response.removed_local_credential {
        return Err(invalid_response(
            "Removal of the selected security-key credential could not be confirmed.",
        ));
    }
    Ok(YubiRevocationDto {
        alias: response.alias,
        user_chain_sequence: response.user_chain_sequence,
        removed_local_credential: response.removed_local_credential,
    })
}

pub(super) fn yubi_changed_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<YubiChangedDto, AgentError> {
    let response: YubiChangedResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias || !response.changed {
        return Err(invalid_response(
            "Security-key update could not be confirmed.",
        ));
    }
    Ok(YubiChangedDto {
        alias: response.alias,
        changed: response.changed,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiCardDto {
    pub serial: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiEnrollmentDto {
    pub alias: String,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_serial: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiAccountDto {
    pub alias: String,
    pub username: String,
    pub yubi_id: String,
    pub subkey_id: String,
    pub user_chain_sequence: u64,
    pub management_enrolled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiSyncDto {
    pub username: String,
    pub user_chain_sequence: u64,
    pub directories: usize,
    pub entries: usize,
    pub federation: Vec<YubiFederationRefreshDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiFederationRefreshDto {
    pub local_profile: String,
    pub local_team_alias: String,
    pub refreshed: bool,
    pub deferred: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiPinStatusDto {
    pub remaining: u8,
    pub blocked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiLifecycleDto {
    pub alias: String,
    pub management_enrolled: bool,
    pub management_generation: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiSubkeyRecoveryDto {
    pub alias: String,
    pub subkey_id: String,
    pub certificate_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiRevocationDto {
    pub alias: String,
    pub user_chain_sequence: u64,
    pub removed_local_credential: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiChangedDto {
    pub alias: String,
    pub changed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiCardResponse {
    name: String,
    serial: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiEnrollmentResponse {
    alias: String,
    state: String,
    device_id_hex: Option<String>,
    card_serial: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiAccountResponse {
    alias: String,
    username: String,
    yubi_id_hex: String,
    subkey_id_hex: String,
    user_chain_sequence: u64,
    management_enrolled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiFederationSyncResponse {
    sync: AccountSyncResponse,
    federation: Vec<YubiFederationRefreshResponse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiFederationRefreshResponse {
    local_profile: String,
    local_team_alias: String,
    refreshed: bool,
    deferred: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiPinStatusResponse {
    remaining: u8,
    blocked: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiLifecycleResponse {
    alias: String,
    management_enrolled: bool,
    management_generation: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiSubkeyRecoveryResponse {
    alias: String,
    subkey_id_hex: String,
    certificate_count: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiRevocationResponse {
    alias: String,
    user_chain_sequence: u64,
    removed_local_credential: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiChangedResponse {
    alias: String,
    changed: bool,
}

async fn yubi_enrollments_for_profile(
    state: &AppState,
    profile: String,
) -> Result<Vec<YubiEnrollmentDto>, AgentError> {
    let value = read_profile_operation_value(
        state,
        profile.clone(),
        Operation::ListYubiAccounts { profile },
    )
    .await?;
    yubi_enrollment_dtos(value)
}

async fn require_complete_yubi(
    state: &AppState,
    profile: String,
    alias: &str,
) -> Result<(), AgentError> {
    let enrollments = yubi_enrollments_for_profile(state, profile).await?;
    require_yubi_enrollment(&enrollments, alias, "complete")
}

#[tauri::command]
pub async fn list_yubi_cards(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<Vec<YubiCardDto>, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let value = read_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ListYubiCards { profile },
    )
    .await?;
    yubi_card_dtos(value)
}

#[tauri::command]
pub async fn list_yubi_accounts(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<Vec<YubiEnrollmentDto>, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    yubi_enrollments_for_profile(&state, profile).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_yubi_account(
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
    card_serial: u32,
    signing_slot: u8,
    pq_slot: u8,
    pin: String,
    puk: String,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<YubiAccountDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Enter a valid local security-key alias.")?;
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
    let (signing_slot, pq_slot) = yubi_slots(signing_slot, pq_slot)?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    let retry_configuration = yubi_retry_configuration(puk, pin_attempts, puk_attempts)?;
    let enrollments = yubi_enrollments_for_profile(&state, profile.clone()).await?;
    if enrollments.iter().any(|entry| entry.alias == alias) {
        return Err(AgentError::new(
            "already-exists",
            "A security-key enrollment already uses this local alias.",
            false,
        ));
    }
    let cards_value = read_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ListYubiCards {
            profile: profile.clone(),
        },
    )
    .await?;
    if !yubi_card_dtos(cards_value)?
        .iter()
        .any(|card| card.serial == card_serial)
    {
        return Err(AgentError::new(
            "security-key-not-connected",
            "The selected security key is not connected. Refresh connected keys and try again.",
            false,
        ));
    }
    let expected = alias.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::CreateYubiAccount {
            profile,
            alias,
            username,
            device_name,
            email,
            invite: SecretString::new(invite.as_str()),
            passphrase,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration: Some(retry_configuration),
        },
        MutationKind::Create,
    )
    .await?;
    yubi_account_response(value, &expected, None)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_yubi_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
) -> Result<YubiAccountDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid pending security-key alias.")?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    let enrollments = yubi_enrollments_for_profile(&state, profile.clone()).await?;
    require_yubi_enrollment(&enrollments, &alias, "pending")?;
    let expected = alias.clone();
    let value = apply_pending_operation_value(
        &state,
        profile.clone(),
        PendingOperationKind::YubiEnrollment,
        alias.clone(),
        None,
        Operation::ResumeYubiAccount {
            profile,
            alias,
            pin,
        },
    )
    .await?;
    yubi_account_response(value, &expected, None)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provision_yubi_device(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    target_alias: String,
    device_name: String,
    card_serial: u32,
    signing_slot: u8,
    pq_slot: u8,
    pin: String,
    puk: String,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<YubiAccountDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account_dto = state.selected_account_dto(&account_store_id)?;
    let account = foks_agent_proto::AccountStoreRef {
        profile: account_dto.profile.clone(),
        account_alias: account_dto.alias.clone(),
    };
    let target_alias =
        bounded_local_name(&target_alias, "Enter a valid local security-key alias.")?;
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let (signing_slot, pq_slot) = yubi_slots(signing_slot, pq_slot)?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    let retry_configuration = yubi_retry_configuration(puk, pin_attempts, puk_attempts)?;
    let enrollments = yubi_enrollments_for_profile(&state, account.profile.clone()).await?;
    if enrollments.iter().any(|entry| entry.alias == target_alias) {
        return Err(AgentError::new(
            "already-exists",
            "A security-key enrollment already uses this local alias.",
            false,
        ));
    }
    let cards_value = read_profile_operation_value(
        &state,
        account.profile.clone(),
        Operation::ListYubiCards {
            profile: account.profile.clone(),
        },
    )
    .await?;
    if !yubi_card_dtos(cards_value)?
        .iter()
        .any(|card| card.serial == card_serial)
    {
        return Err(AgentError::new(
            "security-key-not-connected",
            "The selected security key is not connected. Refresh connected keys and try again.",
            false,
        ));
    }
    let expected = target_alias.clone();
    let profile = account.profile;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ProvisionYubiDevice {
            profile,
            source_alias: account.account_alias,
            target_alias,
            device_name,
            serial: positive_recovery_serial()?,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration: Some(retry_configuration),
        },
        MutationKind::Create,
    )
    .await?;
    yubi_account_response(value, &expected, Some(&account_dto.username))
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn sync_yubi_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
    with_federation: bool,
) -> Result<YubiSyncDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let expected_profile = profile.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::SyncYubiAccount {
            profile,
            alias,
            pin,
            with_federation,
        },
        MutationKind::Resume,
    )
    .await?;
    yubi_sync_response(value, &expected_profile, with_federation)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn yubi_pin_status(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
) -> Result<YubiPinStatusDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let value = read_profile_operation_value(
        &state,
        profile.clone(),
        Operation::YubiPinStatus { profile, alias },
    )
    .await?;
    yubi_pin_status_response(value)
}

#[tauri::command]
pub async fn change_yubi_pin(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    old_pin: String,
    new_pin: String,
) -> Result<YubiPinStatusDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let old_pin = bounded_secret(old_pin, 128, "Current PIN is required.")?;
    let new_pin = bounded_secret(
        new_pin,
        128,
        "New PIN is required and must be at most 128 bytes.",
    )?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ChangeYubiPin {
            profile,
            alias,
            old_pin,
            new_pin,
        },
        MutationKind::Guarded,
    )
    .await?;
    yubi_pin_status_response(value)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

async fn yubi_passphrase_operation(
    state: &AppState,
    profile: String,
    alias: String,
    pin: SecretString,
    passphrase: SecretString,
    mode: &'static str,
) -> Result<PassphraseReportDto, AgentError> {
    require_complete_yubi(state, profile.clone(), &alias).await?;
    let operation = match mode {
        "set" => Operation::SetYubiPassphrase {
            profile: profile.clone(),
            alias,
            pin,
            passphrase,
        },
        "change" => Operation::ChangeYubiPassphrase {
            profile: profile.clone(),
            alias,
            pin,
            passphrase,
        },
        "verify" => Operation::VerifyYubiPassphrase {
            profile: profile.clone(),
            alias,
            pin,
            passphrase,
        },
        _ => {
            return Err(invalid_request(
                "Invalid security-key passphrase operation.",
            ))
        }
    };
    let value =
        apply_profile_operation_value(state, profile, operation, MutationKind::Guarded).await?;
    passphrase_report_response(value)
        .map_err(|error| ambiguous_mutation_response(state, error.message))
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn set_yubi_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
    passphrase: String,
    confirmation: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    yubi_passphrase_operation(&state, profile, alias, pin, passphrase, "set").await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn change_yubi_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
    passphrase: String,
    confirmation: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    yubi_passphrase_operation(&state, profile, alias, pin, passphrase, "change").await
}

#[tauri::command]
pub async fn verify_yubi_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
    passphrase: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    let passphrase = bounded_secret(
        passphrase,
        MAXIMUM_PASSPHRASE_BYTES,
        "Passphrase is required and must be at most 1,024 bytes.",
    )?;
    yubi_passphrase_operation(&state, profile, alias, pin, passphrase, "verify").await
}

#[tauri::command]
pub async fn change_yubi_puk(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    old_puk: String,
    new_puk: String,
) -> Result<YubiChangedDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let old_puk = bounded_secret(old_puk, 128, "Current unlock code (PUK) is required.")?;
    let new_puk = bounded_secret(
        new_puk,
        128,
        "New unlock code (PUK) is required and must be at most 128 bytes.",
    )?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let expected = alias.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ChangeYubiPuk {
            profile,
            alias,
            old_puk,
            new_puk,
        },
        MutationKind::Guarded,
    )
    .await?;
    yubi_changed_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn unblock_yubi_pin(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    puk: String,
    new_pin: String,
) -> Result<YubiPinStatusDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let puk = bounded_secret(puk, 128, "Current unlock code (PUK) is required.")?;
    let new_pin = bounded_secret(
        new_pin,
        128,
        "New PIN is required and must be at most 128 bytes.",
    )?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::UnblockYubiPin {
            profile,
            alias,
            puk,
            new_pin,
        },
        MutationKind::Guarded,
    )
    .await?;
    yubi_pin_status_response(value)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

async fn yubi_lifecycle_operation(
    state: &AppState,
    profile: String,
    alias: String,
    operation: Operation,
    kind: MutationKind,
) -> Result<YubiLifecycleDto, AgentError> {
    require_complete_yubi(state, profile.clone(), &alias).await?;
    let value = apply_profile_operation_value(state, profile, operation, kind).await?;
    yubi_lifecycle_response(value, &alias)
        .map_err(|error| ambiguous_mutation_response(state, error.message))
}

#[tauri::command]
pub async fn rotate_yubi_management_key(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
) -> Result<YubiLifecycleDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    yubi_lifecycle_operation(
        &state,
        profile.clone(),
        alias.clone(),
        Operation::RotateYubiManagementKey {
            profile,
            alias,
            pin,
        },
        MutationKind::Guarded,
    )
    .await
}

#[tauri::command]
pub async fn resume_yubi_management_key(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: Option<String>,
) -> Result<YubiLifecycleDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = pin
        .map(|pin| bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes."))
        .transpose()?;
    yubi_lifecycle_operation(
        &state,
        profile.clone(),
        alias.clone(),
        Operation::ResumeYubiManagementKey {
            profile,
            alias,
            pin,
        },
        MutationKind::Resume,
    )
    .await
}

#[tauri::command]
pub async fn recover_yubi_management_key(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    yubi_alias: String,
) -> Result<YubiLifecycleDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let yubi_alias = bounded_local_name(&yubi_alias, "Choose a valid security-key alias.")?;
    let profile = account.profile;
    let operation = Operation::RecoverYubiManagementKey {
        profile: profile.clone(),
        yubi_alias: yubi_alias.clone(),
        software_alias: account.account_alias,
    };
    yubi_lifecycle_operation(&state, profile, yubi_alias, operation, MutationKind::Resume).await
}

#[tauri::command]
pub async fn recover_yubi_subkey(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
) -> Result<YubiSubkeyRecoveryDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "PIN is required and must be at most 128 bytes.")?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let expected = alias.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::RecoverYubiSubkey {
            profile,
            alias,
            pin,
        },
        MutationKind::Resume,
    )
    .await?;
    yubi_subkey_recovery_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn revoke_yubi_device(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    yubi_alias: String,
    confirmation: String,
) -> Result<YubiRevocationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let yubi_alias = bounded_local_name(&yubi_alias, "Choose a valid security-key alias.")?;
    if confirmation != yubi_alias {
        return Err(invalid_request(
            "Type the exact security-key alias to revoke it.",
        ));
    }
    require_complete_yubi(&state, account.profile.clone(), &yubi_alias).await?;
    let transport = state.agent.transport();
    let signer_account = account.clone();
    let devices = tauri::async_runtime::spawn_blocking(move || {
        load_account_devices(transport.as_ref(), &signer_account)
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!(
            "failed to run security-key revocation preflight: {error}"
        ))
    })??;
    require_software_revocation_signer(&devices)?;
    let expected = yubi_alias.clone();
    let profile = account.profile;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::RevokeYubiDevice {
            profile,
            yubi_alias,
            software_alias: account.account_alias,
        },
        MutationKind::Guarded,
    )
    .await?;
    yubi_revocation_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}
