//! Local OAuth handles and progress. Provider tokens never cross this boundary.
use crate::SecretString;
pub use foks_proto::{SsoAccountStatusView, SsoPurpose};
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
        purpose: foks_proto::SsoPurpose,
        pin: Option<SecretString>,
    },
    AccountStatus {
        pin: Option<SecretString>,
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
    pub operation_id: Option<String>,
    pub account_alias: String,
    pub purpose: foks_proto::SsoPurpose,
    pub account_status: Option<foks_proto::SsoAccountStatusView>,
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
            Self::Begin { pin, .. } | Self::AccountStatus { pin } => {
                pin.as_ref().is_none_or(|p| p.expose().len() <= 32)
            }
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
                    && device_name.len() <= foks_proto::MAXIMUM_DEVICE_NAME_BYTES
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
                    && device_name.len() <= foks_proto::MAXIMUM_DEVICE_NAME_BYTES
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

    #[test]
    fn sso_device_names_allow_unicode_display_bytes() {
        for (name, expected) in [("é".repeat(200), true), ("é".repeat(2049), false)] {
            let signup = SsoAction::FinishSignup {
                operation_id: "a".repeat(32),
                device_name: name.clone(),
                invite: SecretString::new(""),
                passphrase: None,
            };
            assert_eq!(signup.validate(), expected);
            let yubi = SsoAction::BeginYubiSignup {
                card_serial: 1,
                signing_slot: 0x82,
                pq_slot: 0x83,
                pin: SecretString::new("123456"),
                device_name: name,
                invite: SecretString::new(""),
            };
            assert_eq!(yubi.validate(), expected);
        }
    }
}
