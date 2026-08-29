//! Testable screen and agent-operation model for the native FOKS desktop app.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::{fs, path::PathBuf};

use foks_agent_client::AgentClient;
use foks_agent_proto::{FederationRole, Operation, ResponseResult, SecretString};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Status,
    Profiles,
    Accounts,
    YubiKeys,
    PersonalKv,
    Teams,
    Jobs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassphraseAction {
    Set,
    Change,
    Verify,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YubiAction {
    ListCards,
    ResumeAccount,
    Sync,
    PinStatus,
    RotateManagementKey,
    ResumeManagementKey,
    RecoverManagementKey,
    RecoverSubkey,
    Revoke,
}

impl Screen {
    pub const ALL: [Self; 7] = [
        Self::Status,
        Self::Profiles,
        Self::Accounts,
        Self::YubiKeys,
        Self::PersonalKv,
        Self::Teams,
        Self::Jobs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Profiles => "Profiles",
            Self::Accounts => "Accounts",
            Self::YubiKeys => "YubiKeys",
            Self::PersonalKv => "Personal KV",
            Self::Teams => "Teams",
            Self::Jobs => "Scheduled work",
        }
    }
}

pub trait AgentTransport: Send + Sync + 'static {
    fn call(&self, operation: Operation) -> Result<Value, String>;
}

impl AgentTransport for AgentClient {
    fn call(&self, operation: Operation) -> Result<Value, String> {
        let response = self.call(operation).map_err(|error| error.to_string())?;
        match response.result {
            ResponseResult::Success { value } => Ok(value),
            ResponseResult::Error { code, message } => {
                Err(format!("agent returned {code:?}: {message}"))
            }
        }
    }
}

pub struct DesktopModel {
    transport: Arc<dyn AgentTransport>,
    screen: Screen,
    selected_profile: Option<String>,
    selected_account: Option<String>,
    value: Option<Value>,
    error: Option<String>,
}

impl DesktopModel {
    pub fn new(transport: Arc<dyn AgentTransport>) -> Self {
        Self {
            transport,
            screen: Screen::Status,
            selected_profile: None,
            selected_account: None,
            value: None,
            error: None,
        }
    }

    pub fn transport(&self) -> Arc<dyn AgentTransport> {
        Arc::clone(&self.transport)
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn selected_profile(&self) -> Option<&str> {
        self.selected_profile.as_deref()
    }

    pub fn selected_account(&self) -> Option<&str> {
        self.selected_account.as_deref()
    }

    pub fn value(&self) -> Option<&Value> {
        self.value.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn navigate(&mut self, screen: Screen) {
        self.screen = screen;
        self.value = None;
        self.error = None;
    }

    pub fn select_profile(&mut self, profile: impl Into<String>) {
        self.selected_profile = Some(profile.into());
        self.selected_account = None;
        self.value = None;
    }

    pub fn select_account(&mut self, account: impl Into<String>) {
        self.selected_account = Some(account.into());
        self.value = None;
    }

    pub fn operation(&self) -> Result<Operation, &'static str> {
        let profile = || {
            self.selected_profile
                .clone()
                .ok_or("select a profile first")
        };
        let account = || {
            self.selected_account
                .clone()
                .ok_or("select an account first")
        };
        match self.screen {
            Screen::Status => Ok(Operation::Ping),
            Screen::Profiles => Ok(Operation::ListProfiles),
            Screen::Accounts => Ok(Operation::ListAccounts {
                profile: profile()?,
            }),
            Screen::YubiKeys => Ok(Operation::ListYubiAccounts {
                profile: profile()?,
            }),
            Screen::PersonalKv => Ok(Operation::ListKv {
                profile: profile()?,
                alias: account()?,
            }),
            Screen::Teams => Ok(Operation::ListTeams {
                profile: profile()?,
            }),
            Screen::Jobs => Ok(Operation::RunDueJobs {
                profile: profile()?,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_account_operation(
        &self,
        alias: &str,
        username: &str,
        device_name: &str,
        email: &str,
        invite: &str,
        passphrase: Option<SecretString>,
        confirmation: Option<SecretString>,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if alias.trim().is_empty() || username.trim().is_empty() || device_name.trim().is_empty() {
            return Err("alias, username, and device name are required");
        }
        let passphrase = confirmed_optional_passphrase(passphrase, confirmation)?;
        Ok(Operation::CreateAccount {
            profile,
            alias: alias.to_owned(),
            username: username.to_owned(),
            device_name: device_name.to_owned(),
            email: email.to_owned(),
            invite: invite.to_owned(),
            passphrase,
        })
    }

    pub fn passphrase_operation(
        &self,
        action: PassphraseAction,
        passphrase: SecretString,
        confirmation: Option<SecretString>,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        let alias = self
            .selected_account
            .clone()
            .ok_or("select an account first")?;
        if passphrase.expose().is_empty() || passphrase.expose().len() > 1024 {
            return Err("passphrase must contain 1 to 1024 bytes");
        }
        match action {
            PassphraseAction::Set | PassphraseAction::Change => {
                let confirmation = confirmation.ok_or("confirm the passphrase")?;
                if passphrase.expose() != confirmation.expose() {
                    return Err("passphrase confirmation does not match");
                }
            }
            PassphraseAction::Verify if confirmation.is_some() => {
                return Err("verification does not take a confirmation");
            }
            PassphraseAction::Verify => {}
        }
        Ok(match action {
            PassphraseAction::Set => Operation::SetPassphrase {
                profile,
                alias,
                passphrase,
            },
            PassphraseAction::Change => Operation::ChangePassphrase {
                profile,
                alias,
                passphrase,
            },
            PassphraseAction::Verify => Operation::VerifyPassphrase {
                profile,
                alias,
                passphrase,
            },
        })
    }

    pub fn federation_admission_operation(
        &self,
        local_team_alias: &str,
        remote_profile: &str,
        remote_team_alias: &str,
        role: FederationRole,
        visibility: i16,
    ) -> Result<Operation, &'static str> {
        let local_profile = self
            .selected_profile
            .clone()
            .ok_or("select the local profile first")?;
        if local_team_alias.trim().is_empty()
            || remote_profile.trim().is_empty()
            || remote_team_alias.trim().is_empty()
            || remote_profile == local_profile
        {
            return Err("enter distinct profiles and both team aliases");
        }
        if role != FederationRole::Member && visibility != 0 {
            return Err("visibility applies only to member roles");
        }
        Ok(Operation::AdmitFederatedTeam {
            local_profile,
            local_team_alias: local_team_alias.to_owned(),
            remote_profile: remote_profile.to_owned(),
            remote_team_alias: remote_team_alias.to_owned(),
            role,
            visibility,
        })
    }

    pub fn federated_teams_operation(
        &self,
        local_team_alias: &str,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select the local profile first")?;
        if local_team_alias.trim().is_empty() {
            return Err("enter the local team alias");
        }
        Ok(Operation::ListFederatedTeams {
            profile,
            team_alias: local_team_alias.to_owned(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_yubi_account_operation(
        &self,
        alias: &str,
        username: &str,
        device_name: &str,
        email: &str,
        invite: SecretString,
        passphrase: Option<SecretString>,
        confirmation: Option<SecretString>,
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if alias.trim().is_empty() || username.trim().is_empty() || device_name.trim().is_empty() {
            return Err("alias, username, and device name are required");
        }
        validate_yubi_inputs(card_serial, signing_slot, pq_slot, pin.expose())?;
        Ok(Operation::CreateYubiAccount {
            profile,
            alias: alias.to_owned(),
            username: username.to_owned(),
            device_name: device_name.to_owned(),
            email: email.to_owned(),
            invite,
            passphrase: confirmed_optional_passphrase(passphrase, confirmation)?,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
        })
    }

    pub fn yubi_action_operation(
        &self,
        action: YubiAction,
        alias: &str,
        pin: Option<SecretString>,
        software_alias: Option<&str>,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        match action {
            YubiAction::ListCards => Ok(Operation::ListYubiCards { profile }),
            _ if alias.trim().is_empty() => Err("enter a YubiKey account alias"),
            YubiAction::ResumeAccount => Ok(Operation::ResumeYubiAccount {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
            }),
            YubiAction::PinStatus => Ok(Operation::YubiPinStatus {
                profile,
                alias: alias.to_owned(),
            }),
            YubiAction::Sync => Ok(Operation::SyncYubiAccount {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
            }),
            YubiAction::RotateManagementKey => Ok(Operation::RotateYubiManagementKey {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
            }),
            YubiAction::ResumeManagementKey => Ok(Operation::ResumeYubiManagementKey {
                profile,
                alias: alias.to_owned(),
                pin: Some(required_pin(pin)?),
            }),
            YubiAction::RecoverManagementKey => Ok(Operation::RecoverYubiManagementKey {
                profile,
                yubi_alias: alias.to_owned(),
                software_alias: software_alias
                    .filter(|alias| !alias.trim().is_empty())
                    .ok_or("enter a software account alias")?
                    .to_owned(),
            }),
            YubiAction::RecoverSubkey => Ok(Operation::RecoverYubiSubkey {
                profile,
                alias: alias.to_owned(),
                pin: required_pin(pin)?,
            }),
            YubiAction::Revoke => Ok(Operation::RevokeYubiDevice {
                profile,
                yubi_alias: alias.to_owned(),
                software_alias: software_alias
                    .filter(|alias| !alias.trim().is_empty())
                    .ok_or("enter a software account alias")?
                    .to_owned(),
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn provision_yubi_operation(
        &self,
        source_alias: &str,
        target_alias: &str,
        device_name: &str,
        serial: u64,
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
    ) -> Result<Operation, &'static str> {
        let profile = self
            .selected_profile
            .clone()
            .ok_or("select a profile first")?;
        if source_alias.trim().is_empty()
            || target_alias.trim().is_empty()
            || device_name.trim().is_empty()
            || serial == 0
        {
            return Err("source alias, target alias, device name, and serial are required");
        }
        validate_yubi_inputs(card_serial, signing_slot, pq_slot, pin.expose())?;
        Ok(Operation::ProvisionYubiDevice {
            profile,
            source_alias: source_alias.to_owned(),
            target_alias: target_alias.to_owned(),
            device_name: device_name.to_owned(),
            serial,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
        })
    }

    pub fn change_yubi_pin_operation(
        &self,
        alias: &str,
        old_pin: SecretString,
        new_pin: SecretString,
    ) -> Result<Operation, &'static str> {
        validate_pin(old_pin.expose())?;
        validate_pin(new_pin.expose())?;
        Ok(Operation::ChangeYubiPin {
            profile: self.selected_yubi_profile()?,
            alias: required_alias(alias)?,
            old_pin,
            new_pin,
        })
    }

    pub fn change_yubi_puk_operation(
        &self,
        alias: &str,
        old_puk: SecretString,
        new_puk: SecretString,
    ) -> Result<Operation, &'static str> {
        validate_pin(old_puk.expose())?;
        validate_pin(new_puk.expose())?;
        Ok(Operation::ChangeYubiPuk {
            profile: self.selected_yubi_profile()?,
            alias: required_alias(alias)?,
            old_puk,
            new_puk,
        })
    }

    pub fn unblock_yubi_pin_operation(
        &self,
        alias: &str,
        puk: SecretString,
        new_pin: SecretString,
    ) -> Result<Operation, &'static str> {
        validate_pin(puk.expose())?;
        validate_pin(new_pin.expose())?;
        Ok(Operation::UnblockYubiPin {
            profile: self.selected_yubi_profile()?,
            alias: required_alias(alias)?,
            puk,
            new_pin,
        })
    }

    pub fn configure_yubi_retries_operation(
        &self,
        alias: &str,
        pin: SecretString,
        puk: SecretString,
        pin_attempts: u8,
        puk_attempts: u8,
    ) -> Result<Operation, &'static str> {
        validate_pin(pin.expose())?;
        validate_pin(puk.expose())?;
        if !(1..=15).contains(&pin_attempts) || !(1..=15).contains(&puk_attempts) {
            return Err("PIN and PUK retries must each be from 1 through 15");
        }
        Ok(Operation::ConfigureYubiRetries {
            profile: self.selected_yubi_profile()?,
            alias: required_alias(alias)?,
            pin,
            puk,
            pin_attempts,
            puk_attempts,
        })
    }

    fn selected_yubi_profile(&self) -> Result<String, &'static str> {
        self.selected_profile
            .clone()
            .ok_or("select a profile first")
    }

    pub fn accept(&mut self, result: Result<Value, String>) {
        match result {
            Ok(value) => {
                if self.screen == Screen::Profiles && self.selected_profile.is_none() {
                    self.selected_profile = value
                        .as_array()
                        .and_then(|profiles| profiles.first())
                        .and_then(|profile| profile.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                if self.screen == Screen::Accounts && self.selected_account.is_none() {
                    self.selected_account = value
                        .as_array()
                        .and_then(|accounts| accounts.first())
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                self.value = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.value = None;
                self.error = Some(error);
            }
        }
    }
}

fn validate_yubi_inputs(
    card_serial: u32,
    signing_slot: u8,
    pq_slot: u8,
    pin: &str,
) -> Result<(), &'static str> {
    if card_serial == 0 {
        return Err("select a nonzero YubiKey serial");
    }
    if !(0x82..=0x95).contains(&signing_slot)
        || !(0x82..=0x95).contains(&pq_slot)
        || signing_slot == pq_slot
    {
        return Err("use two distinct PIV retired-key slots from 0x82 through 0x95");
    }
    if !(6..=8).contains(&pin.len()) || !pin.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("PIN must contain six to eight printable ASCII characters");
    }
    Ok(())
}

fn required_pin(pin: Option<SecretString>) -> Result<SecretString, &'static str> {
    let pin = pin.ok_or("enter the YubiKey PIN")?;
    validate_pin(pin.expose())?;
    Ok(pin)
}

fn validate_pin(pin: &str) -> Result<(), &'static str> {
    if !(6..=8).contains(&pin.len()) || !pin.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("PIN or PUK must contain six to eight printable ASCII characters");
    }
    Ok(())
}

fn required_alias(alias: &str) -> Result<String, &'static str> {
    if alias.trim().is_empty() {
        Err("enter a YubiKey account alias")
    } else {
        Ok(alias.to_owned())
    }
}

fn confirmed_optional_passphrase(
    passphrase: Option<SecretString>,
    confirmation: Option<SecretString>,
) -> Result<Option<SecretString>, &'static str> {
    match (passphrase, confirmation) {
        (None, None) => Ok(None),
        (Some(passphrase), Some(confirmation))
            if passphrase.expose().is_empty() && confirmation.expose().is_empty() =>
        {
            Ok(None)
        }
        (Some(passphrase), Some(confirmation)) => {
            if passphrase.expose().len() > 1024 {
                return Err("passphrase exceeds 1024 bytes");
            }
            if passphrase.expose() != confirmation.expose() {
                return Err("passphrase confirmation does not match");
            }
            Ok(Some(passphrase))
        }
        _ => Err("provide and confirm the signup passphrase"),
    }
}

/// Installs a local-only crash marker hook. Reports contain no panic payload,
/// paths, protocol values, or automatic upload mechanism.
pub fn install_crash_reporter(directory: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = write_crash_marker(&directory);
        previous(info);
    }));
}

fn write_crash_marker(directory: &std::path::Path) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
        if !directory.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(directory)?;
        }
        let metadata = fs::symlink_metadata(directory)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "crash directory is not private",
            ));
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = directory.join(format!("crash-{timestamp}-{}.txt", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        writeln!(file, "foks-desktop-version={}", env!("CARGO_PKG_VERSION"))?;
        writeln!(file, "timestamp={timestamp}")?;
        writeln!(file, "platform={}", std::env::consts::OS)?;
        file.sync_all()?;
        Ok(path)
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "crash markers are unsupported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct MockTransport {
        operations: Mutex<Vec<Operation>>,
    }

    impl AgentTransport for MockTransport {
        fn call(&self, operation: Operation) -> Result<Value, String> {
            self.operations.lock().unwrap().push(operation);
            Ok(Value::Null)
        }
    }

    #[test]
    fn screens_cannot_cross_profile_or_account_selection() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.navigate(Screen::PersonalKv);
        assert_eq!(model.operation(), Err("select a profile first"));
        model.select_profile("hosted");
        assert_eq!(model.operation(), Err("select an account first"));
        model.select_account("personal");
        assert_eq!(
            model.operation().unwrap(),
            Operation::ListKv {
                profile: "hosted".to_owned(),
                alias: "personal".to_owned(),
            }
        );
    }

    #[test]
    fn profile_and_account_lists_choose_only_explicit_response_values() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.navigate(Screen::Profiles);
        model.accept(Ok(serde_json::json!([{"name": "local"}])));
        assert_eq!(model.selected_profile(), Some("local"));
        model.navigate(Screen::Accounts);
        model.accept(Ok(serde_json::json!(["personal"])));
        assert_eq!(model.selected_account(), Some("personal"));
    }

    #[test]
    fn account_creation_is_bound_to_the_selected_profile_and_invite() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        assert_eq!(
            model.create_account_operation("personal", "rae", "laptop", "", "invite", None, None,),
            Err("select a profile first")
        );
        model.select_profile("local");
        assert_eq!(
            model
                .create_account_operation(
                    "personal",
                    "rae",
                    "laptop",
                    "rae@example.test",
                    "small-team+launch",
                    None,
                    None,
                )
                .unwrap(),
            Operation::CreateAccount {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                username: "rae".to_owned(),
                device_name: "laptop".to_owned(),
                email: "rae@example.test".to_owned(),
                invite: "small-team+launch".to_owned(),
                passphrase: None,
            }
        );
    }

    #[test]
    fn account_and_security_forms_bind_passphrase_operations_to_the_selection() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.select_profile("local");

        assert_eq!(
            model
                .create_account_operation(
                    "personal",
                    "rae",
                    "laptop",
                    "",
                    "s.invite",
                    Some(SecretString::new("signup passphrase")),
                    Some(SecretString::new("signup passphrase")),
                )
                .unwrap(),
            Operation::CreateAccount {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                username: "rae".to_owned(),
                device_name: "laptop".to_owned(),
                email: String::new(),
                invite: "s.invite".to_owned(),
                passphrase: Some(SecretString::new("signup passphrase")),
            }
        );
        assert_eq!(
            model.create_account_operation(
                "personal",
                "rae",
                "laptop",
                "",
                "",
                Some(SecretString::new("one")),
                Some(SecretString::new("two")),
            ),
            Err("passphrase confirmation does not match")
        );

        model.select_account("personal");
        assert_eq!(
            model
                .passphrase_operation(
                    PassphraseAction::Set,
                    SecretString::new("first passphrase"),
                    Some(SecretString::new("first passphrase")),
                )
                .unwrap(),
            Operation::SetPassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("first passphrase"),
            }
        );
        assert_eq!(
            model
                .passphrase_operation(
                    PassphraseAction::Change,
                    SecretString::new("second passphrase"),
                    Some(SecretString::new("second passphrase")),
                )
                .unwrap(),
            Operation::ChangePassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("second passphrase"),
            }
        );
        assert_eq!(
            model
                .passphrase_operation(
                    PassphraseAction::Verify,
                    SecretString::new("second passphrase"),
                    None,
                )
                .unwrap(),
            Operation::VerifyPassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("second passphrase"),
            }
        );
        assert_eq!(
            model.passphrase_operation(
                PassphraseAction::Verify,
                SecretString::new("second passphrase"),
                Some(SecretString::new("second passphrase")),
            ),
            Err("verification does not take a confirmation")
        );
    }

    #[test]
    fn federation_form_binds_both_profiles_and_validates_role_visibility() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        assert_eq!(
            model.federation_admission_operation(
                "engineering",
                "partner",
                "security",
                FederationRole::Member,
                0,
            ),
            Err("select the local profile first")
        );
        model.select_profile("local");
        assert_eq!(
            model
                .federation_admission_operation(
                    "engineering",
                    "partner",
                    "security",
                    FederationRole::Member,
                    4,
                )
                .unwrap(),
            Operation::AdmitFederatedTeam {
                local_profile: "local".to_owned(),
                local_team_alias: "engineering".to_owned(),
                remote_profile: "partner".to_owned(),
                remote_team_alias: "security".to_owned(),
                role: FederationRole::Member,
                visibility: 4,
            }
        );
        assert_eq!(
            model.federation_admission_operation(
                "engineering",
                "local",
                "security",
                FederationRole::Member,
                0,
            ),
            Err("enter distinct profiles and both team aliases")
        );
        assert_eq!(
            model.federation_admission_operation(
                "engineering",
                "partner",
                "security",
                FederationRole::Admin,
                1,
            ),
            Err("visibility applies only to member roles")
        );
        assert_eq!(
            model.federated_teams_operation("engineering").unwrap(),
            Operation::ListFederatedTeams {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            }
        );
    }

    #[test]
    fn yubikey_screen_binds_hardware_and_recovery_operations_to_the_profile() {
        let transport = Arc::new(MockTransport {
            operations: Mutex::new(Vec::new()),
        });
        let mut model = DesktopModel::new(transport);
        model.select_profile("local");
        model.navigate(Screen::YubiKeys);
        assert_eq!(
            model.operation().unwrap(),
            Operation::ListYubiAccounts {
                profile: "local".to_owned(),
            }
        );
        assert_eq!(
            model.yubi_action_operation(YubiAction::ListCards, "", None, None),
            Ok(Operation::ListYubiCards {
                profile: "local".to_owned(),
            })
        );
        assert_eq!(
            model
                .create_yubi_account_operation(
                    "hardware",
                    "rae",
                    "primary key",
                    "",
                    SecretString::new("s.invite"),
                    None,
                    None,
                    42,
                    0x82,
                    0x83,
                    SecretString::new("123456"),
                )
                .unwrap(),
            Operation::CreateYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                username: "rae".to_owned(),
                device_name: "primary key".to_owned(),
                email: String::new(),
                invite: SecretString::new("s.invite"),
                passphrase: None,
                card_serial: 42,
                signing_slot: 0x82,
                pq_slot: 0x83,
                pin: SecretString::new("123456"),
            }
        );
        assert_eq!(
            model.yubi_action_operation(
                YubiAction::RecoverManagementKey,
                "hardware",
                None,
                Some("personal"),
            ),
            Ok(Operation::RecoverYubiManagementKey {
                profile: "local".to_owned(),
                yubi_alias: "hardware".to_owned(),
                software_alias: "personal".to_owned(),
            })
        );
        assert_eq!(
            model.yubi_action_operation(YubiAction::Sync, "hardware", None, None),
            Err("enter the YubiKey PIN")
        );
        assert_eq!(
            model.yubi_action_operation(
                YubiAction::ResumeAccount,
                "hardware",
                Some(SecretString::new("123456")),
                None,
            ),
            Ok(Operation::ResumeYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("123456"),
            })
        );
        assert_eq!(
            model.yubi_action_operation(
                YubiAction::ResumeManagementKey,
                "hardware",
                Some(SecretString::new("123456")),
                None,
            ),
            Ok(Operation::ResumeYubiManagementKey {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: Some(SecretString::new("123456")),
            })
        );
        assert_eq!(
            model.create_yubi_account_operation(
                "hardware",
                "rae",
                "primary key",
                "",
                SecretString::new(""),
                None,
                None,
                42,
                0x82,
                0x82,
                SecretString::new("123456"),
            ),
            Err("use two distinct PIV retired-key slots from 0x82 through 0x95")
        );
    }

    #[test]
    fn crash_markers_are_private_and_exclude_failure_payloads() {
        let temporary = tempfile::tempdir().unwrap();
        let marker = write_crash_marker(&temporary.path().join("crashes")).unwrap();
        let contents = fs::read_to_string(marker).unwrap();
        assert!(contents.contains("foks-desktop-version="));
        assert!(!contents.contains("panic"));
        assert!(!contents.contains(temporary.path().to_string_lossy().as_ref()));
    }
}
