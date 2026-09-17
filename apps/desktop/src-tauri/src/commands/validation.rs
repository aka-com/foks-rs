//! Shared request bounds, identity checks, secret handling, and window validation.

use crate::agent::AgentError;
use crate::commands::context::MAIN;
use foks_agent_proto::{SecretString, YubiRetryConfiguration};
use foks_desktop::CatalogSnapshot;
use serde::Deserialize;
use zeroize::Zeroizing;

/// Maximum text payload size (2048 bytes minus 8-byte framing header).
pub(super) const MAXIMUM_TEXT_ITEM_BYTES: usize = 2048 - 8;
pub(super) const MAXIMUM_CLIPBOARD_TEXT_BYTES: usize = 128 * 1024;
pub(super) const MAXIMUM_INVITE_BYTES: usize = 4 * 1024;
pub(super) const MAXIMUM_PASSPHRASE_BYTES: usize = 1024;
pub(super) const MAXIMUM_RECOVERY_PHRASE_BYTES: usize = 4 * 1024;
pub(super) const MAXIMUM_FIRST_RUN_ROWS: usize = 4 * 1024;
pub(super) const MAXIMUM_SYNC_FACTS: usize = 10_000_000;
const ENTITY_ID_HEX_BYTES: usize = 66;
pub(super) const USER_ID_PREFIX: &str = "01";
pub(super) const HOST_ID_PREFIX: &str = "02";
pub(super) const NAMED_TEAM_ID_PREFIX: &str = "03";
pub(super) const DEVICE_ID_PREFIX: &str = "04";
const YUBI_ID_PREFIX: &str = "08";
pub(super) const YUBI_ID_HEX_BYTES: usize = 68;
pub(super) const SUBKEY_ID_PREFIX: &str = "0d";
pub(super) const BACKUP_ID_PREFIX: &str = "10";
pub(super) const AD_HOC_TEAM_ID_PREFIX: &str = "14";

pub(super) fn require_profile_available(
    catalog: &CatalogSnapshot,
    profile: &str,
) -> Result<(), AgentError> {
    if catalog.profile_blocked(profile) {
        Err(AgentError::new(
            "capability-unavailable",
            "This server is currently blocked. Refresh its status before continuing.",
            false,
        ))
    } else {
        Ok(())
    }
}

pub(super) fn required_field(value: &str, message: &'static str) -> Result<String, AgentError> {
    let value = value.trim();
    if value.is_empty() {
        Err(invalid_request(message))
    } else {
        Ok(value.to_owned())
    }
}

pub(super) fn valid_response_text(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value.trim() == value
        && !value.contains(['\0', '\r', '\n'])
}

pub(super) fn valid_local_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(super) fn bounded_local_name(value: &str, message: &'static str) -> Result<String, AgentError> {
    if valid_local_name(value) {
        Ok(value.to_owned())
    } else {
        Err(invalid_request(message))
    }
}

fn valid_entity_id_hex(value: &str) -> bool {
    value.len() == ENTITY_ID_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn valid_typed_entity_id_hex(value: &str, prefix: &str) -> bool {
    valid_entity_id_hex(value) && value.starts_with(prefix)
}

pub(super) fn valid_device_member_id_hex(value: &str) -> bool {
    valid_typed_entity_id_hex(value, DEVICE_ID_PREFIX)
        || (value.len() == YUBI_ID_HEX_BYTES
            && value.starts_with(YUBI_ID_PREFIX)
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

pub(super) fn valid_go_candidate_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn valid_operation_id_hex(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn require_response_row_cap(
    value: &serde_json::Value,
    noun: &str,
) -> Result<(), AgentError> {
    match value.as_array() {
        Some(rows) if rows.len() <= MAXIMUM_FIRST_RUN_ROWS => Ok(()),
        Some(_) => Err(invalid_response(format!(
            "Response contained too many {noun}."
        ))),
        None => Err(invalid_response(format!(
            "Background service returned an invalid list of {noun}."
        ))),
    }
}

pub(super) fn require_nested_response_row_cap(
    value: &serde_json::Value,
    field: &str,
    noun: &str,
) -> Result<(), AgentError> {
    match value.get(field).and_then(serde_json::Value::as_array) {
        Some(rows) if rows.len() <= MAXIMUM_FIRST_RUN_ROWS => Ok(()),
        Some(_) => Err(invalid_response(format!(
            "Response exceeded maximum allowed {noun}."
        ))),
        None => Err(invalid_response(format!(
            "The agent returned an invalid {noun} response."
        ))),
    }
}

pub(super) fn bounded_field(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<String, AgentError> {
    let value = required_field(value, message)?;
    if value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(invalid_request(message))
    } else {
        Ok(value)
    }
}

pub(super) fn optional_bounded_field(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<String, AgentError> {
    if value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(invalid_request(message))
    } else {
        Ok(value.trim().to_owned())
    }
}

pub(super) fn confirmed_passphrase(
    passphrase: String,
    confirmation: String,
) -> Result<SecretString, AgentError> {
    let passphrase = Zeroizing::new(passphrase);
    let confirmation = Zeroizing::new(confirmation);
    if passphrase.as_str() != confirmation.as_str() {
        return Err(invalid_request("Passphrase confirmation does not match."));
    }
    if passphrase.is_empty()
        || passphrase.len() > MAXIMUM_PASSPHRASE_BYTES
        || passphrase.contains(['\0', '\r', '\n'])
    {
        return Err(invalid_request(
            "A passphrase is required and cannot exceed 1,024 characters.",
        ));
    }
    Ok(SecretString::new(passphrase.as_str()))
}

pub(super) fn optional_confirmed_passphrase(
    passphrase: Option<String>,
    confirmation: Option<String>,
) -> Result<Option<SecretString>, AgentError> {
    match (passphrase, confirmation) {
        (None, None) => Ok(None),
        (Some(passphrase), Some(confirmation)) => {
            confirmed_passphrase(passphrase, confirmation).map(Some)
        }
        _ => Err(invalid_request(
            "Provide both the passphrase and confirmation, or leave both blank.",
        )),
    }
}

pub(super) fn bounded_secret(
    value: String,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<SecretString, AgentError> {
    let value = Zeroizing::new(value);
    if value.is_empty() || value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(invalid_request(message))
    } else {
        Ok(SecretString::new(value.as_str()))
    }
}

pub(super) fn yubi_slots(signing_slot: u8, pq_slot: u8) -> Result<(u8, u8), AgentError> {
    let retired = |slot: u8| (0x82..=0x95).contains(&slot);
    if signing_slot == pq_slot || !retired(signing_slot) || !retired(pq_slot) {
        Err(invalid_request(
            "Select two distinct retired key slots between 0x82 and 0x95.",
        ))
    } else {
        Ok((signing_slot, pq_slot))
    }
}

pub(super) fn yubi_retry_configuration(
    puk: String,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<YubiRetryConfiguration, AgentError> {
    if pin_attempts == 0 || puk_attempts == 0 {
        return Err(invalid_request(
            "PIN and unlock-code retry counts must be greater than zero.",
        ));
    }
    Ok(YubiRetryConfiguration {
        puk: bounded_secret(
            puk,
            128,
            "Unlock code cannot be empty and must be at most 128 bytes.",
        )?,
        pin_attempts,
        puk_attempts,
    })
}

pub(super) fn pairing_phrase(value: String) -> Result<SecretString, AgentError> {
    let value = Zeroizing::new(value);
    if value.len() > MAXIMUM_RECOVERY_PHRASE_BYTES
        || foks_crypto::KexSecret::from_phrase(value.as_str()).is_err()
    {
        return Err(invalid_request("Enter a valid device-pairing phrase."));
    }
    Ok(SecretString::new(value.as_str()))
}

/// A username the server will accept, checked here so a space or capital is
/// refused with a sentence instead of a normalization error from the client.
pub(super) fn valid_username(value: &str) -> Result<String, AgentError> {
    let value = bounded_field(value, 256, "Enter a username.")?;
    if foks_verify::normalize_username(value.as_bytes()).is_none() {
        return Err(invalid_request(
            "Usernames use 3 to 25 letters, numbers, and single underscores.",
        ));
    }
    Ok(value)
}

/// A device name the server will accept, by the same rules the client applies.
pub(super) fn valid_device_name(value: &str) -> Result<String, AgentError> {
    let value = bounded_field(value, 256, "Enter a device name.")?;
    if foks_verify::normalize_device_name(value.as_bytes()).is_none() {
        return Err(invalid_request(
            "Device names use 2 to 200 letters, numbers, spaces, and . _ + ' -, and start with a letter or number.",
        ));
    }
    Ok(value)
}

/// A paper key phrase, parsed before it leaves the app so a typo is named by
/// position rather than reported as a failed recovery.
pub(super) fn backup_phrase(value: String) -> Result<SecretString, AgentError> {
    let value = Zeroizing::new(value);
    if value.is_empty()
        || value.len() > MAXIMUM_RECOVERY_PHRASE_BYTES
        || value.contains(['\0', '\r', '\n'])
    {
        return Err(invalid_request(
            "Recovery phrase must be a single line of at most 4,096 bytes.",
        ));
    }
    if let Err(error) = foks_crypto::BackupKey::from_phrase(&value) {
        use foks_crypto::BackupPhraseError as E;
        return Err(invalid_request(match error {
            E::TokenCount { found } => format!(
                "A paper key has {} words and numbers; this one has {found}.",
                foks_crypto::BACKUP_PHRASE_TOKENS
            ),
            E::Word { index } => format!(
                "Word {} of the paper key is not a recognized word. Check its spelling.",
                index + 1
            ),
            E::Number { index } | E::NumberRange { index } => format!(
                "Number {} of the paper key should be a whole number from 0 to 8191.",
                index + 1
            ),
            _ => "That paper key is not valid.".to_owned(),
        }));
    }
    Ok(SecretString::new(value.as_str()))
}

pub(super) fn exact_profile_confirmation(
    profile: &str,
    confirmation: &str,
    action: &str,
) -> Result<(), AgentError> {
    if confirmation == profile {
        Ok(())
    } else {
        Err(invalid_request(format!(
            "Enter the exact server profile name to {action} it."
        )))
    }
}

pub(super) fn positive_recovery_serial_with(
    mut fill: impl FnMut(&mut [u8]) -> Result<(), ()>,
) -> Result<u64, AgentError> {
    for _ in 0..4 {
        let mut bytes = [0u8; 8];
        fill(&mut bytes).map_err(|()| {
            AgentError::new(
                "randomness-unavailable",
                "Failed to generate a secure random device serial number.",
                true,
            )
        })?;
        let serial = u64::from_le_bytes(bytes);
        if serial != 0 {
            return Ok(serial);
        }
    }
    Err(AgentError::new(
        "randomness-unavailable",
        "Failed to produce a valid random device serial number.",
        true,
    ))
}

pub(super) fn positive_recovery_serial() -> Result<u64, AgentError> {
    positive_recovery_serial_with(|bytes| getrandom::fill(bytes).map_err(|_| ()))
}

pub fn require_main_window(webview: &tauri::Webview) -> Result<(), AgentError> {
    if main_window_allowed(webview.label()) {
        Ok(())
    } else {
        Err(AgentError::new(
            "window-not-allowed",
            "This request did not come from the main application window.",
            false,
        ))
    }
}

pub(super) fn main_window_allowed(label: &str) -> bool {
    label == MAIN
}

pub(super) fn valid_typed_yubi_id_hex(value: &str) -> bool {
    value.len() == YUBI_ID_HEX_BYTES
        && value.starts_with(YUBI_ID_PREFIX)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn serialize_secret<S>(
    value: &Zeroizing<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}

pub(super) fn deserialize_secret<'de, D>(deserializer: D) -> Result<Zeroizing<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(Zeroizing::new)
}

pub(super) fn invalid_response(message: impl Into<String>) -> AgentError {
    AgentError::new("invalid-response", message, false)
}

pub(super) const DOWNLOAD_CHUNK_BYTES: u32 = 128 * 1024;

pub(super) fn invalid_request(message: impl Into<String>) -> AgentError {
    AgentError::new("invalid-request", message, false)
}
