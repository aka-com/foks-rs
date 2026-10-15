//! Shared request bounds, identity checks, secret handling, and window validation.

use crate::agent::AgentError;
use crate::commands::context::MAIN;
use foks_agent_proto::{SecretString, YubiRetryConfiguration};
use foks_desktop::ops::{self, accounts};
use foks_desktop::CatalogSnapshot;
use serde::Deserialize;
use zeroize::Zeroizing;

// The field checks and entity-id prefixes shared with `foks-desktop-backend`
// live in `foks_desktop::ops`; the wrappers below only convert their error.
pub(super) use accounts::{
    MAXIMUM_INVITE_BYTES, MAXIMUM_PASSPHRASE_BYTES, MAXIMUM_RECOVERY_PHRASE_BYTES,
};
pub(super) use ops::{
    valid_local_name, valid_typed_entity_id_hex, AD_HOC_TEAM_ID_PREFIX, BACKUP_ID_PREFIX,
    DEVICE_ID_PREFIX, HOST_ID_PREFIX, NAMED_TEAM_ID_PREFIX, SUBKEY_ID_PREFIX, USER_ID_PREFIX,
};

/// Maximum text payload size (2048 bytes minus 8-byte framing header).
pub(super) const MAXIMUM_TEXT_ITEM_BYTES: usize = 2048 - 8;
pub(super) const MAXIMUM_CLIPBOARD_TEXT_BYTES: usize = 128 * 1024;
pub(super) const MAXIMUM_FIRST_RUN_ROWS: usize = 4 * 1024;
pub(super) const MAXIMUM_SYNC_FACTS: usize = 10_000_000;
const YUBI_ID_PREFIX: &str = "08";
pub(super) const YUBI_ID_HEX_BYTES: usize = 68;

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
    Ok(ops::required_field(value, message)?)
}

pub(super) fn valid_response_text(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value.trim() == value
        && !value.contains(['\0', '\r', '\n'])
}

pub(super) fn bounded_local_name(value: &str, message: &'static str) -> Result<String, AgentError> {
    Ok(ops::bounded_local_name(value, message)?)
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
    Ok(ops::bounded_field(value, maximum_bytes, message)?)
}

pub(super) fn optional_bounded_field(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<String, AgentError> {
    Ok(ops::optional_bounded_field(value, maximum_bytes, message)?)
}

pub(super) fn confirmed_passphrase(
    passphrase: String,
    confirmation: String,
) -> Result<SecretString, AgentError> {
    let passphrase = Zeroizing::new(passphrase);
    let confirmation = Zeroizing::new(confirmation);
    Ok(accounts::confirmed_passphrase(&passphrase, &confirmation)?)
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
    Ok(ops::bounded_secret(&value, maximum_bytes, message)?)
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
    // `foks_yubi::PinRetryConfiguration::new` accepts one to fifteen attempts.
    let allowed = |attempts: u8| (1..=15).contains(&attempts);
    if !allowed(pin_attempts) || !allowed(puk_attempts) {
        return Err(invalid_request(
            "PIN and unlock-code retry counts must be between 1 and 15.",
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

pub(super) fn valid_device_name(value: &str) -> Result<String, AgentError> {
    Ok(accounts::valid_device_name(value)?)
}

pub(super) fn exact_profile_confirmation(
    profile_name: &str,
    confirmed_profile_name: &str,
    action: &str,
) -> Result<(), AgentError> {
    if confirmed_profile_name == profile_name {
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

/// One download chunk's plaintext. Byte payloads are base64 on the local
/// protocol, so the agent's own payload bound is what fits its frame, and a
/// longer request is refused as invalid; asking for exactly that bound is the
/// fewest round trips a download can take.
pub(super) const DOWNLOAD_CHUNK_BYTES: u32 = foks_agent_proto::MAXIMUM_KV_PAYLOAD_BYTES as u32;

/// The most a download will assemble before it gives up. Legacy files may
/// report no size, so the end-of-file flag always ends the loop; this bounds
/// the loop against an agent that never sets it. It is the protocol's own
/// large-file ceiling, so it refuses nothing a real file can reach.
pub(super) const MAXIMUM_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;

pub(super) fn invalid_request(message: impl Into<String>) -> AgentError {
    AgentError::new("invalid-request", message, false)
}

/// Input rejected by a shared `foks_desktop::ops` builder is a request error.
impl From<foks_desktop::ops::InvalidInput> for AgentError {
    fn from(error: foks_desktop::ops::InvalidInput) -> Self {
        invalid_request(error.message())
    }
}
