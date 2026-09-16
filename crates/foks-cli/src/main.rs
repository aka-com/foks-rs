#![forbid(unsafe_code)]
mod account_conveniences;
mod bot_token;
mod invitations;
mod mcp;
mod retention;
mod sso;
mod state;
mod web_admin;

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use clap::{Parser as _, ValueEnum};
use foks_client_app::{
    derive_vault_key, AccountVault, Capability, CheckedProfileSession, ClientCredentials,
    CredentialBackend, FederationDestinationRole, KexAcceptanceInput, Passphrase, Profile,
    ProfileRegistry, ProfileSession, ProtocolPolicy, TeamMemberRole, TrustRoot, UnlockedYubiActor,
    YubiProvisionInput, YubiSignupInput,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_yubi::{
    CardId, HardwareYubiProvider, Pin, PinRetryConfiguration, SlotId, YubiProvider as _,
};
use zeroize::Zeroizing;

#[derive(clap::Parser)]
#[command(name = "foks-rs", about = "Standalone Rust client for FOKS")]
struct Arguments {
    /// Application state directory. The CLI does not infer a default path or read AKA state.
    #[arg(long)]
    state_dir: PathBuf,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    #[command(subcommand)]
    State(state::StateCommand),
    #[command(subcommand)]
    Mcp(mcp::McpCommand),
    #[command(subcommand)]
    Retention(retention::RetentionCommand),
    #[command(subcommand)]
    Sso(sso::SsoCommand),
    /// Initialize FOKS client state.
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
    /// Credential backend. The native backend uses Keychain on macOS and Secret Service on Linux.
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
    Show {
        name: String,
    },
    Add(ProfileAdd),
    Verify(ProfileAdd),
    /// Delete a profile and its associated local state.
    Remove {
        name: String,
        #[arg(long)]
        confirm_delete: bool,
    },
    Probe {
        name: String,
    },
    /// Delete a profile's rollback checkpoint and hard-state database.
    ResetHardState {
        name: String,
        #[arg(long)]
        confirm_delete: bool,
    },
    /// Applies a signed compatibility lease or drift revocation artifact.
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
    #[command(subcommand)]
    Admin(web_admin::AdminCommand),
    #[command(subcommand)]
    Bot(bot_token::BotCommand),
    #[command(subcommand)]
    Rename(account_conveniences::RenameCommand),
    List {
        profile: String,
    },
    Create(AccountCreate),
    Resume {
        profile: String,
        alias: String,
    },
    Sync {
        profile: String,
        alias: String,
    },
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
        /// Create intermediate parent directories as needed.
        #[arg(long = "mkdir-p", short = 'p')]
        mkdir_p: bool,
    },
    Mkdir {
        profile: String,
        alias: String,
        path: String,
        /// Create intermediate parent directories as needed.
        #[arg(long = "mkdir-p", short = 'p')]
        mkdir_p: bool,
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
    /// Revoke exactly one software device and rotate account keys.
    Revoke {
        profile: String,
        alias: String,
        device_id: String,
    },
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
    PairOffer {
        profile: String,
        account_alias: String,
    },
    PairRepublish {
        profile: String,
        account_alias: String,
    },
    PairFinish {
        profile: String,
        account_alias: String,
    },
    PairAccept {
        profile: String,
        target_alias: String,
        #[arg(long)]
        phrase_file: PathBuf,
        #[arg(long)]
        device_name: String,
        #[arg(long, default_value_t = 1)]
        serial: u64,
    },
    PairResumeAccept {
        profile: String,
        target_alias: String,
    },
}

#[derive(clap::Subcommand)]
enum RecoveryCommand {
    /// Revoke exactly one locally recorded backup credential.
    Revoke {
        profile: String,
        account_alias: String,
        backup_alias: String,
        backup_id: String,
    },
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
    #[command(subcommand)]
    Invite(invitations::InvitationCommand),
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
    ListMembers {
        profile: String,
        team_alias: String,
    },
    AddMember {
        profile: String,
        team_alias: String,
        username: String,
        #[arg(long, value_enum, default_value_t = FederationRoleArgument::Member)]
        role: FederationRoleArgument,
        #[arg(long, default_value_t = 0)]
        visibility: i16,
    },
    ResumeAddMember {
        profile: String,
        team_alias: String,
        username: String,
    },
    /// Demotes a team member to a lower role.
    DemoteMember {
        profile: String,
        team_alias: String,
        username: String,
        #[arg(long, value_enum, default_value_t = FederationRoleArgument::Member)]
        role: FederationRoleArgument,
        #[arg(long, default_value_t = 0)]
        visibility: i16,
    },
    RemoveMember {
        profile: String,
        team_alias: String,
        username: String,
    },
    ResumeMemberEdit {
        profile: String,
        team_alias: String,
    },
    /// Add a team from another pinned profile to a local named team.
    AdmitRemote {
        local_profile: String,
        local_team_alias: String,
        remote_profile: String,
        remote_team_alias: String,
        #[arg(long, value_enum, default_value_t = FederationRoleArgument::Member)]
        role: FederationRoleArgument,
        #[arg(long, default_value_t = 0)]
        visibility: i16,
    },
    /// Lists protected remote-team bindings for one local team.
    ListRemote {
        profile: String,
        team_alias: String,
    },
    /// Refresh federated security state for a local team.
    ///
    /// Supply `--local-pin-file` and/or `--remote-pin-file` when an administrator
    /// requires hardware authentication. If the refresh accesses
    /// additional hardware-authenticated profiles, supply
    /// `--unlock PROFILE=ALIAS=PIN_FILE` for each.
    RefreshRemote {
        profile: String,
        team_alias: String,
        #[arg(long, requires = "local_pin_file")]
        local_yubi_alias: Option<String>,
        #[arg(long, requires = "local_yubi_alias")]
        local_pin_file: Option<PathBuf>,
        #[arg(long, requires = "remote_yubi_alias")]
        remote_profile: Option<String>,
        #[arg(long, requires_all = ["remote_profile", "remote_pin_file"])]
        remote_yubi_alias: Option<String>,
        #[arg(long, requires = "remote_yubi_alias")]
        remote_pin_file: Option<PathBuf>,
        /// Additional PROFILE=ALIAS=PIN_FILE hardware unlock. Repeat for
        /// each additional hardware-authenticated profile accessed by the refresh.
        #[arg(long = "unlock")]
        unlocks: Vec<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum FederationRoleArgument {
    Member,
    Admin,
    Owner,
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
    /// Private one-line file containing a signup invite.
    #[arg(long)]
    invite_file: Option<PathBuf>,
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
        /// Also run federated security responders for teams administrable by
        /// this key. Bindings requiring unsupplied hardware credentials will be
        /// deferred rather than failing the sync.
        #[arg(long)]
        with_federation: bool,
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
    #[command(flatten)]
    retry: YubiRetryArguments,
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
    #[command(flatten)]
    retry: YubiRetryArguments,
}

#[derive(clap::Args)]
struct YubiRetryArguments {
    /// Private PUK file used only to set retry counts before FOKS key generation.
    #[arg(long, requires_all = ["pin_attempts", "puk_attempts"])]
    retry_puk_file: Option<PathBuf>,
    /// PIN retry count to set during initial card preparation (1 through 15).
    #[arg(long, requires_all = ["retry_puk_file", "puk_attempts"])]
    pin_attempts: Option<u8>,
    /// PUK retry count to set during initial card preparation (1 through 15).
    #[arg(long, requires_all = ["retry_puk_file", "pin_attempts"])]
    puk_attempts: Option<u8>,
}

#[derive(clap::Args)]
struct PassphraseChange {
    profile: String,
    alias: String,
    #[arg(long)]
    passphrase_file: PathBuf,
    #[arg(long)]
    passphrase_confirmation_file: PathBuf,
    /// Private PIN file; when supplied, the alias is treated as Yubi-backed.
    #[arg(long)]
    pin_file: Option<PathBuf>,
}

#[derive(clap::Args)]
struct PassphraseVerify {
    profile: String,
    alias: String,
    #[arg(long)]
    passphrase_file: PathBuf,
    /// Private PIN file; when supplied, the alias is treated as Yubi-backed.
    #[arg(long)]
    pin_file: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run(Arguments::parse()) {
        eprintln!("foks-rs: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), Box<dyn std::error::Error>> {
    match arguments.command {
        Command::State(command) => state::run(&arguments.state_dir, command),
        Command::Mcp(command) => mcp::run(&arguments.state_dir, command),
        Command::Retention(command) => retention::run(&arguments.state_dir, command),
        Command::Sso(command) => sso::run(&arguments.state_dir, command),
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
        "initialized FOKS client state",
    )
}

fn profile_from_arguments(arguments: ProfileAdd) -> Result<Profile, Box<dyn std::error::Error>> {
    let protocol = match arguments.generation {
        ProfileGeneration::V019 => {
            if arguments.canary_public_key.is_some() || arguments.canary_url.is_some() {
                return Err(
                    "v0.1.9 profiles do not support compatibility-lease configuration".into(),
                );
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
    Ok(Profile {
        name: arguments.name,
        label: None,
        probe: arguments.target,
        protocol,
        trust,
    })
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
        ProfileCommand::Show { name } => {
            let profile = registry.profile(&name)?;
            output(
                json,
                profile,
                &format!("{} -> {}", profile.name, profile.probe),
            )
        }
        ProfileCommand::Add(arguments) => {
            let profile = profile_from_arguments(arguments)?;
            registry.add(profile.clone())?;
            output(json, &profile, "profile added")
        }
        ProfileCommand::Verify(arguments) => {
            let expected = profile_from_arguments(arguments)?;
            let configured = registry.profile(&expected.name)?;
            if configured != &expected {
                return Err(format!(
                    "FOKS profile '{}' does not match the required configuration",
                    expected.name
                )
                .into());
            }
            output(json, configured, "profile verified")
        }
        ProfileCommand::Remove {
            name,
            confirm_delete,
        } => {
            if !confirm_delete {
                return Err(
                    "profile remove requires --confirm-delete because it erases this profile's account credentials, rollback checkpoint, pins, mutation journals, and scheduled jobs"
                        .into(),
                );
            }
            let credentials = ClientCredentials::open(state_dir)?;
            let removed = credentials.remove_profile(&mut registry, &name)?;
            output(
                json,
                &serde_json::json!({ "profile": name, "removed": removed }),
                if removed {
                    "profile removed and its local state erased"
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
        AccountCommand::Admin(command) => web_admin::run(state_dir, command),
        AccountCommand::Bot(command) => bot_token::run(state_dir, command),
        AccountCommand::Rename(command) => account_conveniences::run(state_dir, command),
        AccountCommand::List { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |_session, vault, _| {
                let aliases = vault.aliases()?;
                output(json, &aliases, &format!("{} account(s)", aliases.len()))
            })
        }
        AccountCommand::Create(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
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
                    _ => return Err("both --passphrase-file and --passphrase-confirmation-file are required when setting a passphrase".into()),
                };
                let report = session.create_account(
                    &arguments.alias,
                    &arguments.username,
                    &arguments.device_name,
                    &arguments.email,
                    invite.as_deref().map_or("", String::as_str),
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
                output(json, &report, "account created")
            })
        }
        AccountCommand::Sync { profile, alias } => {
            mcp::ensure_agent(state_dir)?;
            let response = foks_agent_client::AgentClient::new(state_dir.join("foks-rs.sock"))
                .call(foks_agent_proto::Operation::SyncAccount { profile, alias })?;
            match response.result {
                foks_agent_proto::ResponseResult::Success { value } => {
                    output(json, &value, "account synchronized")
                }
                foks_agent_proto::ResponseResult::Error { message, .. } => Err(message.into()),
            }
        }
    }
}

fn passphrase_command(
    state_dir: &Path,
    json: bool,
    command: PassphraseCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    let provider = HardwareYubiProvider::new();
    match command {
        PassphraseCommand::Set(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let passphrase = confirmed_passphrase(
                    &arguments.passphrase_file,
                    &arguments.passphrase_confirmation_file,
                )?;
                let report = match arguments.pin_file.as_deref() {
                    Some(path) => session.set_yubi_passphrase(
                        &arguments.alias,
                        read_pin(path)?,
                        passphrase,
                        &provider,
                        vault,
                        master,
                    )?,
                    None => session.set_passphrase(&arguments.alias, passphrase, vault)?,
                };
                output(json, &report, "passphrase configured and verified")
            })
        }
        PassphraseCommand::Change(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let passphrase = confirmed_passphrase(
                    &arguments.passphrase_file,
                    &arguments.passphrase_confirmation_file,
                )?;
                let report = match arguments.pin_file.as_deref() {
                    Some(path) => session.change_yubi_passphrase(
                        &arguments.alias,
                        read_pin(path)?,
                        passphrase,
                        &provider,
                        vault,
                        master,
                    )?,
                    None => session.change_passphrase(&arguments.alias, passphrase, vault)?,
                };
                output(json, &report, "passphrase changed and verified")
            })
        }
        PassphraseCommand::Verify(arguments) => {
            let session = ProfileSession::open(&registry, &arguments.profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let passphrase = Passphrase::new(read_passphrase(&arguments.passphrase_file)?)?;
                let report = match arguments.pin_file.as_deref() {
                    Some(path) => session.verify_yubi_passphrase(
                        &arguments.alias,
                        read_pin(path)?,
                        passphrase,
                        &provider,
                        vault,
                        master,
                    )?,
                    None => session.verify_passphrase(&arguments.alias, passphrase, vault)?,
                };
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
                    "KV file downloaded successfully",
                )
            })
        }
        KvCommand::Put {
            profile,
            alias,
            path,
            input,
            overwrite,
            mkdir_p,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let mut file = File::open(input)?;
                let report = session
                    .put_kv_file(&alias, &path, &mut file, overwrite, mkdir_p, vault, master)?;
                output(json, &report, "KV file committed and synchronized")
            })
        }
        KvCommand::Mkdir {
            profile,
            alias,
            path,
            mkdir_p,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.mkdir_kv(&alias, &path, mkdir_p, vault, master)?;
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
            let credentials = ClientCredentials::open(state_dir)?;
            credentials.with_checked_session(&session, |session| {
                let master = credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                let report = session.run_due_jobs_with_federation(
                    now_microseconds()?,
                    &mut vault,
                    &registry,
                    &credentials,
                    &master,
                )?;
                // Surface deferred jobs in human-readable output so operator
                // action items are visible without requiring --json.
                let deferred = report
                    .runs
                    .iter()
                    .filter_map(|run| run.deferred.as_deref())
                    .collect::<Vec<_>>();
                let human = if deferred.is_empty() {
                    format!("{} scheduled job(s) processed", report.runs.len())
                } else {
                    format!(
                        "{} scheduled job(s) processed; {} deferred: {}",
                        report.runs.len(),
                        deferred.len(),
                        deferred.join("; ")
                    )
                };
                output(json, &report, &human)
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
        DeviceCommand::Revoke {
            profile,
            alias,
            device_id,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.remove_software_device(&alias, &device_id, vault, master)?;
                output(json, &report, "software device revoked")
            })
        }
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
                output(json, &report, "device linked")
            })
        }
        DeviceCommand::PairOffer {
            profile,
            account_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.start_owner_device_pairing(&account_alias, vault)?;
                output(json, &report, &report.phrase)
            })
        }
        DeviceCommand::PairRepublish {
            profile,
            account_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.republish_owner_device_pairing(&account_alias, vault)?;
                output(json, &report, &report.phrase)
            })
        }
        DeviceCommand::PairFinish {
            profile,
            account_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.finish_owner_device_pairing(&account_alias, vault, master)?;
                output(json, &report, "device pairing completed")
            })
        }
        DeviceCommand::PairAccept {
            profile,
            target_alias,
            phrase_file,
            device_name,
            serial,
        } => {
            let phrase = read_kex_phrase(&phrase_file)?;
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report = session.accept_owner_device_pairing(
                    KexAcceptanceInput {
                        target_alias,
                        device_name,
                        serial,
                        phrase: phrase.to_string(),
                    },
                    vault,
                )?;
                output(json, &report, "interactive owner-device pairing accepted")
            })
        }
        DeviceCommand::PairResumeAccept {
            profile,
            target_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let report =
                    session.resume_owner_device_pairing_acceptance(&target_alias, vault)?;
                output(json, &report, "interactive pairing acceptance resumed")
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
        RecoveryCommand::Revoke {
            profile,
            account_alias,
            backup_alias,
            backup_id,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.revoke_owner_backup(
                    &account_alias,
                    &backup_alias,
                    &backup_id,
                    vault,
                    master,
                )?;
                output(json, &report, "backup credential revoked")
            })
        }
        RecoveryCommand::Enroll {
            profile,
            account_alias,
            backup_alias,
            output: destination,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let phrase = session
                    .prepare_owner_backup(&account_alias, &backup_alias, vault)?
                    .expose_joined();
                write_new_private(&destination, phrase.as_bytes())?;
                let report = session.commit_owner_backup(
                    &account_alias,
                    &backup_alias,
                    Zeroizing::new(phrase.as_str().to_owned()),
                    vault,
                )?;
                output(
                    json,
                    &serde_json::json!({
                        "backup_alias": backup_alias,
                        "backup_id_hex": report.backup_id_hex,
                        "output": destination,
                        "tokens": 17,
                    }),
                    "backup credential enrolled; recovery phrase written to the requested file",
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
            let retry_configuration = read_yubi_retry_configuration(&arguments.retry)?;
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
                    _ => return Err("both --passphrase-file and --passphrase-confirmation-file are required when setting a passphrase".into()),
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
                        retry_configuration,
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
            let retry_configuration = read_yubi_retry_configuration(&arguments.retry)?;
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
                        retry_configuration,
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
            with_federation,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let pin = read_pin(&pin_file)?;
            if !with_federation {
                return with_vault(state_dir, &session, |session, vault, master| {
                    let report =
                        session.sync_yubi_account(&alias, pin, &provider, vault, master)?;
                    output(json, &report, "YubiKey account synchronized")
                });
            }
            let credentials = ClientCredentials::open(state_dir)?;
            credentials.with_checked_session(&session, |session| {
                let master = credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let report = session.sync_yubi_account_with_federation(
                    &alias,
                    pin,
                    &provider,
                    &mut AccountVault::new(&mut store),
                    &registry,
                    &credentials,
                    &master,
                )?;
                output(
                    json,
                    &report,
                    &format!(
                        "YubiKey account synchronized; {} federated binding(s) processed",
                        report.federation.len()
                    ),
                )
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
        YubiCommand::RotateManagementKey {
            profile,
            alias,
            pin_file,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            let pin = read_pin(&pin_file)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report =
                    session.rotate_yubi_management_key(&alias, pin, &provider, vault, master)?;
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
            with_vault(state_dir, &session, |session, vault, master| {
                let report =
                    session.resume_yubi_management_key(&alias, pin, &provider, vault, master)?;
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
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.recover_yubi_subkey(&alias, pin, &provider, vault, master)?;
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

fn read_yubi_retry_configuration(
    arguments: &YubiRetryArguments,
) -> Result<Option<PinRetryConfiguration>, Box<dyn std::error::Error>> {
    match (
        arguments.retry_puk_file.as_deref(),
        arguments.pin_attempts,
        arguments.puk_attempts,
    ) {
        (None, None, None) => Ok(None),
        (Some(puk_file), Some(pin_attempts), Some(puk_attempts)) => Ok(Some(
            PinRetryConfiguration::new(read_pin(puk_file)?, pin_attempts, puk_attempts)?,
        )),
        _ => Err("retry PUK file, PIN attempts, and PUK attempts must be supplied together".into()),
    }
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
        return Err("invite file must be a regular file of at most 4096 bytes".into());
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
        TeamCommand::Invite(command) => invitations::run(state_dir, command),
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
                output(json, &report, "team created")
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
        TeamCommand::ListMembers {
            profile,
            team_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let members = session.list_team_members(&team_alias, vault)?;
                output(json, &members, &format!("{} team member(s)", members.len()))
            })
        }
        TeamCommand::AddMember {
            profile,
            team_alias,
            username,
            role,
            visibility,
        } => {
            let destination = local_team_destination(role, visibility)?;
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.add_local_team_member(
                    &team_alias,
                    &username,
                    destination,
                    vault,
                    master,
                )?;
                output(json, &report, "local team member added")
            })
        }
        TeamCommand::ResumeAddMember {
            profile,
            team_alias,
            username,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.resume_local_team_member_addition(
                    &team_alias,
                    &username,
                    vault,
                    master,
                )?;
                output(json, &report, "local team-member addition reconciled")
            })
        }
        TeamCommand::DemoteMember {
            profile,
            team_alias,
            username,
            role,
            visibility,
        } => {
            let destination = local_team_destination(role, visibility)?;
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.demote_local_team_member(
                    &team_alias,
                    &username,
                    destination,
                    vault,
                    master,
                )?;
                output(json, &report, "local team member demoted")
            })
        }
        TeamCommand::RemoveMember {
            profile,
            team_alias,
            username,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report =
                    session.remove_local_team_member(&team_alias, &username, vault, master)?;
                output(json, &report, "local team member removed")
            })
        }
        TeamCommand::ResumeMemberEdit {
            profile,
            team_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, master| {
                let report = session.resume_local_team_member_edit(&team_alias, vault, master)?;
                output(json, &report, "team member edit reconciled")
            })
        }
        TeamCommand::AdmitRemote {
            local_profile,
            local_team_alias,
            remote_profile,
            remote_team_alias,
            role,
            visibility,
        } => {
            let destination = federation_destination(role, visibility)?;
            let local = ProfileSession::open(&registry, &local_profile)?;
            let remote = ProfileSession::open(&registry, &remote_profile)?;
            let credentials = ClientCredentials::open(state_dir)?;
            let report = credentials.with_checked_sessions(&local, &remote, |local, remote| {
                let master = credentials.master_key()?;
                let mut local_store = EncryptedFileSecretStore::open(
                    &local.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut remote_store = EncryptedFileSecretStore::open(
                    &remote.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                local.admit_federated_team(
                    remote,
                    &local_team_alias,
                    &remote_team_alias,
                    destination,
                    &mut AccountVault::new(&mut local_store),
                    &mut AccountVault::new(&mut remote_store),
                    &master,
                )
            })?;
            output(
                json,
                &report,
                "remote team added and reconciliation scheduled",
            )
        }
        TeamCommand::RefreshRemote {
            profile,
            team_alias,
            local_yubi_alias,
            local_pin_file,
            remote_profile,
            remote_yubi_alias,
            remote_pin_file,
            unlocks,
        } => refresh_remote_command(
            state_dir,
            json,
            &registry,
            RefreshRemoteArguments {
                profile,
                team_alias,
                local_yubi_alias,
                local_pin_file,
                remote_profile,
                remote_yubi_alias,
                remote_pin_file,
                unlocks,
            },
        ),
        TeamCommand::ListRemote {
            profile,
            team_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |session, vault, _| {
                let memberships = session.list_federated_memberships(&team_alias, vault)?;
                output(
                    json,
                    &memberships,
                    &format!("{} federated team binding(s)", memberships.len()),
                )
            })
        }
    }
}

struct RefreshRemoteArguments {
    profile: String,
    team_alias: String,
    local_yubi_alias: Option<String>,
    local_pin_file: Option<PathBuf>,
    remote_profile: Option<String>,
    remote_yubi_alias: Option<String>,
    remote_pin_file: Option<PathBuf>,
    unlocks: Vec<String>,
}

/// One `PROFILE=ALIAS=PIN_FILE` unlock, parsed but not yet opened.
struct RequestedUnlock {
    profile: String,
    alias: String,
    pin_file: PathBuf,
}

fn parse_unlock(value: &str) -> Result<RequestedUnlock, Box<dyn std::error::Error>> {
    let mut fields = value.splitn(3, '=');
    let profile = fields.next().unwrap_or_default();
    let alias = fields.next().unwrap_or_default();
    let pin_file = fields.next().unwrap_or_default();
    if profile.is_empty() || alias.is_empty() || pin_file.is_empty() {
        return Err("--unlock must be PROFILE=ALIAS=PIN_FILE".into());
    }
    Ok(RequestedUnlock {
        profile: profile.to_owned(),
        alias: alias.to_owned(),
        pin_file: PathBuf::from(pin_file),
    })
}

/// Drives one federated security responder with a YubiKey unlocked on any
/// number of the profiles the refresh will touch.
///
/// Lock ordering matters here. Every profile's durable Yubi record is read
/// under its own short-lived checked session, all of which are released before
/// the refresh takes the local one, because the refresh re-acquires the other
/// profiles without waiting. Holding their operation locks across it would
/// make each of them look permanently busy. The record only names the device;
/// the credential built from it is re-checked against the authenticated user
/// chain at every use, so reading it ahead of the refresh grants nothing.
fn refresh_remote_command(
    state_dir: &Path,
    json: bool,
    registry: &ProfileRegistry,
    arguments: RefreshRemoteArguments,
) -> Result<(), Box<dyn std::error::Error>> {
    let provider = HardwareYubiProvider::new();
    let session = ProfileSession::open(registry, &arguments.profile)?;
    // Verify capabilities before hardware interaction to avoid unnecessary
    // PIN attempts or user prompts if the profile lacks permission.
    session.profile().require(Capability::Teams)?;
    session.profile().require(Capability::Federation)?;
    let credentials = ClientCredentials::open(state_dir)?;
    let master = credentials.master_key()?;

    let mut requested = Vec::new();
    if let (Some(alias), Some(pin_file)) = (
        arguments.local_yubi_alias.as_deref(),
        arguments.local_pin_file.as_deref(),
    ) {
        requested.push(RequestedUnlock {
            profile: arguments.profile.clone(),
            alias: alias.to_owned(),
            pin_file: pin_file.to_owned(),
        });
    }
    if let (Some(profile), Some(alias), Some(pin_file)) = (
        arguments.remote_profile.as_deref(),
        arguments.remote_yubi_alias.as_deref(),
        arguments.remote_pin_file.as_deref(),
    ) {
        requested.push(RequestedUnlock {
            profile: profile.to_owned(),
            alias: alias.to_owned(),
            pin_file: pin_file.to_owned(),
        });
    }
    for unlock in &arguments.unlocks {
        requested.push(parse_unlock(unlock)?);
    }

    // Read each record, then release that profile's lock before opening the
    // next, so no two operation locks are ever held at once here.
    let mut opened = Vec::new();
    for unlock in requested {
        let unlock_session = ProfileSession::open(registry, &unlock.profile)?;
        // Every profile contributing a hardware credential must authorize
        // the responder before its PIN is read or the device is opened.
        unlock_session.profile().require(Capability::Teams)?;
        unlock_session.profile().require(Capability::Federation)?;
        let loaded = credentials.with_checked_session(&unlock_session, |checked| {
            let mut store = EncryptedFileSecretStore::open(
                &checked.paths().credential_store,
                derive_vault_key(&master),
            )?;
            AccountVault::new(&mut store).yubi_account(&unlock.alias)
        })?;
        let pin = read_pin(&unlock.pin_file)?;
        let device = provider.open(&loaded.locator, Some(&pin))?;
        opened.push((unlock.profile, unlock.alias, loaded, device));
    }
    let unlocked_credentials = opened
        .iter()
        .map(|(_, _, loaded, device)| loaded.credential(device.as_ref()))
        .collect::<Vec<_>>();
    let actors = opened
        .iter()
        .zip(&unlocked_credentials)
        .map(|((profile, alias, _, _), credential)| UnlockedYubiActor {
            profile,
            alias,
            credential,
        })
        .collect::<Vec<_>>();

    credentials.with_checked_session(&session, |session| {
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        let report = session.refresh_federated_security_with_unlocked_yubi(
            &arguments.team_alias,
            &actors,
            &mut AccountVault::new(&mut store),
            registry,
            &credentials,
            &master,
        )?;
        let human = match report.deferred.as_deref() {
            Some(reason) => format!("federated security refresh deferred: {reason}"),
            None => "federated security refreshed".to_owned(),
        };
        output(json, &report, &human)
    })
}

fn federation_destination(
    role: FederationRoleArgument,
    visibility: i16,
) -> Result<FederationDestinationRole, Box<dyn std::error::Error>> {
    match role {
        FederationRoleArgument::Member => Ok(FederationDestinationRole::Member { visibility }),
        FederationRoleArgument::Admin | FederationRoleArgument::Owner => {
            Err("federated teams can only hold member roles".into())
        }
    }
}

fn local_team_destination(
    role: FederationRoleArgument,
    visibility: i16,
) -> Result<TeamMemberRole, Box<dyn std::error::Error>> {
    match role {
        FederationRoleArgument::Member => Ok(TeamMemberRole::Member { visibility }),
        FederationRoleArgument::Admin if visibility == 0 => Ok(TeamMemberRole::Admin),
        FederationRoleArgument::Owner if visibility == 0 => Ok(TeamMemberRole::Owner),
        FederationRoleArgument::Admin | FederationRoleArgument::Owner => {
            Err("--visibility applies only to member roles".into())
        }
    }
}

fn read_phrase(path: &Path) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    read_phrase_tokens(path, 17, "backup")
}

fn read_kex_phrase(path: &Path) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    read_phrase_tokens(path, 13, "KEX")
}

fn read_phrase_tokens(
    path: &Path,
    expected_tokens: usize,
    kind: &str,
) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return Err(format!(
            "{kind} phrase file must be a regular non-symlink file of at most 4096 bytes"
        )
        .into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                format!("{kind} phrase file permissions allow group or other access").into(),
            );
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
        return Err(format!("{kind} phrase file exceeds the 4096-byte limit").into());
    }
    if phrase.split_whitespace().count() != expected_tokens {
        return Err(format!(
            "{kind} phrase file does not contain exactly {expected_tokens} tokens"
        )
        .into());
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
        return Err("passphrase file must be a regular file of at most 1024 bytes".into());
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
        return Err("passphrase file contains invalid characters or exceeds 1024 bytes".into());
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
        return Err(
            format!("artifact must be a regular non-symlink file under {maximum} bytes").into(),
        );
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
        return Err(format!("artifact content exceeds maximum size of {maximum} bytes").into());
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
        profile_command(
            &state,
            false,
            ProfileCommand::Verify(ProfileAdd {
                name: "local".to_owned(),
                target: "localhost:4430".to_owned(),
                generation: ProfileGeneration::V019,
                ca_der: Some(state.join("local-ca.der")),
                canary_public_key: None,
                canary_url: None,
            }),
        )
        .unwrap();
        for (target, ca_der) in [
            ("localhost:4431", state.join("local-ca.der")),
            ("localhost:4430", state.join("other-ca.der")),
        ] {
            let error = profile_command(
                &state,
                false,
                ProfileCommand::Verify(ProfileAdd {
                    name: "local".to_owned(),
                    target: target.to_owned(),
                    generation: ProfileGeneration::V019,
                    ca_der: Some(ca_der),
                    canary_public_key: None,
                    canary_url: None,
                }),
            )
            .unwrap_err();
            assert!(error
                .to_string()
                .contains("does not match the required configuration"));
        }
        let error = profile_command(
            &state,
            false,
            ProfileCommand::Verify(ProfileAdd {
                name: "local".to_owned(),
                target: "localhost:4430".to_owned(),
                generation: ProfileGeneration::CurrentProbeOnly,
                ca_der: Some(state.join("local-ca.der")),
                canary_public_key: Some("00".to_owned()),
                canary_url: Some("https://localhost/canary".to_owned()),
            }),
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("does not match the required configuration"));
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

    #[test]
    fn retry_policy_is_available_only_on_enrollment_commands() {
        let parsed = Arguments::try_parse_from([
            "foks-rs",
            "--state-dir",
            "/tmp/foks-cli-test",
            "yubi",
            "create",
            "local",
            "hardware",
            "--username",
            "satoshi",
            "--device-name",
            "primary key",
            "--card-serial",
            "7",
            "--pin-file",
            "/tmp/pin",
            "--retry-puk-file",
            "/tmp/puk",
            "--pin-attempts",
            "5",
            "--puk-attempts",
            "4",
        ])
        .unwrap();
        let Command::Yubi(YubiCommand::Create(create)) = parsed.command else {
            panic!("expected Yubi create command");
        };
        assert_eq!(create.retry.pin_attempts, Some(5));
        assert_eq!(create.retry.puk_attempts, Some(4));

        assert!(Arguments::try_parse_from([
            "foks-rs",
            "--state-dir",
            "/tmp/foks-cli-test",
            "yubi",
            "create",
            "local",
            "hardware",
            "--username",
            "satoshi",
            "--device-name",
            "primary key",
            "--card-serial",
            "7",
            "--pin-file",
            "/tmp/pin",
            "--retry-puk-file",
            "/tmp/puk",
        ])
        .is_err());
        assert!(Arguments::try_parse_from([
            "foks-rs",
            "--state-dir",
            "/tmp/foks-cli-test",
            "yubi",
            "configure-retries",
            "local",
            "hardware",
        ])
        .is_err());
    }

    /// A cascade can reach further than the immediate federation pair, so the
    /// responder has to accept one hardware unlock per profile it visits, not
    /// just a local and a remote one.
    #[test]
    fn refresh_remote_accepts_repeated_profile_qualified_unlocks() {
        assert!(Arguments::try_parse_from([
            "foks-rs",
            "--state-dir",
            "/tmp/foks-cli-test",
            "team",
            "refresh-remote",
            "local",
            "engineering",
        ])
        .is_ok());
        let parsed = Arguments::try_parse_from([
            "foks-rs",
            "--state-dir",
            "/tmp/foks-cli-test",
            "team",
            "refresh-remote",
            "local",
            "engineering",
            "--local-yubi-alias",
            "local-hardware",
            "--local-pin-file",
            "/tmp/local-pin",
            "--unlock",
            "third=third-hardware=/tmp/third-pin",
            "--unlock",
            "fourth=fourth-hardware=/tmp/fourth-pin",
        ])
        .unwrap();
        let Command::Team(TeamCommand::RefreshRemote {
            profile,
            team_alias,
            local_yubi_alias,
            unlocks,
            ..
        }) = parsed.command
        else {
            panic!("expected the federated refresh command");
        };
        assert_eq!(profile, "local");
        assert_eq!(team_alias, "engineering");
        assert_eq!(local_yubi_alias.as_deref(), Some("local-hardware"));
        assert_eq!(
            unlocks,
            [
                "third=third-hardware=/tmp/third-pin",
                "fourth=fourth-hardware=/tmp/fourth-pin",
            ]
        );

        // An unlock that does not name all three fields is a typo, not a
        // partial instruction to guess at.
        assert!(parse_unlock("third=third-hardware").is_err());
        assert!(parse_unlock("=alias=/tmp/pin").is_err());
        let unlock = parse_unlock("third=third-hardware=/tmp/third-pin").unwrap();
        assert_eq!(unlock.profile, "third");
        assert_eq!(unlock.alias, "third-hardware");
        assert_eq!(unlock.pin_file, std::path::Path::new("/tmp/third-pin"));
    }
}
