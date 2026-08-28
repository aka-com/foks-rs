//! Scriptable backend shell for the GPUI desktop's typed agent boundary.
//!
//! Keeping this as a second binary makes desktop operations testable without
//! initializing a native window or linking UI concerns into FOKS core crates.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::Parser as _;
use foks_agent_client::AgentClient;
use foks_agent_proto::{Operation, ResponseResult, SecretString};

#[derive(clap::Parser)]
#[command(name = "foks-desktop-backend")]
struct Arguments {
    #[arg(long)]
    agent_socket: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Status,
    Profiles,
    Accounts {
        profile: String,
    },
    CreateAccount {
        profile: String,
        alias: String,
        #[arg(long)]
        username: String,
        #[arg(long)]
        device_name: String,
        #[arg(long, default_value = "")]
        email: String,
        #[arg(long, default_value = "")]
        invite: String,
        #[arg(long, requires = "passphrase_confirmation_file")]
        passphrase_file: Option<PathBuf>,
        #[arg(long, requires = "passphrase_file")]
        passphrase_confirmation_file: Option<PathBuf>,
    },
    PassphraseSet(PassphraseChange),
    PassphraseChange(PassphraseChange),
    PassphraseVerify(PassphraseVerify),
    Sync {
        profile: String,
        alias: String,
    },
    Kv {
        profile: String,
        alias: String,
    },
    Teams {
        profile: String,
    },
    TeamSync {
        profile: String,
        team_alias: String,
    },
    RunJobs {
        profile: String,
    },
}

#[derive(clap::Args)]
struct PassphraseChange {
    profile: String,
    alias: String,
    #[arg(long)]
    passphrase_file: PathBuf,
    #[arg(long)]
    passphrase_confirmation_file: PathBuf,
}

#[derive(clap::Args)]
struct PassphraseVerify {
    profile: String,
    alias: String,
    #[arg(long)]
    passphrase_file: PathBuf,
}

fn main() {
    if let Err(error) = run(Arguments::parse()) {
        eprintln!("foks-desktop-backend: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), Box<dyn std::error::Error>> {
    let operation = match arguments.command {
        Command::Status => Operation::Ping,
        Command::Profiles => Operation::ListProfiles,
        Command::Accounts { profile } => Operation::ListAccounts { profile },
        Command::CreateAccount {
            profile,
            alias,
            username,
            device_name,
            email,
            invite,
            passphrase_file,
            passphrase_confirmation_file,
        } => Operation::CreateAccount {
            profile,
            alias,
            username,
            device_name,
            email,
            invite,
            passphrase: match (passphrase_file, passphrase_confirmation_file) {
                (Some(path), Some(confirmation)) => {
                    Some(read_confirmed_secret(&path, &confirmation)?)
                }
                (None, None) => None,
                _ => return Err("both passphrase files are required".into()),
            },
        },
        Command::PassphraseSet(arguments) => Operation::SetPassphrase {
            profile: arguments.profile,
            alias: arguments.alias,
            passphrase: read_confirmed_secret(
                &arguments.passphrase_file,
                &arguments.passphrase_confirmation_file,
            )?,
        },
        Command::PassphraseChange(arguments) => Operation::ChangePassphrase {
            profile: arguments.profile,
            alias: arguments.alias,
            passphrase: read_confirmed_secret(
                &arguments.passphrase_file,
                &arguments.passphrase_confirmation_file,
            )?,
        },
        Command::PassphraseVerify(arguments) => Operation::VerifyPassphrase {
            profile: arguments.profile,
            alias: arguments.alias,
            passphrase: read_secret(&arguments.passphrase_file)?,
        },
        Command::Sync { profile, alias } => Operation::SyncAccount { profile, alias },
        Command::Kv { profile, alias } => Operation::ListKv { profile, alias },
        Command::Teams { profile } => Operation::ListTeams { profile },
        Command::TeamSync {
            profile,
            team_alias,
        } => Operation::SyncTeam {
            profile,
            team_alias,
        },
        Command::RunJobs { profile } => Operation::RunDueJobs { profile },
    };
    let response = AgentClient::new(arguments.agent_socket).call(operation)?;
    match response.result {
        ResponseResult::Success { value } => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        ResponseResult::Error { code, message } => {
            Err(format!("agent returned {code:?}: {message}").into())
        }
    }
}

fn read_confirmed_secret(
    path: &std::path::Path,
    confirmation_path: &std::path::Path,
) -> Result<SecretString, Box<dyn std::error::Error>> {
    let passphrase = read_secret(path)?;
    let confirmation = read_secret(confirmation_path)?;
    if passphrase.expose() != confirmation.expose() {
        return Err("passphrase confirmation does not match".into());
    }
    Ok(passphrase)
}

fn read_secret(path: &std::path::Path) -> Result<SecretString, Box<dyn std::error::Error>> {
    use std::io::Read as _;
    use zeroize::Zeroizing;

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 1026 {
        return Err("passphrase file is not a bounded regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("passphrase file permissions allow group or other access".into());
        }
    }
    let mut value = Zeroizing::new(String::new());
    file.take(1027).read_to_string(&mut value)?;
    if value.ends_with("\r\n") {
        let length = value.len() - 2;
        value.truncate(length);
    } else if value.ends_with('\n') {
        let length = value.len() - 1;
        value.truncate(length);
    }
    if value.is_empty() || value.len() > 1024 || value.contains(['\0', '\r', '\n']) {
        return Err("passphrase must be one nonempty line of at most 1024 bytes".into());
    }
    Ok(SecretString::new(value.as_str()))
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    use super::*;

    #[test]
    fn backend_passphrase_files_are_private_bounded_and_not_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let private = directory.path().join("passphrase");
        std::fs::write(&private, "desktop passphrase\n").unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_secret(&private).unwrap().expose(),
            "desktop passphrase"
        );

        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_secret(&private).is_err());

        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("passphrase-link");
        symlink(&private, &link).unwrap();
        assert!(read_secret(&link).is_err());
    }
}
