//! Nonsecret operation records for a specific desktop account operation. A missing
//! operation record is never evidence that an agent or remote mutation did not apply.

use super::validation::{
    invalid_request, require_main_window, valid_go_candidate_id, valid_local_name,
    valid_response_text, valid_typed_entity_id_hex, HOST_ID_PREFIX,
};
use super::{context::AppState, enrollment, servers, types::MutationDto};
use crate::agent::AgentError;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::{Manager, State};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountAttempt {
    pub id: String,
    pub profile: String,
    pub host_id: String,
    pub alias: String,
    pub device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    pub kind: AccountKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AccountKind {
    Signup,
    Recovery,
    Copy,
    Pairing,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Complete,
    Rejected,
    Unknown,
    Running,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptStatus {
    pub attempt: AccountAttempt,
    pub outcome: Outcome,
}

// Secret request fields are never serialized into an operation record or Debug output.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountRequest {
    pub attempt: AccountAttempt,
    #[serde(default)]
    pub resume: bool,
    pub device_name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub invite: String,
    #[serde(default)]
    pub phrase: String,
    #[serde(default)]
    pub candidate_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AccountOperationRecord {
    version: u8,
    state_id: String,
    attempt: AccountAttempt,
    outcome: Outcome,
}

fn validate(attempt: &AccountAttempt) -> Result<(), AgentError> {
    if attempt.id.len() != 36
        || !attempt
            .id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-')
        || !valid_local_name(&attempt.profile)
        || !valid_local_name(&attempt.alias)
        || !valid_response_text(&attempt.device_name, 256)
        || attempt
            .candidate_id
            .as_deref()
            .is_some_and(|id| !valid_go_candidate_id(id))
        || !valid_typed_entity_id_hex(&attempt.host_id, HOST_ID_PREFIX)
    {
        return Err(invalid_request("Invalid account setup attempt."));
    }
    Ok(())
}

fn io_error(error: impl std::fmt::Display) -> AgentError {
    AgentError::unknown(format!(
        "Could not save or read account setup status: {error}"
    ))
}

fn read_operation_record(
    directory: &Path,
    state_id: &str,
    attempt: &AccountAttempt,
) -> Result<Outcome, AgentError> {
    let path = directory.join(format!("{}.json", attempt.id));
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Outcome::Unknown),
        Err(error) => return Err(io_error(error)),
    };
    let mut data = Vec::new();
    file.take(16_384).read_to_end(&mut data).map_err(io_error)?;
    let operation_record: AccountOperationRecord =
        serde_json::from_slice(&data).map_err(io_error)?;
    if operation_record.version != 2
        || operation_record.state_id != state_id
        || &operation_record.attempt != attempt
    {
        return Err(invalid_request(
            "This setup attempt does not match the current account or workspace.",
        ));
    }
    Ok(operation_record.outcome)
}

fn write_operation_record(
    directory: &Path,
    state_id: &str,
    attempt: &AccountAttempt,
    outcome: Outcome,
) -> Result<(), AgentError> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(io_error)?;
    serde_json::to_writer(
        &mut temporary,
        &AccountOperationRecord {
            version: 2,
            state_id: state_id.to_owned(),
            attempt: attempt.clone(),
            outcome,
        },
    )
    .map_err(io_error)?;
    temporary.flush().map_err(io_error)?;
    temporary.as_file().sync_all().map_err(io_error)?;
    temporary
        .persist(directory.join(format!("{}.json", attempt.id)))
        .map_err(io_error)?;
    #[cfg(unix)]
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(io_error)?;
    Ok(())
}

fn client_state_identity(state: &AppState) -> Result<String, AgentError> {
    let socket = state.agent.socket();
    let root = socket
        .parent()
        .ok_or_else(|| io_error("invalid agent socket path"))?;
    foks_client_app::portability::state_identity(root).map_err(io_error)
}

fn operation_record_directory(app: &tauri::AppHandle) -> Result<PathBuf, AgentError> {
    // Keep the existing directory name so upgrades retain setup outcomes.
    Ok(app
        .path()
        .app_data_dir()
        .map_err(io_error)?
        .join("account-operation-receipts-v1"))
}

#[tauri::command]
pub async fn first_run_operation_status(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    attempt: AccountAttempt,
) -> Result<AttemptStatus, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    validate(&attempt)?;
    let directory = operation_record_directory(&app)?;
    let state_id = client_state_identity(&state)?;
    let outcome = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join(format!("{}.lock", attempt.id)))
    {
        Ok(lock) => match lock.try_lock_exclusive() {
            Ok(()) => read_operation_record(&directory, &state_id, &attempt)?,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Outcome::Running,
            Err(error) => return Err(io_error(error)),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Outcome::Unknown,
        Err(error) => return Err(io_error(error)),
    };
    Ok(AttemptStatus { attempt, outcome })
}

#[tauri::command]
pub async fn run_first_run_account_operation(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    request: AccountRequest,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    validate(&request.attempt)?;
    if request.attempt.device_name != request.device_name
        || request.attempt.candidate_id.as_deref().unwrap_or("") != request.candidate_id
    {
        return Err(invalid_request(
            "The account request does not match its saved setup attempt.",
        ));
    }
    let directory = operation_record_directory(&app)?;
    fs::create_dir_all(&directory).map_err(io_error)?;
    let attempt = request.attempt.clone();
    let state_id = client_state_identity(&state)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(format!("{}.lock", attempt.id)))
        .map_err(io_error)?;
    lock.try_lock_exclusive().map_err(|_| {
        AgentError::new(
            "mutation-in-flight",
            "Account setup is still running.",
            false,
        )
    })?;
    let exists = directory.join(format!("{}.json", attempt.id)).exists();
    let previous = read_operation_record(&directory, &state_id, &attempt)?;
    if previous == Outcome::Complete {
        return Ok(MutationDto { applied: true });
    }
    if exists && (!request.resume || previous == Outcome::Rejected) {
        let mut error = AgentError::new(
            "ambiguous",
            "Check account setup status before continuing.",
            false,
        );
        error.ambiguous = true;
        return Err(error);
    }
    let status = servers::describe_server_status(
        webview.clone(),
        state.clone(),
        attempt.profile.clone(),
        Some(true),
    )
    .await?;
    if status.host.as_ref().map(|host| host.host_id.as_str()) != Some(attempt.host_id.as_str()) {
        return Err(invalid_request(
            "The server identity does not match this account setup.",
        ));
    }
    write_operation_record(&directory, &state_id, &attempt, Outcome::Unknown)?;
    let profile = attempt.profile.clone();
    let alias = attempt.alias.clone();
    let result = match (attempt.kind, request.resume) {
        (AccountKind::Signup, false) => {
            enrollment::create_first_run_account(
                app,
                webview,
                state,
                profile,
                alias,
                request.username,
                request.device_name,
                request.email,
                request.invite,
                None,
                None,
            )
            .await
        }
        (AccountKind::Signup, true) => {
            enrollment::resume_first_run_account(app, webview, state, profile, alias).await
        }
        (AccountKind::Recovery, false) => {
            enrollment::recover_owner_account(
                app,
                webview,
                state,
                profile,
                alias,
                request.phrase,
                request.device_name,
            )
            .await
        }
        (AccountKind::Recovery, true) => {
            enrollment::resume_owner_recovery(
                app,
                webview,
                state,
                profile,
                alias,
                request.phrase,
                request.device_name,
            )
            .await
        }
        (AccountKind::Copy, false) => enrollment::copy_go_profile_device(
            app,
            webview,
            state,
            request.candidate_id,
            profile,
            alias,
        )
        .await
        .map(|_| MutationDto { applied: true }),
        (AccountKind::Pairing, true) => enrollment::resume_go_profile_pairing(
            app,
            webview,
            state,
            request.candidate_id,
            profile,
            alias,
        )
        .await
        .map(|_| MutationDto { applied: true }),
        (AccountKind::Pairing, false) => enrollment::accept_go_profile_pairing(
            app,
            webview,
            state,
            enrollment::GoProfilePairingRequest {
                candidate_id: request.candidate_id,
                profile,
                target_alias: alias,
                device_name: request.device_name,
                phrase: request.phrase,
            },
        )
        .await
        .map(|_| MutationDto { applied: true }),
        (AccountKind::Copy, true) => Err(invalid_request(
            "Copying a device does not have a resume operation.",
        )),
    };
    let outcome = match &result {
        Ok(_) => Outcome::Complete,
        Err(error)
            if !exists
                && !request.resume
                && !error.ambiguous
                && !error.fatal
                && ![
                    "agent-lost",
                    "deadline-exceeded",
                    "response-binding",
                    "unknown",
                    "mutation-in-flight",
                ]
                .contains(&error.code.as_str()) =>
        {
            Outcome::Rejected
        }
        Err(_) => Outcome::Unknown,
    };
    if let Err(error) = write_operation_record(&directory, &state_id, &attempt, outcome) {
        let mut error = error;
        error.ambiguous = true;
        error.code = "ambiguous".to_owned();
        return Err(error);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn attempt() -> AccountAttempt {
        AccountAttempt {
            id: "11111111-1111-1111-1111-111111111111".into(),
            profile: "personal".into(),
            alias: "me".into(),
            device_name: "Mac".into(),
            candidate_id: None,
            host_id: format!("02{}", "1".repeat(64)),
            kind: AccountKind::Signup,
        }
    }
    #[test]
    fn operation_records_are_durable_and_bound_to_attempt_host_alias_and_client_state() {
        let dir = tempfile::tempdir().unwrap();
        let state_id = "aabbccdd";
        let a = attempt();
        assert_eq!(
            read_operation_record(dir.path(), state_id, &a).unwrap(),
            Outcome::Unknown
        );
        write_operation_record(dir.path(), state_id, &a, Outcome::Unknown).unwrap();
        write_operation_record(dir.path(), state_id, &a, Outcome::Complete).unwrap();
        assert_eq!(
            read_operation_record(dir.path(), state_id, &a).unwrap(),
            Outcome::Complete
        );
        assert!(read_operation_record(dir.path(), "eeff0011", &a).is_err());
        let mut wrong = a.clone();
        wrong.alias = "other".into();
        assert!(read_operation_record(dir.path(), state_id, &wrong).is_err());
        let mut wrong = a;
        wrong.host_id = format!("02{}", "2".repeat(64));
        assert!(read_operation_record(dir.path(), state_id, &wrong).is_err());
    }
    #[test]
    fn operation_records_are_independent_of_the_state_root_path() {
        let dir = tempfile::tempdir().unwrap();
        let state_id = "aabbccdd";
        let a = attempt();
        write_operation_record(dir.path(), state_id, &a, Outcome::Complete).unwrap();
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join(format!("{}.json", a.id))).unwrap())
                .unwrap();
        assert_eq!(stored["version"], 2);
        assert_eq!(stored["state_id"], state_id);
        assert!(stored.get("socket").is_none());
        assert_eq!(
            read_operation_record(dir.path(), state_id, &a).unwrap(),
            Outcome::Complete
        );
    }
    #[test]
    fn version_one_operation_records_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let a = attempt();
        let legacy = serde_json::json!({
            "version": 1,
            "socket": "/test/agent.sock",
            "attempt": {
                "id": a.id,
                "profile": a.profile,
                "hostId": a.host_id,
                "alias": a.alias,
                "deviceName": a.device_name,
                "kind": "signup",
            },
            "outcome": "complete",
        });
        fs::write(
            dir.path().join(format!("{}.json", a.id)),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        assert!(read_operation_record(dir.path(), "aabbccdd", &a).is_err());
    }
    #[test]
    fn operation_record_names_cannot_escape_the_directory() {
        let mut a = attempt();
        a.id = "../outside".into();
        assert!(validate(&a).is_err());
    }
    #[test]
    fn operation_record_lock_excludes_duplicate_dispatch_and_releases_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attempt.lock");
        let first = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        first.try_lock_exclusive().unwrap();
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(second.try_lock_exclusive().is_err());
        drop(first);
        second.try_lock_exclusive().unwrap();
    }
    #[test]
    fn corrupt_operation_record_does_not_authorize_retry() {
        let dir = tempfile::tempdir().unwrap();
        let a = attempt();
        fs::write(dir.path().join(format!("{}.json", a.id)), b"{partial").unwrap();
        assert!(read_operation_record(dir.path(), "aabbccdd", &a).is_err());
    }
}
