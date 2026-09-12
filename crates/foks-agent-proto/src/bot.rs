use crate::{KvRole, SecretString};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum BotAction {
    Load {
        token: SecretString,
    },
    Revoke {
        device_id: String,
        pin: Option<SecretString>,
    },
    Unload,
    List,
    Prepare {
        role: KvRole,
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
    Export {
        operation_id: String,
    },
}
impl BotAction {
    pub fn validate(&self) -> bool {
        match self {
            Self::Revoke { device_id, pin } => {
                device_id.len() == 66
                    && device_id.starts_with("13")
                    && device_id
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    && pin
                        .as_ref()
                        .is_none_or(|p| (1..=32).contains(&p.expose().len()))
            }
            Self::Load { token } => token.expose().len() == 37,
            Self::Attempt { operation_id, pin } | Self::Status { operation_id, pin } => {
                crate::account::RenameAction::Status {
                    operation_id: operation_id.clone(),
                    pin: pin.clone(),
                }
                .validate()
            }
            Self::Cancel { operation_id } | Self::Export { operation_id } => {
                crate::account::RenameAction::Cancel {
                    operation_id: operation_id.clone(),
                }
                .validate()
            }
            Self::Prepare { pin, .. } => pin
                .as_ref()
                .is_none_or(|p| (1..=32).contains(&p.expose().len())),
            _ => true,
        }
    }
    pub fn pin(&self) -> Option<&SecretString> {
        match self {
            Self::Revoke { pin, .. }
            | Self::Prepare { pin, .. }
            | Self::Attempt { pin, .. }
            | Self::Status { pin, .. } => pin.as_ref(),
            _ => None,
        }
    }
}
