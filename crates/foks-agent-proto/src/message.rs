use serde::{Deserialize, Serialize};
use zeroize::Zeroize as _;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub operation: Operation,
}

impl Request {
    pub fn new(id: u64, operation: Operation) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            operation,
        }
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum Operation {
    Ping,
    ListProfiles,
    Probe {
        profile: String,
    },
    ListAccounts {
        profile: String,
    },
    CreateAccount {
        profile: String,
        alias: String,
        username: String,
        device_name: String,
        email: String,
        invite: String,
        passphrase: Option<SecretString>,
    },
    SetPassphrase {
        profile: String,
        alias: String,
        passphrase: SecretString,
    },
    ChangePassphrase {
        profile: String,
        alias: String,
        passphrase: SecretString,
    },
    VerifyPassphrase {
        profile: String,
        alias: String,
        passphrase: SecretString,
    },
    SyncAccount {
        profile: String,
        alias: String,
    },
    ListYubiCards {
        profile: String,
    },
    ListYubiAccounts {
        profile: String,
    },
    CreateYubiAccount {
        profile: String,
        alias: String,
        username: String,
        device_name: String,
        email: String,
        invite: SecretString,
        passphrase: Option<SecretString>,
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
    },
    ResumeYubiAccount {
        profile: String,
        alias: String,
        pin: SecretString,
    },
    ProvisionYubiDevice {
        profile: String,
        source_alias: String,
        target_alias: String,
        device_name: String,
        serial: u64,
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
    },
    SyncYubiAccount {
        profile: String,
        alias: String,
        pin: SecretString,
    },
    YubiPinStatus {
        profile: String,
        alias: String,
    },
    ChangeYubiPin {
        profile: String,
        alias: String,
        old_pin: SecretString,
        new_pin: SecretString,
    },
    ChangeYubiPuk {
        profile: String,
        alias: String,
        old_puk: SecretString,
        new_puk: SecretString,
    },
    UnblockYubiPin {
        profile: String,
        alias: String,
        puk: SecretString,
        new_pin: SecretString,
    },
    ConfigureYubiRetries {
        profile: String,
        alias: String,
        pin: SecretString,
        puk: SecretString,
        pin_attempts: u8,
        puk_attempts: u8,
    },
    RotateYubiManagementKey {
        profile: String,
        alias: String,
        pin: SecretString,
    },
    ResumeYubiManagementKey {
        profile: String,
        alias: String,
        pin: Option<SecretString>,
    },
    RecoverYubiManagementKey {
        profile: String,
        yubi_alias: String,
        software_alias: String,
    },
    RecoverYubiSubkey {
        profile: String,
        alias: String,
        pin: SecretString,
    },
    RevokeYubiDevice {
        profile: String,
        yubi_alias: String,
        software_alias: String,
    },
    ListKv {
        profile: String,
        alias: String,
    },
    ListTeams {
        profile: String,
    },
    SyncTeam {
        profile: String,
        team_alias: String,
    },
    RunDueJobs {
        profile: String,
    },
}

impl std::fmt::Debug for Operation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ping => formatter.write_str("Ping"),
            Self::ListProfiles => formatter.write_str("ListProfiles"),
            Self::Probe { profile } => formatter
                .debug_struct("Probe")
                .field("profile", profile)
                .finish(),
            Self::ListAccounts { profile } => formatter
                .debug_struct("ListAccounts")
                .field("profile", profile)
                .finish(),
            Self::CreateAccount {
                profile,
                alias,
                username,
                device_name,
                email,
                invite: _,
                passphrase: _,
            } => formatter
                .debug_struct("CreateAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("username", username)
                .field("device_name", device_name)
                .field("email", email)
                .field("invite", &"<redacted>")
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::SetPassphrase { profile, alias, .. } => formatter
                .debug_struct("SetPassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::ChangePassphrase { profile, alias, .. } => formatter
                .debug_struct("ChangePassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::VerifyPassphrase { profile, alias, .. } => formatter
                .debug_struct("VerifyPassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::SyncAccount { profile, alias } => formatter
                .debug_struct("SyncAccount")
                .field("profile", profile)
                .field("alias", alias)
                .finish(),
            Self::ListYubiCards { profile } => formatter
                .debug_struct("ListYubiCards")
                .field("profile", profile)
                .finish(),
            Self::ListYubiAccounts { profile } => formatter
                .debug_struct("ListYubiAccounts")
                .field("profile", profile)
                .finish(),
            Self::CreateYubiAccount {
                profile,
                alias,
                username,
                device_name,
                email,
                invite: _,
                passphrase: _,
                card_serial,
                signing_slot,
                pq_slot,
                pin: _,
            } => formatter
                .debug_struct("CreateYubiAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("username", username)
                .field("device_name", device_name)
                .field("email", email)
                .field("invite", &"<redacted>")
                .field("passphrase", &"<redacted>")
                .field("card_serial", card_serial)
                .field("signing_slot", signing_slot)
                .field("pq_slot", pq_slot)
                .field("pin", &"<redacted>")
                .finish(),
            Self::ResumeYubiAccount {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("ResumeYubiAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::ProvisionYubiDevice {
                profile,
                source_alias,
                target_alias,
                device_name,
                serial,
                card_serial,
                signing_slot,
                pq_slot,
                pin: _,
            } => formatter
                .debug_struct("ProvisionYubiDevice")
                .field("profile", profile)
                .field("source_alias", source_alias)
                .field("target_alias", target_alias)
                .field("device_name", device_name)
                .field("serial", serial)
                .field("card_serial", card_serial)
                .field("signing_slot", signing_slot)
                .field("pq_slot", pq_slot)
                .field("pin", &"<redacted>")
                .finish(),
            Self::SyncYubiAccount {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("SyncYubiAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::YubiPinStatus { profile, alias } => formatter
                .debug_struct("YubiPinStatus")
                .field("profile", profile)
                .field("alias", alias)
                .finish(),
            Self::ChangeYubiPin {
                profile,
                alias,
                old_pin: _,
                new_pin: _,
            } => formatter
                .debug_struct("ChangeYubiPin")
                .field("profile", profile)
                .field("alias", alias)
                .field("old_pin", &"<redacted>")
                .field("new_pin", &"<redacted>")
                .finish(),
            Self::ChangeYubiPuk {
                profile,
                alias,
                old_puk: _,
                new_puk: _,
            } => formatter
                .debug_struct("ChangeYubiPuk")
                .field("profile", profile)
                .field("alias", alias)
                .field("old_puk", &"<redacted>")
                .field("new_puk", &"<redacted>")
                .finish(),
            Self::UnblockYubiPin {
                profile,
                alias,
                puk: _,
                new_pin: _,
            } => formatter
                .debug_struct("UnblockYubiPin")
                .field("profile", profile)
                .field("alias", alias)
                .field("puk", &"<redacted>")
                .field("new_pin", &"<redacted>")
                .finish(),
            Self::ConfigureYubiRetries {
                profile,
                alias,
                pin: _,
                puk: _,
                pin_attempts,
                puk_attempts,
            } => formatter
                .debug_struct("ConfigureYubiRetries")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .field("puk", &"<redacted>")
                .field("pin_attempts", pin_attempts)
                .field("puk_attempts", puk_attempts)
                .finish(),
            Self::RotateYubiManagementKey {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("RotateYubiManagementKey")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::ResumeYubiManagementKey {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("ResumeYubiManagementKey")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::RecoverYubiManagementKey {
                profile,
                yubi_alias,
                software_alias,
            } => formatter
                .debug_struct("RecoverYubiManagementKey")
                .field("profile", profile)
                .field("yubi_alias", yubi_alias)
                .field("software_alias", software_alias)
                .finish(),
            Self::RecoverYubiSubkey {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("RecoverYubiSubkey")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::RevokeYubiDevice {
                profile,
                yubi_alias,
                software_alias,
            } => formatter
                .debug_struct("RevokeYubiDevice")
                .field("profile", profile)
                .field("yubi_alias", yubi_alias)
                .field("software_alias", software_alias)
                .finish(),
            Self::ListKv { profile, alias } => formatter
                .debug_struct("ListKv")
                .field("profile", profile)
                .field("alias", alias)
                .finish(),
            Self::ListTeams { profile } => formatter
                .debug_struct("ListTeams")
                .field("profile", profile)
                .finish(),
            Self::SyncTeam {
                profile,
                team_alias,
            } => formatter
                .debug_struct("SyncTeam")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::RunDueJobs { profile } => formatter
                .debug_struct("RunDueJobs")
                .field("profile", profile)
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    #[serde(flatten)]
    pub result: ResponseResult,
}

impl Response {
    pub fn success(id: u64, value: serde_json::Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            result: ResponseResult::Success { value },
        }
    }

    pub fn error(id: u64, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            result: ResponseResult::Error {
                code,
                message: message.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ResponseResult {
    Success { value: serde_json::Value },
    Error { code: ErrorCode, message: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    InvalidRequest,
    VersionMismatch,
    Busy,
    DeadlineExceeded,
    OperationFailed,
}
