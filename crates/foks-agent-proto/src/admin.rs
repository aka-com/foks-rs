//! Native-only hosted admin handoff. Session URLs are not UI data.
use crate::SecretString;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AdminAction {
    Policy,
    Configure { destination: String },
    Prepare { pin: Option<SecretString> },
}
impl AdminAction {
    pub fn validate(&self) -> bool {
        match self {
            Self::Policy => true,
            Self::Configure { destination } => destination.len() <= 2048,
            Self::Prepare { pin } => pin
                .as_ref()
                .is_none_or(|p| (1..=32).contains(&p.expose().len())),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminNavigation {
    pub profile: String,
    pub account_alias: String,
    pub host_id: String,
    pub uid: String,
    pub destination: String,
    pub url: SecretString,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminPolicy {
    pub profile: String,
    pub account_alias: String,
    pub host_id: String,
    pub uid: String,
    pub destination: String,
}

impl std::fmt::Debug for AdminAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Policy => "AdminPolicy",
            Self::Configure { .. } => "ConfigureAdmin(<redacted>)",
            Self::Prepare { .. } => "PrepareAdmin(<redacted>)",
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_never_debug_print_pins_or_pasted_sessions() {
        for action in [
            AdminAction::Configure {
                destination: "https://host/?s=secret".into(),
            },
            AdminAction::Prepare {
                pin: Some(SecretString::new("secret")),
            },
        ] {
            assert!(!format!("{action:?}").contains("secret"));
        }
    }
}
