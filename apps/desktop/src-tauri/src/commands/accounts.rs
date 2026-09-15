//! Account inventory, devices, passphrases, and owner backups.

use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::execution::{
    ambiguous_mutation_response, apply_profile_operation_value, read_profile_operation_value,
    MutationKind,
};
use crate::commands::servers::require_transport_profile;
use crate::commands::types::MutationDto;
use crate::commands::validation::{
    bounded_local_name, bounded_secret, confirmed_passphrase, deserialize_secret, invalid_request,
    invalid_response, require_main_window, require_response_row_cap, serialize_secret,
    valid_device_member_id_hex, valid_local_name, valid_response_text, valid_typed_entity_id_hex,
    BACKUP_ID_PREFIX, MAXIMUM_FIRST_RUN_ROWS, MAXIMUM_PASSPHRASE_BYTES,
    MAXIMUM_RECOVERY_PHRASE_BYTES, MAXIMUM_SYNC_FACTS,
};
use crate::commands::vault::store_id;
use foks_agent_proto::{Operation, SecretString};
use foks_desktop::{CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tauri::State;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountResponse {
    profile: String,
    alias: String,
    username: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPhraseDto {
    pub backup_alias: String,
    #[serde(serialize_with = "serialize_secret")]
    pub phrase: Zeroizing<String>,
}

impl std::fmt::Debug for BackupPhraseDto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BackupPhraseDto")
            .field("backup_alias", &self.backup_alias)
            .field("phrase", &"[REDACTED]")
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupPhraseResponse {
    backup_alias: String,
    #[serde(deserialize_with = "deserialize_secret")]
    phrase: Zeroizing<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccountSyncResponse {
    pub(super) username: String,
    pub(super) user_chain_sequence: u64,
    pub(super) directories: usize,
    pub(super) entries: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassphraseResponse {
    generation: u64,
    stretch_version: String,
    verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupCommitResponse {
    backup_alias: String,
    account_alias: String,
    backup_id_hex: String,
    user_chain_sequence: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupRevocationResponse {
    backup_alias: String,
    account_alias: String,
    backup_id_hex: String,
    user_chain_sequence: u64,
    already_absent: bool,
    removed_local_enrollment: bool,
}

pub(super) fn backup_phrase_response(
    value: serde_json::Value,
    expected_backup: &str,
) -> Result<BackupPhraseDto, AgentError> {
    let response: BackupPhraseResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.backup_alias != expected_backup
        || response.phrase.is_empty()
        || response.phrase.len() > MAXIMUM_RECOVERY_PHRASE_BYTES
        || response.phrase.contains(['\0', '\r', '\n'])
        || foks_crypto::BackupKey::from_phrase(response.phrase.as_str()).is_err()
    {
        return Err(invalid_response(
            "The agent returned an invalid recovery phrase.",
        ));
    }
    Ok(BackupPhraseDto {
        backup_alias: response.backup_alias,
        phrase: response.phrase,
    })
}

pub(super) fn account_sync_response(value: serde_json::Value) -> Result<MutationDto, AgentError> {
    let report: AccountSyncResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    // Account lookup names are normalized before signing, so the authenticated
    // display username does not need to match input bytes exactly.
    if !valid_response_text(&report.username, 256)
        || report.directories > MAXIMUM_SYNC_FACTS
        || report.entries > MAXIMUM_SYNC_FACTS
        || (report.directories == 0 && report.entries != 0)
    {
        return Err(invalid_response(
            "The agent returned an invalid account synchronization response.",
        ));
    }
    let _sequence = report.user_chain_sequence;
    Ok(MutationDto { applied: true })
}

pub(super) fn passphrase_response(value: serde_json::Value) -> Result<MutationDto, AgentError> {
    passphrase_report_response(value).map(|_| MutationDto { applied: true })
}

pub(super) fn passphrase_report_response(
    value: serde_json::Value,
) -> Result<PassphraseReportDto, AgentError> {
    let report: PassphraseResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    // Reject test-only key derivation stretches in production reports.
    if report.generation == 0 || report.stretch_version != "v1" || !report.verified {
        return Err(invalid_response(
            "The agent returned an invalid passphrase verification response.",
        ));
    }
    Ok(PassphraseReportDto {
        generation: report.generation,
        stretch_version: report.stretch_version,
        verified: report.verified,
    })
}

pub(super) fn device_dtos(value: serde_json::Value) -> Result<Vec<DeviceDto>, AgentError> {
    require_response_row_cap(&value, "devices")?;
    let rows: Vec<DeviceResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    let mut ids = std::collections::HashSet::new();
    let mut current_count = 0usize;
    let devices = rows
        .into_iter()
        .map(|row| {
            if !valid_device_member_id_hex(&row.id_hex)
                || !ids.insert(row.id_hex.clone())
                || row
                    .name
                    .as_ref()
                    .is_some_and(|name| !valid_response_text(name, 256))
            {
                return Err(invalid_response(
                    "The agent response contains an invalid or duplicate device.",
                ));
            }
            let role = match row.role.as_str() {
                "owner" => "owner",
                "admin" => "admin",
                "member" => "member",
                _ => {
                    return Err(invalid_response(
                        "The agent returned an unknown device role.",
                    ))
                }
            };
            current_count += usize::from(row.current);
            Ok(DeviceDto {
                id: row.id_hex,
                name: row.name,
                role,
                current: row.current,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if current_count != 1 {
        return Err(invalid_response(
            "The agent returned an invalid current device state.",
        ));
    }
    Ok(devices)
}

pub(super) fn backup_enrollment_dtos(
    value: serde_json::Value,
    expected_account: &str,
) -> Result<Vec<BackupEnrollmentDto>, AgentError> {
    require_response_row_cap(&value, "backup enrollments")?;
    let rows: Vec<BackupEnrollmentResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    let mut aliases = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    rows.into_iter()
        .map(|row| {
            if row.account_alias != expected_account
                || !valid_local_name(&row.backup_alias)
                || !valid_local_name(&row.account_alias)
                || !valid_typed_entity_id_hex(&row.backup_id_hex, BACKUP_ID_PREFIX)
                || !aliases.insert(row.backup_alias.clone())
                || !ids.insert(row.backup_id_hex.clone())
            {
                return Err(invalid_response(
                    "The agent response contains an invalid or duplicate backup key.",
                ));
            }
            Ok(BackupEnrollmentDto {
                backup_alias: row.backup_alias,
                account_alias: row.account_alias,
                backup_id: row.backup_id_hex,
            })
        })
        .collect()
}

pub(super) fn valid_sync_report(report: &AccountSyncResponse) -> bool {
    valid_response_text(&report.username, 256)
        && report.directories <= MAXIMUM_SYNC_FACTS
        && report.entries <= MAXIMUM_SYNC_FACTS
        && (report.directories != 0 || report.entries == 0)
}

pub(super) fn device_removal_response(
    value: serde_json::Value,
    expected_device: &str,
) -> Result<DeviceRemovalDto, AgentError> {
    let response: DeviceRemovalResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.device_id_hex != expected_device {
        return Err(invalid_response(
            "The agent confirmed removal for a different device identifier.",
        ));
    }
    Ok(DeviceRemovalDto {
        device_id: response.device_id_hex,
        user_chain_sequence: response.user_chain_sequence,
        already_absent: response.already_absent,
    })
}

pub(super) fn backup_commit_response(
    value: serde_json::Value,
    expected_account: &str,
    expected_backup: &str,
) -> Result<MutationDto, AgentError> {
    let report: BackupCommitResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if report.account_alias != expected_account
        || report.backup_alias != expected_backup
        || !valid_typed_entity_id_hex(&report.backup_id_hex, BACKUP_ID_PREFIX)
    {
        return Err(invalid_response(
            "The agent returned an invalid backup confirmation response.",
        ));
    }
    let _sequence = report.user_chain_sequence;
    Ok(MutationDto { applied: true })
}

pub(super) fn backup_revocation_response(
    value: serde_json::Value,
    expected_account: &str,
    expected_backup: &str,
    expected_id: &str,
) -> Result<BackupRevocationDto, AgentError> {
    let report: BackupRevocationResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if report.account_alias != expected_account
        || report.backup_alias != expected_backup
        || report.backup_id_hex != expected_id
        || !valid_typed_entity_id_hex(&report.backup_id_hex, BACKUP_ID_PREFIX)
        || !report.removed_local_enrollment
    {
        return Err(invalid_response(
            "The agent did not confirm revocation of the selected backup key.",
        ));
    }
    Ok(BackupRevocationDto {
        backup_alias: report.backup_alias,
        account_alias: report.account_alias,
        backup_id: report.backup_id_hex,
        user_chain_sequence: report.user_chain_sequence,
        already_absent: report.already_absent,
        removed_local_enrollment: report.removed_local_enrollment,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDto {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub role: &'static str,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRemovalDto {
    pub device_id: String,
    pub user_chain_sequence: u64,
    pub already_absent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEnrollmentDto {
    pub backup_alias: String,
    pub account_alias: String,
    pub backup_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRevocationDto {
    pub backup_alias: String,
    pub account_alias: String,
    pub backup_id: String,
    pub user_chain_sequence: u64,
    pub already_absent: bool,
    pub removed_local_enrollment: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassphraseReportDto {
    pub generation: u64,
    pub stretch_version: String,
    pub verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceResponse {
    id_hex: String,
    name: Option<String>,
    role: String,
    current: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceRemovalResponse {
    device_id_hex: String,
    user_chain_sequence: u64,
    already_absent: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupEnrollmentResponse {
    backup_alias: String,
    account_alias: String,
    backup_id_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccountDto {
    pub store: String,
    pub profile: String,
    pub alias: String,
    pub username: String,
}

pub(super) fn load_accounts(
    transport: &dyn foks_desktop::AgentTransport,
    catalog: &CatalogSnapshot,
) -> Result<Vec<AccountDto>, AgentError> {
    let mut expected = HashMap::new();
    for summary in &catalog.stores {
        if let CatalogStoreSummary::Account { store } = summary {
            // Retain store summaries from failed catalogs so the UI can show
            // which server is blocked without attempting to read accounts from it.
            if catalog.profile_blocked(&store.profile) {
                continue;
            }
            if !valid_response_text(&store.profile, 64) || !valid_local_name(&store.account_alias) {
                return Err(invalid_response("The vault returned an invalid account."));
            }
            let identity = (store.profile.clone(), store.account_alias.clone());
            if expected
                .insert(identity, store_id(&CatalogStoreRef::Account(store.clone())))
                .is_some()
            {
                return Err(invalid_response("The vault returned a duplicate account."));
            }
        }
    }
    if expected.len() > MAXIMUM_FIRST_RUN_ROWS {
        return Err(invalid_response("The vault returned too many accounts."));
    }
    let mut profiles = expected
        .keys()
        .map(|(profile, _)| profile.clone())
        .collect::<Vec<_>>();
    profiles.sort();
    profiles.dedup();
    let mut accounts = Vec::with_capacity(expected.len());
    for profile in profiles {
        let rows: Vec<AccountResponse> = if let Some(overview) = catalog
            .profile_overviews
            .iter()
            .find(|overview| overview.profile == profile)
        {
            let value = match overview.accounts.clone() {
                foks_agent_proto::ResponseResult::Success { value } => value,
                foks_agent_proto::ResponseResult::Error {
                    code,
                    message,
                    fields,
                } => {
                    return Err(AgentError::from_desktop(
                        foks_desktop::AgentError::Protocol {
                            code,
                            message,
                            fields,
                        },
                    ));
                }
            };
            require_response_row_cap(&value, "account identities")?;
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?
        } else {
            let value = transport
                .call(Operation::ListAccounts {
                    profile: profile.clone(),
                })
                .map_err(AgentError::from_desktop)?;
            require_response_row_cap(&value, "account identities")?;
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?
        };
        for row in rows {
            if row.profile != profile
                || !valid_local_name(&row.alias)
                || !valid_response_text(&row.username, 256)
            {
                return Err(invalid_response(
                    "The agent returned invalid account details.",
                ));
            }
            let identity = (row.profile.clone(), row.alias.clone());
            let Some(store) = expected.remove(&identity) else {
                return Err(invalid_response(
                    "The agent returned an account not present in the vault.",
                ));
            };
            accounts.push(AccountDto {
                store,
                profile: row.profile,
                alias: row.alias,
                username: row.username,
            });
        }
    }
    if !expected.is_empty() {
        return Err(invalid_response(
            "An account is missing from the vault response.",
        ));
    }
    accounts
        .sort_by(|left, right| (&left.profile, &left.alias).cmp(&(&right.profile, &right.alias)));
    Ok(accounts)
}

pub(super) fn load_account_devices(
    transport: &dyn foks_desktop::AgentTransport,
    account: &foks_agent_proto::AccountStoreRef,
) -> Result<Vec<DeviceDto>, AgentError> {
    require_transport_profile(transport, &account.profile)?;
    let value = transport
        .call(Operation::ListDevices {
            profile: account.profile.clone(),
            alias: account.account_alias.clone(),
        })
        .map_err(AgentError::from_desktop)?;
    device_dtos(value)
}

pub(super) fn load_backup_enrollments(
    transport: &dyn foks_desktop::AgentTransport,
    account: &foks_agent_proto::AccountStoreRef,
) -> Result<Vec<BackupEnrollmentDto>, AgentError> {
    require_transport_profile(transport, &account.profile)?;
    let value = transport
        .call(Operation::ListBackupEnrollments {
            profile: account.profile.clone(),
            account_alias: account.account_alias.clone(),
        })
        .map_err(AgentError::from_desktop)?;
    backup_enrollment_dtos(value, &account.account_alias)
}

#[tauri::command]
pub async fn list_account_devices(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<Vec<DeviceDto>, AgentError> {
    require_main_window(&webview)?;
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let account = state.selected_account(&account_store_id)?;
    let transport = state.agent.transport();
    let devices = tauri::async_runtime::spawn_blocking(move || {
        load_account_devices(transport.as_ref(), &account)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Failed to load account devices: {error}")))??;
    state.retain_devices(generation, account_store_id, &devices)?;
    Ok(devices)
}

#[tauri::command]
pub async fn list_backup_enrollments(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<Vec<BackupEnrollmentDto>, AgentError> {
    require_main_window(&webview)?;
    let account = state.selected_account(&account_store_id)?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        load_backup_enrollments(transport.as_ref(), &account)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Failed to load backup enrollments: {error}")))?
}

#[tauri::command]
pub async fn revoke_owner_backup(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    backup_alias: String,
    backup_id: String,
    confirmation: String,
) -> Result<BackupRevocationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let backup_alias = bounded_local_name(&backup_alias, "Provide a valid backup alias.")?;
    if confirmation != backup_alias {
        return Err(invalid_request("Backup alias confirmation does not match."));
    }
    if !valid_typed_entity_id_hex(&backup_id, BACKUP_ID_PREFIX) {
        return Err(invalid_request("Select a valid backup."));
    }
    let expected_account = account.account_alias.clone();
    let expected_backup = backup_alias.clone();
    let expected_id = backup_id.clone();
    let profile = account.profile;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::RevokeOwnerBackup {
            profile,
            account_alias: account.account_alias,
            backup_alias,
            backup_id,
        },
        MutationKind::Guarded,
    )
    .await?;
    backup_revocation_response(value, &expected_account, &expected_backup, &expected_id)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn remove_account_device(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    device_id: String,
) -> Result<DeviceRemovalDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_device_target(&account_store_id, &device_id)?;
    let expected = device_id.clone();
    let value = apply_profile_operation_value(
        &state,
        account.profile.clone(),
        Operation::RemoveDevice {
            profile: account.profile,
            signer_alias: account.account_alias,
            device_id,
        },
        MutationKind::Guarded,
    )
    .await?;
    device_removal_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn prepare_owner_backup(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    backup_alias: String,
) -> Result<BackupPhraseDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let account_alias = bounded_local_name(&account_alias, "Choose a valid account alias.")?;
    let backup_alias = bounded_local_name(&backup_alias, "Enter a valid backup alias.")?;
    let expected = backup_alias.clone();
    let operation = Operation::PrepareOwnerBackup {
        profile: profile.clone(),
        account_alias,
        backup_alias,
    };
    let value = read_profile_operation_value(&state, profile, operation).await?;
    backup_phrase_response(value, &expected)
}

#[tauri::command]
pub async fn commit_owner_backup(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    backup_alias: String,
    phrase: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let account_alias = bounded_local_name(&account_alias, "Choose a valid account alias.")?;
    let backup_alias = bounded_local_name(&backup_alias, "Provide a valid backup alias.")?;
    let expected_account = account_alias.clone();
    let expected_backup = backup_alias.clone();
    let phrase = bounded_secret(
        phrase,
        MAXIMUM_RECOVERY_PHRASE_BYTES,
        "Backup phrase must be a single line of at most 4,096 bytes.",
    )?;
    let operation = Operation::CommitOwnerBackup {
        profile: profile.clone(),
        account_alias,
        backup_alias,
        phrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Guarded).await?;
    backup_commit_response(value, &expected_account, &expected_backup)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

async fn account_passphrase_mutation(
    state: &AppState,
    account: foks_agent_proto::AccountStoreRef,
    passphrase: SecretString,
    change: bool,
) -> Result<PassphraseReportDto, AgentError> {
    let operation = if change {
        Operation::ChangePassphrase {
            profile: account.profile.clone(),
            alias: account.account_alias,
            passphrase,
        }
    } else {
        Operation::SetPassphrase {
            profile: account.profile.clone(),
            alias: account.account_alias,
            passphrase,
        }
    };
    let value =
        apply_profile_operation_value(state, account.profile, operation, MutationKind::Guarded)
            .await?;
    passphrase_report_response(value)
        .map_err(|error| ambiguous_mutation_response(state, error.message))
}

#[tauri::command]
pub async fn set_account_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    passphrase: String,
    confirmation: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    account_passphrase_mutation(&state, account, passphrase, false).await
}

#[tauri::command]
pub async fn change_account_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    passphrase: String,
    confirmation: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    account_passphrase_mutation(&state, account, passphrase, true).await
}

#[tauri::command]
pub async fn verify_account_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    passphrase: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let account = state.selected_account(&account_store_id)?;
    let passphrase = bounded_secret(
        passphrase,
        MAXIMUM_PASSPHRASE_BYTES,
        "Passphrase is required and must be at most 1,024 bytes.",
    )?;
    let operation = Operation::VerifyPassphrase {
        profile: account.profile.clone(),
        alias: account.account_alias,
        passphrase,
    };
    let value = read_profile_operation_value(&state, account.profile, operation).await?;
    passphrase_report_response(value)
}

#[tauri::command]
pub async fn list_accounts(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    generation: Option<u64>,
) -> Result<Vec<AccountDto>, AgentError> {
    require_main_window(&webview)?;
    let (generation, catalog) = state.catalog_at(generation)?;
    let catalog = catalog.ok_or_else(|| {
        AgentError::new(
            "catalog-required",
            "Refresh the vault before loading account identities.",
            true,
        )
    })?;
    let transport = state.agent.transport();
    let accounts =
        tauri::async_runtime::spawn_blocking(move || load_accounts(transport.as_ref(), &catalog))
            .await
            .map_err(|error| AgentError::unknown(format!("failed to load accounts: {error}")))??;
    state.retain_accounts(generation, &accounts)?;
    Ok(accounts)
}
