//! Explicit state maintenance; secret values never enter command arguments or JSON.
use foks_client_app::portability;
use std::{
    collections::BTreeMap,
    io::{self, Write as _},
    path::{Path, PathBuf},
};
#[derive(clap::Subcommand)]
pub enum StateCommand {
    Inspect,
    Status,
    ImportStatus,
    Relocate {
        #[arg(long)]
        destination: PathBuf,
    },
    Recover,
    RecoverRelocation,
    /// Print the exact preview and request confirmation before exporting.
    Export {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        key_output: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    Import {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
    },
    /// Verify every imported account without submitting queued operations.
    VerifyOnline {
        #[arg(long)]
        profile: Option<String>,
        /// Repeat PROFILE/ACCOUNT=PRIVATE_FILE for selected bot credentials.
        #[arg(long)]
        bot_token_file: Vec<String>,
        /// Repeat PROFILE/ACCOUNT=PRIVATE_FILE for enrolled YubiKey PINs.
        #[arg(long)]
        pin_file: Vec<String>,
    },
}
fn secret_paths(
    values: Vec<String>,
) -> Result<BTreeMap<String, PathBuf>, Box<dyn std::error::Error>> {
    let mut paths = BTreeMap::new();
    for value in values {
        let (scope, path) = value
            .split_once('=')
            .ok_or("expected PROFILE/ACCOUNT=PRIVATE_FILE")?;
        let (profile, account) = scope
            .split_once('/')
            .ok_or("expected PROFILE/ACCOUNT=PRIVATE_FILE")?;
        if profile.is_empty()
            || account.is_empty()
            || path.is_empty()
            || paths.insert(scope.into(), path.into()).is_some()
        {
            return Err("invalid or duplicate secret file scope".into());
        }
    }
    Ok(paths)
}
pub fn run(root: &Path, command: StateCommand) -> Result<(), Box<dyn std::error::Error>> {
    let report = match command {
        StateCommand::Inspect => serde_json::to_value(portability::inspect_native_state(root)?)?,
        StateCommand::Status => serde_json::to_value(portability::maintenance_status(root)?)?,
        StateCommand::ImportStatus => serde_json::to_value(portability::import_status(root)?)?,
        StateCommand::Relocate { destination } => {
            serde_json::to_value(portability::relocate_state(root, destination)?)?
        }
        StateCommand::Recover => portability::recover_state(root)?,
        StateCommand::RecoverRelocation => {
            serde_json::to_value(portability::recover_relocation(root)?)?
        }
        StateCommand::Import { input, key_file } => serde_json::to_value(
            portability::import_state(input, &portability::read_transfer_key(key_file)?, root)?,
        )?,
        StateCommand::Export {
            output,
            key_output,
            yes,
        } => {
            let preview = portability::prepare_state_export(root)?;
            let report = preview.report()?;
            eprintln!("{}", serde_json::to_string_pretty(&report)?);
            eprintln!("This archive copies existing device credentials. Copies share the same server-side revocation identity. Provision a new device for independent revocation.");
            if !yes {
                eprint!("Export this exact snapshot? Type export to continue: ");
                io::stderr().flush()?;
                let mut line = String::new();
                io::stdin().read_line(&mut line)?;
                if line.trim() != "export" {
                    return Err("export cancelled".into());
                }
            }
            serde_json::to_value(
                preview
                    .authorize(&report.digest)?
                    .export_with_key_file(output, key_output)?,
            )?
        }
        StateCommand::VerifyOnline {
            profile,
            bot_token_file,
            pin_file,
        } => {
            let bots = secret_paths(bot_token_file)?;
            let pins = secret_paths(pin_file)?;
            let registry = foks_client_app::ProfileRegistry::open(root)?;
            let credentials = foks_client_app::ClientCredentials::open(root)?;
            if let Some(name) = &profile {
                registry.profile(name)?;
            }
            let provider = foks_yubi::HardwareYubiProvider::new();
            let mut reports = Vec::new();
            for configured in registry
                .profiles()
                .filter(|p| profile.as_ref().is_none_or(|n| n == &p.name))
            {
                let session = foks_client_app::ProfileSession::open(&registry, &configured.name)?;
                if !credentials.requires_import_verification(&session)? {
                    continue;
                }
                let mut loaded_bots = BTreeMap::new();
                let mut loaded_pins = BTreeMap::new();
                let prefix = format!("{}/", configured.name);
                for (scope, path) in &bots {
                    if let Some(alias) = scope.strip_prefix(&prefix) {
                        loaded_bots.insert(
                            alias.to_owned(),
                            portability::read_private_secret_file(path, 64)?,
                        );
                    }
                }
                for (scope, path) in &pins {
                    if let Some(alias) = scope.strip_prefix(&prefix) {
                        loaded_pins.insert(
                            alias.to_owned(),
                            foks_yubi::Pin::new(
                                portability::read_private_secret_file(path, 128)?.as_str(),
                            )?,
                        );
                    }
                }
                let mut secrets = BTreeMap::new();
                for (alias, token) in &loaded_bots {
                    secrets.insert(
                        alias.clone(),
                        portability::VerificationSecret::BotToken(token),
                    );
                }
                for (alias, pin) in &loaded_pins {
                    if secrets
                        .insert(
                            alias.clone(),
                            portability::VerificationSecret::Yubi {
                                pin,
                                provider: &provider,
                            },
                        )
                        .is_some()
                    {
                        return Err("account has conflicting verification inputs".into());
                    }
                }
                reports.push(portability::verify_imported_profile(
                    &credentials,
                    &session,
                    &secrets,
                )?);
            }
            let failed = reports.iter().any(|r| !r.verified);
            let value = serde_json::to_value(&reports)?;
            if failed {
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Err("some imported accounts remain blocked; supply required credentials or reauthenticate and retry".into());
            }
            value
        }
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
