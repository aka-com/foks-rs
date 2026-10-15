//! Pure agent [`Operation`](foks_agent_proto::Operation) builders shared by
//! the Tauri command layer and `foks-desktop-backend`.
//!
//! Both callers map their own input types (renderer DTOs, command-line
//! arguments) onto these functions, so the input rules live here once and the
//! real-agent backend transcript exercises the same rules as the shipping app.
//! The field checks below are public so the Tauri layer's other commands apply
//! the same implementation rather than a copy.

use std::borrow::Cow;

use foks_agent_proto::SecretString;

pub mod accounts;
pub mod groups;

/// Length of a lowercase hex entity id: a one-byte type prefix and a 32-byte
/// body.
const ENTITY_ID_HEX_BYTES: usize = 66;
pub const USER_ID_PREFIX: &str = "01";
pub const HOST_ID_PREFIX: &str = "02";
pub const NAMED_TEAM_ID_PREFIX: &str = "03";
pub const DEVICE_ID_PREFIX: &str = "04";
pub const SUBKEY_ID_PREFIX: &str = "0d";
pub const BACKUP_ID_PREFIX: &str = "10";
pub const AD_HOC_TEAM_ID_PREFIX: &str = "14";

/// A caller-supplied value that an operation builder rejects.
///
/// The message is user-facing text. The Tauri command layer reports it as an
/// `invalid-request` error; the scripted backend prints it and exits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidInput(Cow<'static, str>);

impl InvalidInput {
    pub fn new(message: impl Into<Cow<'static, str>>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for InvalidInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for InvalidInput {}

/// Trims `value` and rejects it when nothing remains.
pub fn required_field(value: &str, message: &'static str) -> Result<String, InvalidInput> {
    let value = value.trim();
    if value.is_empty() {
        Err(InvalidInput::new(message))
    } else {
        Ok(value.to_owned())
    }
}

/// A required single-line field of at most `maximum_bytes` after trimming.
pub fn bounded_field(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<String, InvalidInput> {
    let value = required_field(value, message)?;
    if value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(InvalidInput::new(message))
    } else {
        Ok(value)
    }
}

/// An optional single-line field of at most `maximum_bytes`, trimmed.
pub fn optional_bounded_field(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<String, InvalidInput> {
    if value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(InvalidInput::new(message))
    } else {
        Ok(value.trim().to_owned())
    }
}

/// A nonempty single-line secret of at most `maximum_bytes`, taken verbatim.
pub fn bounded_secret(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<SecretString, InvalidInput> {
    if value.is_empty() || value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(InvalidInput::new(message))
    } else {
        Ok(SecretString::new(value))
    }
}

/// A local profile, account, team or backup alias: 1 to 64 ASCII letters,
/// digits, hyphens or underscores.
pub fn valid_local_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Accepts a local name verbatim. It is never trimmed or normalized, so an
/// accepted value equals the input.
pub fn bounded_local_name(value: &str, message: &'static str) -> Result<String, InvalidInput> {
    if valid_local_name(value) {
        Ok(value.to_owned())
    } else {
        Err(InvalidInput::new(message))
    }
}

/// Whether `value` is a lowercase hex entity id of the type named by `prefix`.
pub fn valid_typed_entity_id_hex(value: &str, prefix: &str) -> bool {
    value.len() == ENTITY_ID_HEX_BYTES
        && value.starts_with(prefix)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
