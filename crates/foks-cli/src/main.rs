#![forbid(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use clap::{Parser as _, ValueEnum};
use foks_client_app::{
    derive_vault_key, AccountVault, CheckedProfileSession, ClientCredentials, CredentialBackend,
    Passphrase, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
    YubiProvisionInput, YubiSignupInput,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_yubi::{CardId, HardwareYubiProvider, Pin, SlotId, YubiProvider as _};
use zeroize::Zeroizing;

#[derive(clap::Parser)]
#[command(
    name = "foks-rs",
    about = "Standalone native client for the Rust FOKS implementation"
)]
struct Arguments {
    /// Explicit application-state root. No AKA or user data path is inferred.
    #[arg(long)]
    state_dir: PathBuf,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Initializes an independent FOKS client state root.
    Init(InitArguments),
    #[command(subcommand)]
    Profile(ProfileCommand),
    #[command(subcommand)]
    Account(AccountCommand),
    #[command(subcommand)]
    Kv(KvCommand),
    #[command(subcommand)]
    Jobs(JobCommand),
    #[command(subcommand)]
    Device(DeviceCommand),
    #[command(subcommand)]
    Recovery(RecoveryCommand),
    #[command(subcommand)]
    Passphrase(PassphraseCommand),
    #[command(subcommand)]
    Team(TeamCommand),
    #[command(subcommand)]
    Yubi(YubiCommand),
}

#[derive(clap::Args)]
struct InitArguments {
    /// Credential protection. Native is Keychain on macOS and Secret Service on Linux.
    #[arg(long, value_enum, default_value_t = KeyBackendArgument::Native)]
    key_backend: KeyBackendArgument,
}

#[derive(Clone, Copy, ValueEnum)]
enum KeyBackendArgument {
    Native,
    PrivateFile,
}

impl From<KeyBackendArgument> for CredentialBackend {
    fn from(value: KeyBackendArgument) -> Self {
        match value {
            KeyBackendArgument::Native => Self::Native,
            KeyBackendArgument::PrivateFile => Self::PrivateFile,
        }
    }
}

#[derive(clap::Subcommand)]
enum ProfileCommand {
    List,
    Add(ProfileAdd),
    Remove {
        name: String,
    },
    Probe {
        name: String,
    },
    /// Destructively discards a profile's rollback checkpoint and hard-state database.
    ResetHardState {
        name: String,
        #[arg(long)]
        confirm_delete: bool,
    },
    /// Applies a signed compatibility lease or a fail-closed drift revocation.
    ApplyCanary {
        name: String,
        #[arg(long)]
        artifact: PathBuf,
    },
}

#[derive(clap::Args)]
struct ProfileAdd {
    name: String,
    target: String,
    #[arg(long, value_enum, default_value_t = ProfileGeneration::V019)]
    generation: ProfileGeneration,
    #[arg(long)]
    ca_der: Option<PathBuf>,
    /// Ed25519 public key (hex) authorized to sign hosted canary leases.
    #[arg(long)]
    canary_public_key: Option<String>,
    /// Stable HTTPS URL serving the latest signed hosted canary lease.
    #[arg(long)]
    canary_url: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProfileGeneration {
    V019,
    CurrentProbeOnly,
}

#[derive(clap::Subcommand)]
enum AccountCommand {
    List { profile: String },
    Create(AccountCreate),
    Resume { profile: String, alias: String },
    Sync { profile: String, alias: String },
}

#[derive(clap::Subcommand)]
enum KvCommand {
    List {
        profile: String,
        alias: String,
    },
    Get {
        profile: String,
        alias: String,
        path: String,
        #[arg(long)]
        output: PathBuf,
    },
    Put {
        profile: String,
        alias: String,
        path: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    Mkdir {
        profile: String,
        alias: String,
        path: String,
    },
    Remove {
        profile: String,
        alias: String,
        path: String,
        #[arg(long)]
        recursive: bool,
    },
}

#[derive(clap::Subcommand)]
enum JobCommand {
    ScheduleUserRefresh {
        profile: String,
        alias: String,
        #[arg(long, default_value_t = 900)]
        interval_seconds: u64,
    },
    RunDue {
        profile: String,
    },
}

#[derive(clap::Subcommand)]
enum DeviceCommand {
    List {
        profile: String,
        alias: String,
    },
    ProvisionOwner {
        profile: String,
        source_alias: String,
        target_alias: String,
        #[arg(long)]
        device_name: String,
        #[arg(long, default_value_t = 1)]
        serial: u64,
    },
    ResumeProvision {
        profile: String,
        target_alias: String,
    },
}

#[derive(clap::Subcommand)]
enum RecoveryCommand {
    Enroll {
        profile: String,
        account_alias: String,
        backup_alias: String,
        #[arg(long)]
        output: PathBuf,
    },
    Recover {
        profile: String,
        target_alias: String,
        #[arg(long)]
        phrase_file: PathBuf,
        #[arg(long)]
        device_name: String,
        #[arg(long, default_value_t = 1)]
        serial: u64,
    },
    Resume {
        profile: String,
        target_alias: String,
        #[arg(long)]
        phrase_file: PathBuf,
        #[arg(long)]
        device_name: String,
    },
}

#[derive(clap::Subcommand)]
enum TeamCommand {
    List {
        profile: String,
    },
    CreateNamed {
        profile: String,
        account_alias: String,
        team_alias: String,
        #[arg(long)]
        name: String,
    },
    CreateAdhoc {
        profile: String,
        account_alias: String,
        team_alias: String,
    },
    ResumeCreate {
        profile: String,
        team_alias: String,
    },
    Sync {
        profile: String,
        team_alias: String,
    },
}

#[derive(clap::Args)]
struct AccountCreate {
    profile: String,
    alias: String,
    #[arg(long)]
    username: String,
    #[arg(long)]
    device_name: String,
    #[arg(long, default_value = "")]
    email: String,
    /// Signup invite. Standard codes start with `s.`; other values are multi-use codes.
    #[arg(long, default_value = "")]
    invite: String,
    /// Private UTF-8 file containing the passphrase (one trailing newline is ignored).
    #[arg(long, requires = "passphrase_confirmation_file")]
    passphrase_file: Option<PathBuf>,
    /// A second private file that must contain the same passphrase.
    #[arg(long, requires = "passphrase_file")]
    passphrase_confirmation_file: Option<PathBuf>,
}

#[derive(clap::Subcommand)]
enum PassphraseCommand {
    Set(PassphraseChange),
    Change(PassphraseChange),
    Verify(PassphraseVerify),
}

#[derive(clap::Subcommand)]
enum YubiCommand {
    List {
        profile: String,
    },
    Cards {
        profile: String,
    },
    Create(YubiCreate),
    ResumeAccount {
        profile: String,
        alias: String,
        #[arg(long)]
        pin_file: PathBuf,
    },
    Provision(YubiProvision),
    Sync {
        profile: String,
        alias: String,
        #[arg(long)]
        pin_file: PathBuf,
    },
    PinStatus {
        profile: String,
        alias: String,
    },
    ChangePin {
        profile: String,
        alias: String,
        #[arg(long)]
        old_pin_file: PathBuf,
        #[arg(long)]
        new_pin_file: PathBuf,
    },
    ChangePuk {
        profile: String,
        alias: String,
        #[arg(long)]
        old_puk_file: PathBuf,
        #[arg(long)]
        new_puk_file: PathBuf,
    },
    UnblockPin {
        profile: String,
        alias: String,
        #[arg(long)]
        puk_file: PathBuf,
        #[arg(long)]
        new_pin_file: PathBuf,
    },
    ConfigureRetries {
        profile: String,
        alias: String,
        /// Current PIN to verify and preserve after PIV resets it.
        #[arg(long)]
        pin_file: PathBuf,
        /// PUK to restore immediately after PIV resets it.
        #[arg(long)]
        puk_file: PathBuf,
        #[arg(long)]
        pin_attempts: u8,
        #[arg(long)]
        puk_attempts: u8,
    },
    RotateManagementKey {
        profile: String,
        alias: String,
        #[arg(long)]
        pin_file: PathBuf,
    },
    ResumeManagementKey {
        profile: String,
        alias: String,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    RecoverManagementKey {
        profile: String,
        yubi_alias: String,
        software_alias: String,
    },
    RecoverSubkey {
        profile: String,
        alias: String,
        #[arg(long)]
        pin_file: PathBuf,
    },
    Revoke {
        profile: String,
        yubi_alias: String,
        software_alias: String,
    },
}

#[derive(clap::Args)]
struct YubiCreate {
    profile: String,
    alias: String,
    #[arg(long)]
    username: String,
    #[arg(long)]
    device_name: String,
    #[arg(long, default_value = "")]
    email: String,
    /// Private one-line file containing a signup invite.
    #[arg(long)]
    invite_file: Option<PathBuf>,
    #[arg(long)]
    card_serial: u32,
    #[arg(long, default_value = "0x82", value_parser = parse_slot)]
    signing_slot: u8,
    #[arg(long, default_value = "0x83", value_parser = parse_slot)]
    pq_slot: u8,
    #[arg(long)]
    pin_file: PathBuf,
    #[arg(long, requires = "passphrase_confirmation_file")]
    passphrase_file: Option<PathBuf>,
    #[arg(long, requires = "passphrase_file")]
    passphrase_confirmation_file: Option<PathBuf>,
}

#[derive(clap::Args)]
struct YubiProvision {
    profile: String,
    source_alias: String,
    target_alias: String,
    #[arg(long)]
    device_name: String,
    #[arg(long, default_value_t = 1)]
    serial: u64,
    #[arg(long)]
    card_serial: u32,
    #[arg(long, default_value = "0x82", value_parser = parse_slot)]
    signing_slot: u8,
    #[arg(long, default_value = "0x83", value_parser = parse_slot)]
    pq_slot: u8,
    #[arg(long)]
    pin_file: PathBuf,
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
        eprintln!("foks-rs: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), Box<dyn std::error::Error>> {
    match arguments.command {
        Command::Init(init) => initialize(
            &arguments.state_dir,
            arguments.json,
            init.key_backend.into(),
        ),
        Command::Profile(command) => profile_command(&arguments.state_dir, arguments.json, command),
        Command::Account(command) => account_command(&arguments.state_dir, arguments.json, command),
        Command::Kv(command) => kv_command(&arguments.state_dir, arguments.json, command),
        Command::Jobs(command) => job_command(&arguments.state_dir, arguments.json, command),
        Command::Device(command) => device_command(&arguments.state_dir, arguments.json, command),
        Command::Recovery(command) => {
            recovery_command(&arguments.state_dir, arguments.json, command)
        }
        Command::Passphrase(command) => {
            passphrase_command(&arguments.state_dir, arguments.json, command)
        }
        Command::Team(command) => team_command(&arguments.state_dir, arguments.json, command),
        Command::Yubi(command) => yubi_command(&arguments.state_dir, arguments.json, command),
    }
}

fn initialize(
    state_dir: &Path,
    json: bool,
    backend: CredentialBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    ClientCredentials::initialize(state_dir, backend)?;
    ProfileRegistry::open(state_dir)?;
    output(
        json,
        &serde_json::json!({
            "state_dir": state_dir.canonicalize()?,
            "key_backend": match backend {
                CredentialBackend::Native => "native",
                CredentialBackend::PrivateFile => "private-file",
            },
            "external_rollback_checkpoint": backend == CredentialBackend::Native,
        }),
        "initialized standalone FOKS state",
    )
}

fn profile_command(
    state_dir: &Path,
    json: bool,
    command: ProfileCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut registry = ProfileRegistry::open(state_dir)?;
    match command {
        ProfileCommand::List => {
            let profiles = registry.profiles().cloned().collect::<Vec<_>>();
            output(json, &profiles, &format!("{} profile(s)", profiles.len()))
        }
        ProfileCommand::Add(arguments) => {
            let protocol = match arguments.generation {
                ProfileGeneration::V019 => {
                    if arguments.canary_public_key.is_some() || arguments.canary_url.is_some() {
                        return Err("v0.1.9 profiles do not take canary configuration".into());
                    }
                    ProtocolPolicy::V019
                }
                ProfileGeneration::CurrentProbeOnly => ProtocolPolicy::CurrentProbeOnly {
                    canary_public_key: arguments
                        .canary_public_key
                        .ok_or("current profiles require --canary-public-key")?,
                    lease_url: arguments
                        .canary_url
                        .ok_or("current profiles require --canary-url")?,
                    last_artifact: None,
                },
            };
            let trust = match arguments.ca_der {
                Some(path) => TrustRoot::CertificateDer { path },
                None => TrustRoot::WebPki,
            };
            let profile = Profile {
                name: arguments.name,
                probe: arguments.target,
                protocol,
                trust,
            };
            registry.add(profile.clone())?;
            output(json, &profile, "profile added")
        }
        ProfileCommand::Remove { name } => {
            let removed = registry.remove(&name)?;
            output(
                json,
                &serde_json::json!({ "profile": name, "removed": removed }),
                if removed {
                    "profile removed; durable profile data was retained"
                } else {
                    "profile was not present"
                },
            )
        }
        ProfileCommand::Probe { name } => {
            let credentials = ClientCredentials::open(state_dir)?;
            let session = ProfileSession::open(&registry, &name)?;
            let report = credentials.with_checked_session(&session, |session| {
                session
                    .probe_and_pin()
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
            })?;
            output(json, &report, "host verified and pinned")
        }
        ProfileCommand::ResetHardState {
            name,
            confirm_delete,
        } => {
            if !confirm_delete {
                return Err(
                    "reset-hard-state requires --confirm-delete because it discards rollback protection, pins, mutation journals, and scheduled jobs"
                        .into(),
                );
            }
            let credentials = ClientCredentials::open(state_dir)?;
            let session = ProfileSession::open(&registry, &name)?;
            credentials.reset_hard_state(&session)?;
            output(
                json,
                &serde_json::json!({ "profile": name, "hard_state_reset": true }),
                "external checkpoint and hard-state database deleted; probe the profile again before use",
            )
        }
        ProfileCommand::ApplyCanary { name, artifact } => {
            let bytes = read_bounded_private_file(&artifact, 1024 * 1024)?;
            let signed: foks_compat_artifact::SignedCanaryArtifact =
                serde_json::from_slice(&bytes)?;
            let profile = registry.apply_canary(&name, &signed, now_seconds()?)?;
            output(json, &profile, "authenticated canary lease applied")
        }
    }
}

fn account_command(
    state_dir: &Path,
    json: bool,
    command: AccountCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match command {
        AccountCommand::List { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |_session, vault, _| {
                let aliases = vault.aliases()?;
                output(json, &aliases, &format!("{} account(s)", aliases.len()))
            })
        }
        AccountCommand::Create(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let passphrase = match (
                    arguments.passphrase_file.as_deref(),
                    arguments.passphrase_confirmation_file.as_deref(),
                ) {
                    (Some(passphrase), Some(confirmation)) => {
                        Some(confirmed_passphrase(passphrase, confirmation)?)
                    }
                    (None, None) => None,
                    _ => return Err("both passphrase files are required".into()),
                };
                let report = session.create_account(
                    &arguments.alias,
                    &arguments.username,
                    &arguments.device_name,
                    &arguments.email,
                    &arguments.invite,
                    passphrase,
                    vault,
                    master,
                )?;
                output(json, &report, "account created and synchronized")
            })
        }
        AccountCommand::Resume { profile, alias } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.resume_account(&alias, vault, master)?;
                output(json, &report, "account creation reconciled")
            })
        }
        AccountCommand::Sync { profile, alias } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.sync_account(&alias, vault)?;
                output(json, &report, "account synchronized")
            })
        }
    }
}

fn passphrase_command(
    state_dir: &Path,
    json: bool,
    command: PassphraseCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match command {
        PassphraseCommand::Set(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let passphrase = confirmed_passphrase(
                    &arguments.passphrase_file,
                    &arguments.passphrase_confirmation_file,
                )?;
                let report = session.set_passphrase(&arguments.alias, passphrase, vault)?;
                output(json, &report, "passphrase configured and verified")
            })
        }
        PassphraseCommand::Change(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let passphrase = confirmed_passphrase(
                    &arguments.passphrase_file,
                    &arguments.passphrase_confirmation_file,
                )?;
                let report = session.change_passphrase(&arguments.alias, passphrase, vault)?;
                output(json, &report, "passphrase changed and verified")
            })
        }
        PassphraseCommand::Verify(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let passphrase = Passphrase::new(read_passphrase(&arguments.passphrase_file)?)?;
                let report = session.verify_passphrase(&arguments.alias, passphrase, vault)?;
                output(json, &report, "passphrase verified")
            })
        }
    }
}

fn kv_command(
    state_dir: &Path,
    json: bool,
    command: KvCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match command {
        KvCommand::List { profile, alias } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.list_kv(&alias, vault)?;
                output(
                    json,
                    &report,
                    &format!("{} KV entry(s)", report.entries.len()),
                )
            })
        }
        KvCommand::Get {
            profile,
            alias,
            path,
            output: destination,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt as _;
                    options.mode(0o600);
                }
                let mut file = options.open(&destination)?;
                let result = (|| {
                    let bytes = session.write_kv_file(&alias, &path, vault, &mut file)?;
                    file.sync_all()?;
                    Ok::<_, Box<dyn std::error::Error>>(bytes)
                })();
                let bytes = match result {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        drop(file);
                        let _ = std::fs::remove_file(&destination);
                        return Err(error);
                    }
                };
                output(
                    json,
                    &serde_json::json!({
                        "path": path,
                        "output": destination,
                        "bytes": bytes,
                    }),
                    "KV file written without replacing an existing path",
                )
            })
        }
        KvCommand::Put {
            profile,
            alias,
            path,
            input,
            overwrite,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let mut file = File::open(input)?;
                let report =
                    session.put_kv_file(&alias, &path, &mut file, overwrite, vault, master)?;
                output(json, &report, "KV file committed and synchronized")
            })
        }
        KvCommand::Mkdir {
            profile,
            alias,
            path,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.mkdir_kv(&alias, &path, vault, master)?;
                output(json, &report, "KV directory committed and synchronized")
            })
        }
        KvCommand::Remove {
            profile,
            alias,
            path,
            recursive,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.remove_kv(&alias, &path, recursive, vault, master)?;
                output(json, &report, "KV entry removed and synchronized")
            })
        }
    }
}

fn job_command(
    state_dir: &Path,
    json: bool,
    command: JobCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match command {
        JobCommand::ScheduleUserRefresh {
            profile,
            alias,
            interval_seconds,
        } => {
            let interval_micros = interval_seconds
                .checked_mul(1_000_000)
                .ok_or("job interval overflow")?;
            let now = now_microseconds()?;
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let id = session.schedule_user_refresh(&alias, interval_micros, now, vault)?;
                output(
                    json,
                    &serde_json::json!({ "job_id": hex(&id), "first_run_at": now }),
                    "durable user-refresh job scheduled",
                )
            })
        }
        JobCommand::RunDue { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.run_due_jobs(now_microseconds()?, vault)?;
                output(
                    json,
                    &report,
                    &format!("{} scheduled job(s) processed", report.runs.len()),
                )
            })
        }
    }
}

fn device_command(
    state_dir: &Path,
    json: bool,
    command: DeviceCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match command {
        DeviceCommand::List { profile, alias } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let devices = session.list_devices(&alias, vault)?;
                output(
                    json,
                    &devices,
                    &format!("{} enrolled device(s)", devices.len()),
                )
            })
        }
        DeviceCommand::ProvisionOwner {
            profile,
            source_alias,
            target_alias,
            device_name,
            serial,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.provision_owner_device(
                    &source_alias,
                    &target_alias,
                    &device_name,
                    serial,
                    vault,
                    master,
                )?;
                output(json, &report, "owner device provisioned")
            })
        }
        DeviceCommand::ResumeProvision {
            profile,
            target_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.resume_owner_device_provision(&target_alias, vault, master)?;
                output(json, &report, "owner device provision reconciled")
            })
        }
    }
}

fn recovery_command(
    state_dir: &Path,
    json: bool,
    command: RecoveryCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match command {
        RecoveryCommand::Enroll {
            profile,
            account_alias,
            backup_alias,
            output: destination,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let phrase = session.enroll_owner_backup(&account_alias, &backup_alias, vault)?;
                write_new_private(&destination, phrase.as_bytes())?;
                output(
                    json,
                    &serde_json::json!({
                        "backup_alias": backup_alias,
                        "output": destination,
                        "tokens": 17,
                    }),
                    "backup enrolled; phrase written once to the requested private file",
                )
            })
        }
        RecoveryCommand::Recover {
            profile,
            target_alias,
            phrase_file,
            device_name,
            serial,
        } => {
            let phrase = read_phrase(&phrase_file)?;
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.recover_owner_account(
                    &target_alias,
                    phrase,
                    &device_name,
                    serial,
                    vault,
                )?;
                output(json, &report, "account recovered to a new owner device")
            })
        }
        RecoveryCommand::Resume {
            profile,
            target_alias,
            phrase_file,
            device_name,
        } => {
            let phrase = read_phrase(&phrase_file)?;
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report =
                    session.resume_owner_recovery(&target_alias, phrase, &device_name, vault)?;
                output(json, &report, "account recovery reconciled")
            })
        }
    }
}

fn yubi_command(
    state_dir: &Path,
    json: bool,
    command: YubiCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    let provider = HardwareYubiProvider::new();
    match command {
        YubiCommand::List { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |_session, vault, _| {
                let aliases = vault.yubi_aliases()?;
                output(
                    json,
                    &aliases,
                    &format!("{} Yubi credential(s)", aliases.len()),
                )
            })
        }
        YubiCommand::Cards { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, _vault, _| {
                let cards = session.list_yubi_cards(&provider)?;
                output(json, &cards, &format!("{} YubiKey(s)", cards.len()))
            })
        }
        YubiCommand::Create(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            let card = yubi_card(&provider, arguments.card_serial)?;
            let pin = read_pin(&arguments.pin_file)?;
            let invite = arguments
                .invite_file
                .as_deref()
                .map(read_invite)
                .transpose()?;
            with_vault(state_dir, &session, |session, vault, master| {
                let passphrase = match (
                    arguments.passphrase_file.as_deref(),
                    arguments.passphrase_confirmation_file.as_deref(),
                ) {
                    (Some(passphrase), Some(confirmation)) => {
                        Some(confirmed_passphrase(passphrase, confirmation)?)
                    }
                    (None, None) => None,
                    _ => return Err("both passphrase files are required".into()),
                };
                let report = session.create_yubi_account(
                    YubiSignupInput {
                        alias: arguments.alias,
                        username: arguments.username,
                        device_name: arguments.device_name,
                        email: arguments.email,
                        invite: invite
                            .as_ref()
                            .map_or_else(String::new, |value| value.as_str().to_owned()),
                        passphrase,
                        card,
                        signing_slot: SlotId::new(arguments.signing_slot)?,
                        pq_slot: SlotId::new(arguments.pq_slot)?,
                    },
                    pin,
                    &provider,
                    vault,
                    master,
                )?;
                output(json, &report, "YubiKey account created and synchronized")
            })
        }
        YubiCommand::ResumeAccount {
            profile,
            alias,
            pin_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let pin = read_pin(&pin_file)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.resume_yubi_account(&alias, pin, &provider, vault, master)?;
                output(json, &report, "YubiKey account creation reconciled")
            })
        }
        YubiCommand::Provision(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            let card = yubi_card(&provider, arguments.card_serial)?;
            let pin = read_pin(&arguments.pin_file)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.provision_yubi_device(
                    YubiProvisionInput {
                        source_alias: arguments.source_alias,
                        target_alias: arguments.target_alias,
                        device_name: arguments.device_name,
                        serial: arguments.serial,
                        card,
                        signing_slot: SlotId::new(arguments.signing_slot)?,
                        pq_slot: SlotId::new(arguments.pq_slot)?,
                    },
                    pin,
                    &provider,
                    vault,
                    master,
                )?;
                output(json, &report, "YubiKey device provisioned")
            })
        }
        YubiCommand::Sync {
            profile,
            alias,
            pin_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let pin = read_pin(&pin_file)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.sync_yubi_account(&alias, pin, &provider, vault)?;
                output(json, &report, "YubiKey account synchronized")
            })
        }
        YubiCommand::PinStatus { profile, alias } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let status = session.yubi_pin_status(&alias, &provider, vault)?;
                output(json, &status, "YubiKey PIN status read")
            })
        }
        YubiCommand::ChangePin {
            profile,
            alias,
            old_pin_file,
            new_pin_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let old_pin = read_pin(&old_pin_file)?;
            let new_pin = read_pin(&new_pin_file)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let status = session.change_yubi_pin(&alias, old_pin, new_pin, &provider, vault)?;
                output(json, &status, "YubiKey PIN changed")
            })
        }
        YubiCommand::ChangePuk {
            profile,
            alias,
            old_puk_file,
            new_puk_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let old_puk = read_pin(&old_puk_file)?;
            let new_puk = read_pin(&new_puk_file)?;
            with_vault(state_dir, &session, |session, vault, _| {
                session.change_yubi_puk(&alias, old_puk, new_puk, &provider, vault)?;
                output(
                    json,
                    &serde_json::json!({ "alias": alias, "changed": true }),
                    "YubiKey PUK changed",
                )
            })
        }
        YubiCommand::UnblockPin {
            profile,
            alias,
            puk_file,
            new_pin_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let puk = read_pin(&puk_file)?;
            let new_pin = read_pin(&new_pin_file)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let status = session.unblock_yubi_pin(&alias, puk, new_pin, &provider, vault)?;
                output(json, &status, "YubiKey PIN unblocked")
            })
        }
        YubiCommand::ConfigureRetries {
            profile,
            alias,
            pin_file,
            puk_file,
            pin_attempts,
            puk_attempts,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let pin = read_pin(&pin_file)?;
            let puk = read_pin(&puk_file)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let status = session.configure_yubi_retries(
                    &alias,
                    pin,
                    puk,
                    pin_attempts,
                    puk_attempts,
                    &provider,
                    vault,
                )?;
                output(json, &status, "YubiKey retry policy changed")
            })
        }
        YubiCommand::RotateManagementKey {
            profile,
            alias,
            pin_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let pin = read_pin(&pin_file)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.rotate_yubi_management_key(&alias, pin, &provider, vault)?;
                output(json, &report, "YubiKey management key rotated")
            })
        }
        YubiCommand::ResumeManagementKey {
            profile,
            alias,
            pin_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let pin = pin_file.as_deref().map(read_pin).transpose()?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.resume_yubi_management_key(&alias, pin, &provider, vault)?;
                output(json, &report, "YubiKey management-key rotation reconciled")
            })
        }
        YubiCommand::RecoverManagementKey {
            profile,
            yubi_alias,
            software_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report =
                    session.recover_yubi_management_key(&yubi_alias, &software_alias, vault)?;
                output(json, &report, "YubiKey management key recovered")
            })
        }
        YubiCommand::RecoverSubkey {
            profile,
            alias,
            pin_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let pin = read_pin(&pin_file)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.recover_yubi_subkey(&alias, pin, &provider, vault)?;
                output(json, &report, "YubiKey delegated subkey recovered")
            })
        }
        YubiCommand::Revoke {
            profile,
            yubi_alias,
            software_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report =
                    session.revoke_yubi_device(&software_alias, &yubi_alias, vault, master)?;
                output(
                    json,
                    &report,
                    "YubiKey device revoked and local credential removed",
                )
            })
        }
    }
}

fn yubi_card(
    provider: &HardwareYubiProvider,
    serial: u32,
) -> Result<CardId, Box<dyn std::error::Error>> {
    let matches = provider
        .cards()?
        .into_iter()
        .filter(|card| card.serial == serial)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [card] => Ok(card.clone()),
        [] => Err(format!("YubiKey serial {serial} is not connected").into()),
        _ => Err(format!("YubiKey serial {serial} is ambiguous").into()),
    }
}

fn read_pin(path: &Path) -> Result<Pin, Box<dyn std::error::Error>> {
    let value = read_passphrase(path)?;
    Ok(Pin::new(value.as_str())?)
}

fn read_invite(path: &Path) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 4098 {
        return Err("invite file is not a bounded regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("invite file permissions allow group or other access".into());
        }
    }
    let mut value = Zeroizing::new(String::new());
    file.take(4099).read_to_string(&mut value)?;
    if value.ends_with("\r\n") {
        let length = value.len() - 2;
        value.truncate(length);
    } else if value.ends_with('\n') {
        let length = value.len() - 1;
        value.truncate(length);
    }
    if value.is_empty() || value.len() > 4096 || value.contains(['\0', '\r', '\n']) {
        return Err("invite must be one nonempty line of at most 4096 bytes".into());
    }
    Ok(value)
}

fn parse_slot(value: &str) -> Result<u8, String> {
    let value = value.trim();
    let parsed = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or_else(
            || value.parse::<u8>(),
            |value| u8::from_str_radix(value, 16),
        )
        .map_err(|_| "slot must be an 8-bit decimal or 0x-prefixed value".to_owned())?;
    SlotId::new(parsed).map_err(|error| error.to_string())?;
    Ok(parsed)
}

fn team_command(
    state_dir: &Path,
    json: bool,
    command: TeamCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match command {
        TeamCommand::List { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let teams = session.list_teams(vault)?;
                output(json, &teams, &format!("{} team(s)", teams.len()))
            })
        }
        TeamCommand::CreateNamed {
            profile,
            account_alias,
            team_alias,
            name,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report =
                    session.create_named_team(&account_alias, &team_alias, &name, vault, master)?;
                output(json, &report, "named team created and synchronized")
            })
        }
        TeamCommand::CreateAdhoc {
            profile,
            account_alias,
            team_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report =
                    session.create_adhoc_team(&account_alias, &team_alias, vault, master)?;
                output(json, &report, "ad-hoc team created and synchronized")
            })
        }
        TeamCommand::ResumeCreate {
            profile,
            team_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.resume_team_creation(&team_alias, vault, master)?;
                output(json, &report, "team creation reconciled")
            })
        }
        TeamCommand::Sync {
            profile,
            team_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.sync_team(&team_alias, vault)?;
                output(json, &report, "team and team KV synchronized")
            })
        }
    }
}

fn read_phrase(path: &Path) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return Err("backup phrase file is not a bounded regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("backup phrase file permissions allow group or other access".into());
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let mut phrase = Zeroizing::new(String::new());
    file.take(4097).read_to_string(&mut phrase)?;
    if phrase.len() > 4096 {
        return Err("backup phrase file grew beyond the size limit".into());
    }
    if phrase.split_whitespace().count() != 17 {
        return Err("backup phrase file does not contain exactly 17 tokens".into());
    }
    Ok(phrase)
}

fn confirmed_passphrase(
    path: &Path,
    confirmation_path: &Path,
) -> Result<Passphrase, Box<dyn std::error::Error>> {
    let passphrase = read_passphrase(path)?;
    let confirmation = read_passphrase(confirmation_path)?;
    if passphrase.as_bytes() != confirmation.as_bytes() {
        return Err("passphrase confirmation does not match".into());
    }
    Ok(Passphrase::new(passphrase.as_bytes())?)
}

fn read_passphrase(path: &Path) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    let mut options = OpenOptions::new();
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
    if value.len() > 1026 || value.contains('\0') {
        return Err("passphrase file is invalid or excessive".into());
    }
    if value.ends_with("\r\n") {
        let length = value.len() - 2;
        value.truncate(length);
    } else if value.ends_with('\n') {
        let length = value.len() - 1;
        value.truncate(length);
    }
    if value.contains(['\r', '\n']) || value.is_empty() || value.len() > 1024 {
        return Err("passphrase must be one nonempty line of at most 1024 bytes".into());
    }
    Ok(value)
}

fn read_bounded_private_file(
    path: &Path,
    maximum: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err("artifact is not a bounded regular file".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err("artifact grew beyond its size limit".into());
    }
    Ok(bytes)
}

fn write_new_private(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn now_microseconds() -> Result<u64, Box<dyn std::error::Error>> {
    use std::time::{SystemTime, UNIX_EPOCH};

    Ok(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros(),
    )?)
}

fn now_seconds() -> Result<u64, Box<dyn std::error::Error>> {
    use std::time::{SystemTime, UNIX_EPOCH};
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
        output
    })
}

fn with_vault<T>(
    state_dir: &Path,
    session: &ProfileSession,
    operation: impl FnOnce(
        &CheckedProfileSession<'_>,
        &mut AccountVault<'_>,
        &[u8; 32],
    ) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let credentials = ClientCredentials::open(state_dir)?;
    credentials.with_checked_session(session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        operation(session, &mut AccountVault::new(&mut store), &master)
    })
}

fn output(
    json: bool,
    value: &impl serde::Serialize,
    human: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{human}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_and_profile_commands_use_only_the_explicit_state_root() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("foks");
        initialize(&state, false, CredentialBackend::PrivateFile).unwrap();
        profile_command(
            &state,
            false,
            ProfileCommand::Add(ProfileAdd {
                name: "local".to_owned(),
                target: "localhost:4430".to_owned(),
                generation: ProfileGeneration::V019,
                ca_der: Some(state.join("local-ca.der")),
                canary_public_key: None,
                canary_url: None,
            }),
        )
        .unwrap();
        assert!(state.join("profiles.toml").is_file());
        assert!(state.join("master.key").is_file());
        assert_eq!(ProfileRegistry::open(&state).unwrap().profiles().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn secret_input_files_are_private_bounded_and_normalized_once() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let private = directory.path().join("passphrase");
        std::fs::write(&private, b"correct horse\n").unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_passphrase(&private).unwrap().as_str(), "correct horse");

        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(read_passphrase(&private).is_err());

        let target = directory.path().join("target");
        std::fs::write(&target, b"secret").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read_passphrase(&link).is_err());

        std::fs::write(&private, b"s.private-invite\n").unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_invite(&private).unwrap().as_str(), "s.private-invite");
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_invite(&private).is_err());
        assert!(read_invite(&link).is_err());
    }
}
