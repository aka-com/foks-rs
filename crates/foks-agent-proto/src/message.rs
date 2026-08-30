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

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct YubiFederationUnlockInput {
    pub profile: String,
    pub alias: String,
    pub pin: SecretString,
}

impl std::fmt::Debug for YubiFederationUnlockInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("YubiFederationUnlockInput")
            .field("profile", &self.profile)
            .field("alias", &self.alias)
            .field("pin", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct YubiRetryConfiguration {
    pub puk: SecretString,
    pub pin_attempts: u8,
    pub puk_attempts: u8,
}

impl std::fmt::Debug for YubiRetryConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("YubiRetryConfiguration")
            .field("puk", &"<redacted>")
            .field("pin_attempts", &self.pin_attempts)
            .field("puk_attempts", &self.puk_attempts)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub operation: Operation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FederationRole {
    Member,
    Admin,
    Owner,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TeamRole {
    Member,
    Admin,
    Owner,
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
        invite: SecretString,
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
    StartDevicePairing {
        profile: String,
        account_alias: String,
    },
    RepublishDevicePairing {
        profile: String,
        account_alias: String,
    },
    FinishDevicePairing {
        profile: String,
        account_alias: String,
    },
    AcceptDevicePairing {
        profile: String,
        target_alias: String,
        device_name: String,
        serial: u64,
        phrase: SecretString,
    },
    ResumeDevicePairingAcceptance {
        profile: String,
        target_alias: String,
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
        retry_configuration: Option<YubiRetryConfiguration>,
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
        retry_configuration: Option<YubiRetryConfiguration>,
    },
    SyncYubiAccount {
        profile: String,
        alias: String,
        pin: SecretString,
        /// Also run every federated security responder this unlocked key can
        /// drive on the named profile.
        #[serde(default)]
        with_federation: bool,
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
    ListTeamMembers {
        profile: String,
        team_alias: String,
    },
    AddTeamMember {
        profile: String,
        team_alias: String,
        username: String,
        role: TeamRole,
        visibility: i16,
    },
    ResumeTeamMemberAddition {
        profile: String,
        team_alias: String,
        username: String,
    },
    DemoteTeamMember {
        profile: String,
        team_alias: String,
        username: String,
        role: TeamRole,
        visibility: i16,
    },
    RemoveTeamMember {
        profile: String,
        team_alias: String,
        username: String,
    },
    ResumeTeamMemberEdit {
        profile: String,
        team_alias: String,
    },
    AdmitFederatedTeam {
        local_profile: String,
        local_team_alias: String,
        remote_profile: String,
        remote_team_alias: String,
        role: FederationRole,
        visibility: i16,
    },
    ListFederatedTeams {
        profile: String,
        team_alias: String,
    },
    /// Runs the federated security responder for one local team. Either side
    /// may supply an already-enrolled Yubi alias plus its PIN; supplying both
    /// is the explicit two-key workflow that no unattended schedule performs.
    RefreshFederatedSecurity {
        profile: String,
        team_alias: String,
        #[serde(default)]
        local_yubi_alias: Option<String>,
        #[serde(default)]
        local_pin: Option<SecretString>,
        #[serde(default)]
        remote_profile: Option<String>,
        #[serde(default)]
        remote_yubi_alias: Option<String>,
        #[serde(default)]
        remote_pin: Option<SecretString>,
        /// Further hardware unlocks for the profiles a cascade reaches
        /// beyond the immediate pair.
        #[serde(default)]
        unlocks: Vec<YubiFederationUnlockInput>,
    },
    RunDueJobs {
        profile: String,
    },
}

impl Operation {
    /// Interactive KEX can contain two sequential relay polls. Frontends and
    /// the resident agent use a longer bounded deadline only for these calls.
    pub fn is_device_pairing_wait(&self) -> bool {
        matches!(
            self,
            Self::FinishDevicePairing { .. } | Self::AcceptDevicePairing { .. }
        )
    }
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
            Self::StartDevicePairing {
                profile,
                account_alias,
            } => formatter
                .debug_struct("StartDevicePairing")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .finish(),
            Self::RepublishDevicePairing {
                profile,
                account_alias,
            } => formatter
                .debug_struct("RepublishDevicePairing")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .finish(),
            Self::FinishDevicePairing {
                profile,
                account_alias,
            } => formatter
                .debug_struct("FinishDevicePairing")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .finish(),
            Self::AcceptDevicePairing {
                profile,
                target_alias,
                device_name,
                serial,
                phrase: _,
            } => formatter
                .debug_struct("AcceptDevicePairing")
                .field("profile", profile)
                .field("target_alias", target_alias)
                .field("device_name", device_name)
                .field("serial", serial)
                .field("phrase", &"<redacted>")
                .finish(),
            Self::ResumeDevicePairingAcceptance {
                profile,
                target_alias,
            } => formatter
                .debug_struct("ResumeDevicePairingAcceptance")
                .field("profile", profile)
                .field("target_alias", target_alias)
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
                retry_configuration,
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
                .field("retry_configuration", retry_configuration)
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
                retry_configuration,
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
                .field("retry_configuration", retry_configuration)
                .finish(),
            Self::SyncYubiAccount {
                profile,
                alias,
                pin: _,
                with_federation,
            } => formatter
                .debug_struct("SyncYubiAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .field("with_federation", with_federation)
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
            Self::ListTeamMembers {
                profile,
                team_alias,
            } => formatter
                .debug_struct("ListTeamMembers")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::AddTeamMember {
                profile,
                team_alias,
                username,
                role,
                visibility,
            } => formatter
                .debug_struct("AddTeamMember")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("username", username)
                .field("role", role)
                .field("visibility", visibility)
                .finish(),
            Self::ResumeTeamMemberAddition {
                profile,
                team_alias,
                username,
            } => formatter
                .debug_struct("ResumeTeamMemberAddition")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("username", username)
                .finish(),
            Self::DemoteTeamMember {
                profile,
                team_alias,
                username,
                role,
                visibility,
            } => formatter
                .debug_struct("DemoteTeamMember")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("username", username)
                .field("role", role)
                .field("visibility", visibility)
                .finish(),
            Self::RemoveTeamMember {
                profile,
                team_alias,
                username,
            } => formatter
                .debug_struct("RemoveTeamMember")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("username", username)
                .finish(),
            Self::ResumeTeamMemberEdit {
                profile,
                team_alias,
            } => formatter
                .debug_struct("ResumeTeamMemberEdit")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::AdmitFederatedTeam {
                local_profile,
                local_team_alias,
                remote_profile,
                remote_team_alias,
                role,
                visibility,
            } => formatter
                .debug_struct("AdmitFederatedTeam")
                .field("local_profile", local_profile)
                .field("local_team_alias", local_team_alias)
                .field("remote_profile", remote_profile)
                .field("remote_team_alias", remote_team_alias)
                .field("role", role)
                .field("visibility", visibility)
                .finish(),
            Self::ListFederatedTeams {
                profile,
                team_alias,
            } => formatter
                .debug_struct("ListFederatedTeams")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::RefreshFederatedSecurity {
                profile,
                team_alias,
                local_yubi_alias,
                local_pin: _,
                remote_profile,
                remote_yubi_alias,
                remote_pin: _,
                unlocks,
            } => formatter
                .debug_struct("RefreshFederatedSecurity")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("local_yubi_alias", local_yubi_alias)
                .field("local_pin", &"<redacted>")
                .field("remote_profile", remote_profile)
                .field("remote_yubi_alias", remote_yubi_alias)
                .field("remote_pin", &"<redacted>")
                .field("unlocks", unlocks)
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
