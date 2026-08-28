//! Testable screen and agent-operation model for the native FOKS desktop app.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::{fs, path::PathBuf};

use foks_agent_client::AgentClient;
use foks_agent_proto::{Operation, ResponseResult, SecretString};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Status,
    Profiles,
    Accounts,
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

impl Screen {
    pub const ALL: [Self; 6] = [
        Self::Status,
        Self::Profiles,
        Self::Accounts,
        Self::PersonalKv,
        Self::Teams,
        Self::Jobs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Profiles => "Profiles",
            Self::Accounts => "Accounts",
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
    fn crash_markers_are_private_and_exclude_failure_payloads() {
        let temporary = tempfile::tempdir().unwrap();
        let marker = write_crash_marker(&temporary.path().join("crashes")).unwrap();
        let contents = fs::read_to_string(marker).unwrap();
        assert!(contents.contains("foks-desktop-version="));
        assert!(!contents.contains("panic"));
        assert!(!contents.contains(temporary.path().to_string_lossy().as_ref()));
    }
}
