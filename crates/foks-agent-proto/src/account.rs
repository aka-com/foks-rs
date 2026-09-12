//! Account mutation handles and bounded frontend actions.
use crate::SecretString;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum RenameAction {
    Prepare {
        username: String,
        pin: Option<SecretString>,
    },
    Attempt {
        operation_id: String,
        pin: Option<SecretString>,
    },
    Status {
        operation_id: String,
        pin: Option<SecretString>,
    },
    Cancel {
        operation_id: String,
    },
}
impl RenameAction {
    pub fn operation_id(&self) -> Option<&str> {
        match self {
            Self::Prepare { .. } => None,
            Self::Attempt { operation_id, .. }
            | Self::Status { operation_id, .. }
            | Self::Cancel { operation_id } => Some(operation_id),
        }
    }
    pub fn pin(&self) -> Option<&SecretString> {
        match self {
            Self::Prepare { pin, .. } | Self::Attempt { pin, .. } | Self::Status { pin, .. } => {
                pin.as_ref()
            }
            Self::Cancel { .. } => None,
        }
    }
    pub fn validate(&self) -> bool {
        self.operation_id().is_none_or(|id| {
            id.len() == 32
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }) && self
            .pin()
            .is_none_or(|p| (1..=32).contains(&p.expose().len()))
            && match self {
                Self::Prepare { username, .. } => !username.is_empty() && username.len() <= 256,
                _ => true,
            }
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenameProgress {
    pub operation_id: String,
    pub account_alias: String,
    pub state: String,
    pub target: Option<String>,
    pub current_username: Option<String>,
    pub hardware_required: bool,
}
impl RenameProgress {
    pub fn validate(&self) -> bool {
        RenameAction::Cancel {
            operation_id: self.operation_id.clone(),
        }
        .validate()
            && matches!(
                self.state.as_str(),
                "prepared"
                    | "submitting"
                    | "submission-unknown"
                    | "remote-verified"
                    | "complete"
                    | "rejected"
            )
            && self.target.as_ref().is_none_or(|s| s.len() <= 256)
            && self
                .current_username
                .as_ref()
                .is_none_or(|s| s.len() <= 4096)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_actions_are_bounded_and_pin_debug_is_redacted() {
        let action = RenameAction::Prepare {
            username: "alice".into(),
            pin: Some(SecretString::new("654321")),
        };
        assert!(action.validate());
        assert!(!format!("{action:?}").contains("654321"));
        assert!(!RenameAction::Cancel {
            operation_id: "A".repeat(32)
        }
        .validate());
        assert!(!RenameAction::Prepare {
            username: "é".repeat(129),
            pin: None
        }
        .validate());
        assert!(serde_json::from_value::<RenameAction>(
            serde_json::json!({"action":"cancel","operation_id":"1".repeat(32),"pin":"secret"})
        )
        .is_err());
    }
}
