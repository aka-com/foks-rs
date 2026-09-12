//! Local OAuth handles and progress. Provider tokens never cross this boundary.
use crate::SecretString;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SsoAction {
    BeginYubiSignup {
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
        device_name: String,
        invite: SecretString,
    },
    FinishYubiSignup {
        operation_id: String,
        pin: SecretString,
    },
    Begin {
        for_login: bool,
    },
    Status {
        operation_id: String,
    },
    Poll {
        operation_id: String,
    },
    Cancel {
        operation_id: String,
    },
    FinishLogin {
        operation_id: String,
        pin: Option<SecretString>,
    },
    FinishSignup {
        operation_id: String,
        device_name: String,
        invite: SecretString,
        passphrase: Option<SecretString>,
    },
}
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SsoProgress {
    pub operation_id: String,
    pub account_alias: String,
    pub for_login: bool,
    pub state: String,
    pub browser_url: Option<String>,
    pub expires_at_ms: u64,
    pub service_access: bool,
}
impl std::fmt::Debug for SsoProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoProgress")
            .field("state", &self.state)
            .field("service_access", &self.service_access)
            .finish_non_exhaustive()
    }
}
impl SsoAction {
    pub fn validate(&self) -> bool {
        fn handle(s: &str) -> bool {
            s.len() == 32
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }
        match self {
            Self::Begin { .. } => true,
            Self::BeginYubiSignup {
                card_serial,
                signing_slot,
                pq_slot,
                pin,
                device_name,
                invite,
            } => {
                *card_serial > 0
                    && signing_slot != pq_slot
                    && pin.expose().len() <= 32
                    && !device_name.is_empty()
                    && device_name.len() <= 256
                    && invite.expose().len() <= 4096
            }
            Self::FinishYubiSignup { operation_id, pin } => {
                handle(operation_id) && pin.expose().len() <= 32
            }
            Self::Status { operation_id }
            | Self::Poll { operation_id }
            | Self::Cancel { operation_id } => handle(operation_id),
            Self::FinishLogin { operation_id, pin } => {
                handle(operation_id) && pin.as_ref().is_none_or(|p| p.expose().len() <= 32)
            }
            Self::FinishSignup {
                operation_id,
                device_name,
                invite,
                passphrase,
            } => {
                handle(operation_id)
                    && !device_name.trim().is_empty()
                    && device_name.len() <= 256
                    && invite.expose().len() <= 4096
                    && passphrase.as_ref().is_none_or(|p| p.expose().len() <= 1024)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flow_actions_are_bounded_and_secrets_never_enter_debug_output() {
        let good = SsoAction::FinishLogin {
            operation_id: "a".repeat(32),
            pin: Some(SecretString::new("654321")),
        };
        assert!(good.validate());
        assert!(!format!("{good:?}").contains("654321"));
        assert_eq!(
            serde_json::from_value::<SsoAction>(serde_json::to_value(&good).unwrap()).unwrap(),
            good
        );
        assert!(!SsoAction::Status {
            operation_id: "../another".into()
        }
        .validate());
        assert!(serde_json::from_str::<SsoAction>(r#"{"action":"status","operation_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","id_token":"secret"}"#).is_err());
    }
}
