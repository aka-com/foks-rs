//! Pure agent [`Operation`](foks_agent_proto::Operation) builders shared by
//! the Tauri command layer and `foks-desktop-backend`.
//!
//! Both callers map their own input types (renderer DTOs, command-line
//! arguments) onto these functions, so the input rules live here once and the
//! real-agent backend transcript exercises the same rules as the shipping app.

pub mod groups;

/// A caller-supplied value that an operation builder rejects.
///
/// The message is user-facing text. The Tauri command layer reports it as an
/// `invalid-request` error; the scripted backend prints it and exits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidInput(&'static str);

impl InvalidInput {
    pub fn message(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for InvalidInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for InvalidInput {}

/// Trims `value` and rejects it when nothing remains.
fn required_field(value: &str, message: &'static str) -> Result<String, InvalidInput> {
    let value = value.trim();
    if value.is_empty() {
        Err(InvalidInput(message))
    } else {
        Ok(value.to_owned())
    }
}

/// Accepts only a lowercase hex entity id of the given type prefix.
fn typed_entity_id_hex(
    value: &str,
    prefix: &str,
    message: &'static str,
) -> Result<String, InvalidInput> {
    const ENTITY_ID_HEX_BYTES: usize = 66;
    if value.len() == ENTITY_ID_HEX_BYTES
        && value.starts_with(prefix)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(value.to_owned())
    } else {
        Err(InvalidInput(message))
    }
}
