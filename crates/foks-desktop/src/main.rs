//! Scriptable backend shell for the Tauri desktop's typed agent boundary.
//!
//! Test harness CLI for the Tauri desktop command layer and agent protocol transcripts.

#![forbid(unsafe_code)]

use std::fs::File;
use std::path::PathBuf;

use clap::Parser as _;
use foks_agent_client::AgentClient;
use foks_agent_proto::{
    AccountStoreRef, CredentialBackend, FederationRole, KvEntryMetadata, KvRole, KvStoreRef,
    KvUploadHeader, Operation, ProfileProtocol, ProfileTrust, ResponseResult, SecretString,
    TeamKind, TeamRole, TeamStoreRef, YubiRetryConfiguration,
};
use foks_desktop::{CatalogItem, CatalogStoreRef, KvAccountMutation};

#[derive(clap::Parser)]
#[command(name = "foks-desktop-backend")]
struct Arguments {
    #[arg(long)]
    agent_socket: PathBuf,
    #[arg(long, default_value_t = 15)]
    request_timeout_seconds: u64,
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Status,
    Initialize {
        #[arg(long, value_enum, default_value_t = CredentialBackendArgument::Native)]
        credential_backend: CredentialBackendArgument,
    },
    Profiles,
    CheckProfile {
        name: String,
        probe: String,
        #[command(flatten)]
        trust: ProfileTrustArguments,
    },
    ProfileAdd {
        name: String,
        probe: String,
        #[command(flatten)]
        trust: ProfileTrustArguments,
    },
    ProfileForget {
        name: String,
    },
    ServerStatus {
        profile: String,
    },
    ServerCheck {
        profile: String,
    },
    ResetDescribe {
        profile: String,
    },
    ResetExecute {
        profile: String,
        #[arg(long)]
        token_file: PathBuf,
    },
    Pending {
        profile: String,
    },
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
        #[arg(long)]
        invite_file: Option<PathBuf>,
        #[arg(long, requires = "passphrase_confirmation_file")]
        passphrase_file: Option<PathBuf>,
        #[arg(long, requires = "passphrase_file")]
        passphrase_confirmation_file: Option<PathBuf>,
    },
    ResumeAccount {
        profile: String,
        alias: String,
    },
    BackupPrepare {
        profile: String,
        account_alias: String,
        backup_alias: String,
    },
    BackupCommit {
        profile: String,
        account_alias: String,
        backup_alias: String,
        #[arg(long)]
        phrase_file: PathBuf,
    },
    BackupList {
        profile: String,
        account_alias: String,
    },
    RecoverAccount {
        profile: String,
        target_alias: String,
        #[arg(long)]
        phrase_file: PathBuf,
        #[arg(long)]
        device_name: String,
    },
    ResumeRecovery {
        profile: String,
        target_alias: String,
        #[arg(long)]
        phrase_file: PathBuf,
        #[arg(long)]
        device_name: String,
    },
    DiscoverTeams {
        profile: String,
        account_alias: String,
    },
    TeamCreate {
        profile: String,
        account_alias: String,
        team_alias: String,
        #[arg(long, value_enum)]
        kind: TeamKindArgument,
        #[arg(long, default_value = "")]
        name: String,
    },
    TeamResume {
        profile: String,
        team_alias: String,
    },
    PassphraseSet(PassphraseChange),
    PassphraseChange(PassphraseChange),
    PassphraseVerify(PassphraseVerify),
    Sync {
        profile: String,
        alias: String,
    },
    DevicePairOffer {
        profile: String,
        account_alias: String,
    },
    DevicePairRepublish {
        profile: String,
        account_alias: String,
    },
    DevicePairFinish {
        profile: String,
        account_alias: String,
    },
    DevicePairAccept {
        profile: String,
        target_alias: String,
        #[arg(long)]
        phrase_file: PathBuf,
        #[arg(long)]
        device_name: String,
    },
    DevicePairResumeAccept {
        profile: String,
        target_alias: String,
    },
    DeviceList {
        profile: String,
        account_alias: String,
    },
    DeviceRemove {
        profile: String,
        signer_alias: String,
        device_id: String,
    },
    YubiCards {
        profile: String,
    },
    YubiAccounts {
        profile: String,
    },
    YubiCreate(YubiCreateArguments),
    YubiResume(YubiPinAliasArguments),
    YubiProvision(YubiProvisionArguments),
    YubiSync {
        profile: String,
        alias: String,
        #[arg(long)]
        pin_file: PathBuf,
        #[arg(long)]
        with_federation: bool,
    },
    YubiPassphraseSet(YubiPassphraseChange),
    YubiPassphraseChange(YubiPassphraseChange),
    YubiPassphraseVerify(YubiPassphraseVerify),
    YubiRevoke {
        profile: String,
        yubi_alias: String,
        software_alias: String,
        #[arg(long)]
        confirm_alias: String,
    },
    Kv {
        profile: String,
        alias: String,
    },
    KvRead {
        #[command(flatten)]
        store: KvStoreArguments,
        path: String,
        version: u64,
    },
    KvCreateText {
        #[command(flatten)]
        store: KvStoreArguments,
        path: String,
        #[arg(long)]
        value_file: PathBuf,
    },
    KvCreateLink {
        #[command(flatten)]
        store: KvStoreArguments,
        path: String,
        #[arg(long)]
        target_file: PathBuf,
    },
    KvEditText {
        #[command(flatten)]
        store: KvStoreArguments,
        path: String,
        version: u64,
        #[command(flatten)]
        roles: KvRoleArguments,
        #[arg(long)]
        value_file: PathBuf,
    },
    KvRemove {
        #[command(flatten)]
        store: KvStoreArguments,
        path: String,
        version: u64,
    },
    KvCreateFile {
        #[command(flatten)]
        store: KvStoreArguments,
        path: String,
        #[arg(long)]
        source_file: PathBuf,
    },
    KvReplaceFile {
        #[command(flatten)]
        store: KvStoreArguments,
        path: String,
        version: u64,
        #[command(flatten)]
        roles: KvRoleArguments,
        #[arg(long)]
        source_file: PathBuf,
    },
    Teams {
        profile: String,
    },
    TeamSync {
        profile: String,
        team_alias: String,
    },
    TeamMembers {
        profile: String,
        team_alias: String,
    },
    TeamAddMember {
        profile: String,
        team_alias: String,
        username: String,
        #[arg(long, value_enum, default_value_t = FederationRoleArgument::Member)]
        role: FederationRoleArgument,
        #[arg(long, default_value_t = 0)]
        visibility: i16,
    },
    TeamResumeAddMember {
        profile: String,
        team_alias: String,
        username: String,
    },
    TeamDemoteMember {
        profile: String,
        team_alias: String,
        party_id_hex: String,
        #[arg(long, value_enum, default_value_t = FederationRoleArgument::Member)]
        role: FederationRoleArgument,
        #[arg(long, default_value_t = 0)]
        visibility: i16,
    },
    TeamRemoveMember {
        profile: String,
        team_alias: String,
        party_id_hex: String,
    },
    TeamResumeMemberEdit {
        profile: String,
        team_alias: String,
    },
    TeamAdmitRemote {
        local_profile: String,
        local_team_alias: String,
        remote_profile: String,
        remote_team_alias: String,
        #[arg(long, value_enum, default_value_t = FederationRoleArgument::Member)]
        role: FederationRoleArgument,
        #[arg(long, default_value_t = 0)]
        visibility: i16,
    },
    TeamListRemote {
        profile: String,
        team_alias: String,
    },
    RunJobs {
        profile: String,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum FederationRoleArgument {
    Member,
    Admin,
    Owner,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum TeamKindArgument {
    Named,
    Adhoc,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum CredentialBackendArgument {
    Native,
    PrivateFile,
}

#[derive(clap::Args)]
struct ProfileTrustArguments {
    /// Trust one explicit DER-encoded root certificate instead of Web PKI.
    #[arg(long)]
    certificate_der: Option<PathBuf>,
}

#[derive(clap::Args)]
struct KvStoreArguments {
    profile: String,
    account_alias: String,
    #[arg(long, requires = "team_id")]
    team_alias: Option<String>,
    #[arg(long, requires = "team_alias")]
    team_id: Option<String>,
}

#[derive(clap::Args)]
struct KvRoleArguments {
    #[arg(long, value_parser = parse_kv_role)]
    read_role: KvRole,
    #[arg(long, value_parser = parse_kv_role)]
    write_role: KvRole,
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

#[derive(clap::Args)]
struct YubiCreateArguments {
    profile: String,
    alias: String,
    #[arg(long)]
    username: String,
    #[arg(long)]
    device_name: String,
    #[arg(long, default_value = "")]
    email: String,
    #[arg(long)]
    invite_file: Option<PathBuf>,
    #[arg(long, requires = "passphrase_confirmation_file")]
    passphrase_file: Option<PathBuf>,
    #[arg(long, requires = "passphrase_file")]
    passphrase_confirmation_file: Option<PathBuf>,
    #[arg(long)]
    card_serial: u32,
    #[arg(long, default_value_t = 0x82)]
    signing_slot: u8,
    #[arg(long, default_value_t = 0x83)]
    pq_slot: u8,
    #[arg(long)]
    pin_file: PathBuf,
    #[arg(long)]
    puk_file: PathBuf,
    #[arg(long, default_value_t = 3)]
    pin_attempts: u8,
    #[arg(long, default_value_t = 3)]
    puk_attempts: u8,
}

#[derive(clap::Args)]
struct YubiPinAliasArguments {
    profile: String,
    alias: String,
    #[arg(long)]
    pin_file: PathBuf,
}

#[derive(clap::Args)]
struct YubiProvisionArguments {
    profile: String,
    source_alias: String,
    target_alias: String,
    #[arg(long)]
    device_name: String,
    #[arg(long)]
    card_serial: u32,
    #[arg(long, default_value_t = 0x82)]
    signing_slot: u8,
    #[arg(long, default_value_t = 0x83)]
    pq_slot: u8,
    #[arg(long)]
    pin_file: PathBuf,
    #[arg(long)]
    puk_file: PathBuf,
    #[arg(long, default_value_t = 3)]
    pin_attempts: u8,
    #[arg(long, default_value_t = 3)]
    puk_attempts: u8,
}

#[derive(clap::Args)]
struct YubiPassphraseChange {
    profile: String,
    alias: String,
    #[arg(long)]
    pin_file: PathBuf,
    #[arg(long)]
    passphrase_file: PathBuf,
    #[arg(long)]
    passphrase_confirmation_file: PathBuf,
}

#[derive(clap::Args)]
struct YubiPassphraseVerify {
    profile: String,
    alias: String,
    #[arg(long)]
    pin_file: PathBuf,
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
    let call = backend_call_for_command(arguments.command)?;
    let mut client = AgentClient::new(arguments.agent_socket);
    client.set_timeout(std::time::Duration::from_secs(
        arguments.request_timeout_seconds,
    ))?;
    let response = match call {
        BackendCall::Operation(operation) => client.call(operation)?,
        BackendCall::Upload { header, mut source } => client.put_kv_stream(header, &mut source)?,
    };
    match response.result {
        ResponseResult::Success { value } => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        ResponseResult::Error { code, message, .. } => {
            Err(format!("agent returned {code:?}: {message}").into())
        }
    }
}

enum BackendCall {
    Operation(Operation),
    Upload {
        header: KvUploadHeader,
        source: File,
    },
}

#[cfg(test)]
fn operation_for_command(command: Command) -> Result<Operation, Box<dyn std::error::Error>> {
    match backend_call_for_command(command)? {
        BackendCall::Operation(operation) => Ok(operation),
        BackendCall::Upload { .. } => Err("file uploads require streaming transfer".into()),
    }
}

fn backend_call_for_command(command: Command) -> Result<BackendCall, Box<dyn std::error::Error>> {
    let operation = match command {
        Command::Status => Operation::AgentStatus,
        Command::Initialize { credential_backend } => Operation::InitializeState {
            backend: match credential_backend {
                CredentialBackendArgument::Native => CredentialBackend::Native,
                CredentialBackendArgument::PrivateFile => CredentialBackend::PrivateFile,
            },
        },
        Command::Profiles => Operation::ListProfiles,
        Command::CheckProfile { name, probe, trust } => Operation::CheckAndAddProfile {
            name,
            probe,
            protocol: ProfileProtocol::V019,
            trust: trust.profile_trust()?,
        },
        Command::ProfileAdd { name, probe, trust } => Operation::AddProfile {
            name,
            probe,
            protocol: ProfileProtocol::V019,
            trust: trust.profile_trust()?,
        },
        Command::ProfileForget { name } => Operation::RemoveProfile { name },
        Command::ServerStatus { profile } => Operation::DescribeServerStatus { profile },
        Command::ServerCheck { profile } => Operation::Probe { profile },
        Command::ResetDescribe { profile } => Operation::DescribeResetHardState { profile },
        Command::ResetExecute {
            profile,
            token_file,
        } => Operation::ResetHardState {
            profile,
            token: read_secret(&token_file)?,
        },
        Command::Pending { profile } => Operation::ListPendingOperations { profile },
        Command::Accounts { profile } => Operation::ListAccounts { profile },
        Command::CreateAccount {
            profile,
            alias,
            username,
            device_name,
            email,
            invite_file,
            passphrase_file,
            passphrase_confirmation_file,
        } => Operation::CreateAccount {
            profile,
            alias,
            username,
            device_name,
            email,
            invite: match invite_file {
                Some(path) => read_invite(&path)?,
                None => SecretString::new(""),
            },
            passphrase: match (passphrase_file, passphrase_confirmation_file) {
                (Some(path), Some(confirmation)) => {
                    Some(read_confirmed_secret(&path, &confirmation)?)
                }
                (None, None) => None,
                _ => return Err("both passphrase files are required".into()),
            },
        },
        Command::ResumeAccount { profile, alias } => Operation::ResumeAccount { profile, alias },
        Command::BackupPrepare {
            profile,
            account_alias,
            backup_alias,
        } => Operation::PrepareOwnerBackup {
            profile,
            account_alias,
            backup_alias,
        },
        Command::BackupCommit {
            profile,
            account_alias,
            backup_alias,
            phrase_file,
        } => Operation::CommitOwnerBackup {
            profile,
            account_alias,
            backup_alias,
            phrase: read_secret(&phrase_file)?,
        },
        Command::BackupList {
            profile,
            account_alias,
        } => Operation::ListBackupEnrollments {
            profile,
            account_alias,
        },
        Command::RecoverAccount {
            profile,
            target_alias,
            phrase_file,
            device_name,
        } => Operation::RecoverOwnerAccount {
            profile,
            target_alias,
            phrase: read_secret(&phrase_file)?,
            device_name,
            serial: positive_device_serial()?,
        },
        Command::ResumeRecovery {
            profile,
            target_alias,
            phrase_file,
            device_name,
        } => Operation::ResumeOwnerRecovery {
            profile,
            target_alias,
            phrase: read_secret(&phrase_file)?,
            device_name,
        },
        Command::DiscoverTeams {
            profile,
            account_alias,
        } => Operation::DiscoverTeams {
            profile,
            account_alias,
        },
        Command::TeamCreate {
            profile,
            account_alias,
            team_alias,
            kind,
            name,
        } => {
            let (name, kind) = match kind {
                TeamKindArgument::Named if name.trim().is_empty() => {
                    return Err("a named group requires --name".into());
                }
                TeamKindArgument::Named => (name, TeamKind::Named),
                TeamKindArgument::Adhoc if !name.is_empty() => {
                    return Err("an ad-hoc group does not accept --name".into());
                }
                TeamKindArgument::Adhoc => (String::new(), TeamKind::AdHoc),
            };
            Operation::CreateTeam {
                profile,
                account_alias,
                team_alias,
                name,
                kind,
            }
        }
        Command::TeamResume {
            profile,
            team_alias,
        } => Operation::ResumeTeamCreation {
            profile,
            team_alias,
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
            current: None,
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
        Command::DevicePairOffer {
            profile,
            account_alias,
        } => Operation::StartDevicePairing {
            profile,
            account_alias,
        },
        Command::DevicePairRepublish {
            profile,
            account_alias,
        } => Operation::RepublishDevicePairing {
            profile,
            account_alias,
        },
        Command::DevicePairFinish {
            profile,
            account_alias,
        } => Operation::FinishDevicePairing {
            profile,
            account_alias,
        },
        Command::DevicePairAccept {
            profile,
            target_alias,
            phrase_file,
            device_name,
        } => Operation::AcceptDevicePairing {
            profile,
            target_alias,
            device_name,
            serial: positive_device_serial()?,
            phrase: read_secret(&phrase_file)?,
        },
        Command::DevicePairResumeAccept {
            profile,
            target_alias,
        } => Operation::ResumeDevicePairingAcceptance {
            profile,
            target_alias,
        },
        Command::DeviceList {
            profile,
            account_alias,
        } => Operation::ListDevices {
            profile,
            alias: account_alias,
        },
        Command::DeviceRemove {
            profile,
            signer_alias,
            device_id,
        } => Operation::RemoveDevice {
            profile,
            signer_alias,
            device_id,
        },
        Command::YubiCards { profile } => Operation::ListYubiCards { profile },
        Command::YubiAccounts { profile } => Operation::ListYubiAccounts { profile },
        Command::YubiCreate(arguments) => {
            validate_yubi_configuration(
                arguments.card_serial,
                arguments.signing_slot,
                arguments.pq_slot,
                arguments.pin_attempts,
                arguments.puk_attempts,
            )?;
            Operation::CreateYubiAccount {
                profile: arguments.profile,
                alias: arguments.alias,
                username: arguments.username,
                device_name: arguments.device_name,
                email: arguments.email,
                invite: match arguments.invite_file {
                    Some(path) => read_invite(&path)?,
                    None => SecretString::new(""),
                },
                passphrase: match (
                    arguments.passphrase_file,
                    arguments.passphrase_confirmation_file,
                ) {
                    (Some(path), Some(confirmation)) => {
                        Some(read_confirmed_secret(&path, &confirmation)?)
                    }
                    (None, None) => None,
                    _ => return Err("both passphrase files are required".into()),
                },
                card_serial: arguments.card_serial,
                signing_slot: arguments.signing_slot,
                pq_slot: arguments.pq_slot,
                pin: read_secret(&arguments.pin_file)?,
                retry_configuration: Some(YubiRetryConfiguration {
                    puk: read_secret(&arguments.puk_file)?,
                    pin_attempts: arguments.pin_attempts,
                    puk_attempts: arguments.puk_attempts,
                }),
            }
        }
        Command::YubiResume(arguments) => Operation::ResumeYubiAccount {
            profile: arguments.profile,
            alias: arguments.alias,
            pin: read_secret(&arguments.pin_file)?,
        },
        Command::YubiProvision(arguments) => {
            validate_yubi_configuration(
                arguments.card_serial,
                arguments.signing_slot,
                arguments.pq_slot,
                arguments.pin_attempts,
                arguments.puk_attempts,
            )?;
            Operation::ProvisionYubiDevice {
                profile: arguments.profile,
                source_alias: arguments.source_alias,
                target_alias: arguments.target_alias,
                device_name: arguments.device_name,
                serial: positive_device_serial()?,
                card_serial: arguments.card_serial,
                signing_slot: arguments.signing_slot,
                pq_slot: arguments.pq_slot,
                pin: read_secret(&arguments.pin_file)?,
                retry_configuration: Some(YubiRetryConfiguration {
                    puk: read_secret(&arguments.puk_file)?,
                    pin_attempts: arguments.pin_attempts,
                    puk_attempts: arguments.puk_attempts,
                }),
            }
        }
        Command::YubiSync {
            profile,
            alias,
            pin_file,
            with_federation,
        } => Operation::SyncYubiAccount {
            profile,
            alias,
            pin: read_secret(&pin_file)?,
            with_federation,
        },
        Command::YubiPassphraseSet(arguments) => Operation::SetYubiPassphrase {
            profile: arguments.profile,
            alias: arguments.alias,
            pin: read_secret(&arguments.pin_file)?,
            passphrase: read_confirmed_secret(
                &arguments.passphrase_file,
                &arguments.passphrase_confirmation_file,
            )?,
        },
        Command::YubiPassphraseChange(arguments) => Operation::ChangeYubiPassphrase {
            profile: arguments.profile,
            alias: arguments.alias,
            pin: read_secret(&arguments.pin_file)?,
            passphrase: read_confirmed_secret(
                &arguments.passphrase_file,
                &arguments.passphrase_confirmation_file,
            )?,
        },
        Command::YubiPassphraseVerify(arguments) => Operation::VerifyYubiPassphrase {
            profile: arguments.profile,
            alias: arguments.alias,
            pin: read_secret(&arguments.pin_file)?,
            passphrase: read_secret(&arguments.passphrase_file)?,
        },
        Command::YubiRevoke {
            profile,
            yubi_alias,
            software_alias,
            confirm_alias,
        } => {
            if confirm_alias != yubi_alias {
                return Err("--confirm-alias must exactly match the Yubi alias".into());
            }
            Operation::RevokeYubiDevice {
                profile,
                yubi_alias,
                software_alias,
            }
        }
        Command::Kv { profile, alias } => Operation::ListKv {
            store: AccountStoreRef {
                profile,
                account_alias: alias,
            },
            cursor: None,
            limit: 100,
            fresh: false,
        },
        Command::KvRead {
            store,
            path,
            version,
        } => Operation::ReadKv {
            store: store.kv_store_ref()?,
            path: checked_kv_path(path)?,
            version,
        },
        Command::KvCreateText {
            store,
            path,
            value_file,
        } => {
            let store = store.catalog_store_ref()?;
            let path = checked_kv_path(path)?;
            let mut content =
                read_private_bytes(&value_file, foks_desktop::MAXIMUM_INLINE_KV_BYTES as u64)?;
            match foks_desktop::create_kv_file_mutation(
                &store,
                &path,
                std::mem::take(&mut *content),
            )? {
                KvAccountMutation::Inline(operation) => operation,
                KvAccountMutation::Stream { .. } => {
                    return Err(
                        "Text value exceeds maximum supported size for inline storage".into(),
                    );
                }
            }
        }
        Command::KvCreateLink {
            store,
            path,
            target_file,
        } => {
            let path = checked_kv_path(path)?;
            let target = read_secret(&target_file)?;
            foks_desktop::create_kv_symlink_operation(
                &store.catalog_store_ref()?,
                &path,
                target.expose(),
            )?
        }
        Command::KvEditText {
            store,
            path,
            version,
            roles,
            value_file,
        } => {
            let item = catalog_item(
                store.catalog_store_ref()?,
                checked_kv_path(path)?,
                "small-file",
                version,
                roles,
            );
            let mut content =
                read_private_bytes(&value_file, foks_desktop::MAXIMUM_INLINE_KV_BYTES as u64)?;
            match foks_desktop::edit_kv_file_mutation(&item, std::mem::take(&mut *content))? {
                KvAccountMutation::Inline(operation) => operation,
                KvAccountMutation::Stream { .. } => {
                    return Err(
                        "Text value exceeds maximum supported size for inline storage".into(),
                    );
                }
            }
        }
        Command::KvRemove {
            store,
            path,
            version,
        } => {
            let item = catalog_item(
                store.catalog_store_ref()?,
                checked_kv_path(path)?,
                "small-file",
                version,
                KvRoleArguments::owner(),
            );
            foks_desktop::remove_kv_operation(&item, false)?
        }
        Command::KvCreateFile {
            store,
            path,
            source_file,
        } => {
            let path = checked_kv_path(path)?;
            let (source, total_length) = open_private_upload(&source_file)?;
            let header = foks_desktop::create_kv_file_upload(
                &store.catalog_store_ref()?,
                &path,
                total_length,
            )?;
            return Ok(BackendCall::Upload { header, source });
        }
        Command::KvReplaceFile {
            store,
            path,
            version,
            roles,
            source_file,
        } => {
            let path = checked_kv_path(path)?;
            let (source, total_length) = open_private_upload(&source_file)?;
            let item = catalog_item(store.catalog_store_ref()?, path, "file", version, roles);
            let header = foks_desktop::edit_kv_file_upload(&item, total_length)?;
            return Ok(BackendCall::Upload { header, source });
        }
        Command::Teams { profile } => Operation::ListTeams { profile },
        Command::TeamSync {
            profile,
            team_alias,
        } => Operation::SyncTeam {
            profile,
            team_alias,
        },
        Command::TeamMembers {
            profile,
            team_alias,
        } => Operation::ListTeamMembers {
            profile,
            team_alias,
        },
        Command::TeamAddMember {
            profile,
            team_alias,
            username,
            role,
            visibility,
        } => Operation::AddTeamMember {
            profile,
            team_alias,
            username,
            role: desktop_team_role(role),
            visibility,
        },
        Command::TeamResumeAddMember {
            profile,
            team_alias,
            username,
        } => Operation::ResumeTeamMemberAddition {
            profile,
            team_alias,
            username,
        },
        Command::TeamDemoteMember {
            profile,
            team_alias,
            party_id_hex,
            role,
            visibility,
        } => Operation::DemoteTeamMember {
            profile,
            team_alias,
            party_id_hex: checked_user_party_id(party_id_hex)?,
            role: desktop_team_role(role),
            visibility,
        },
        Command::TeamRemoveMember {
            profile,
            team_alias,
            party_id_hex,
        } => Operation::RemoveTeamMember {
            profile,
            team_alias,
            party_id_hex: checked_user_party_id(party_id_hex)?,
        },
        Command::TeamResumeMemberEdit {
            profile,
            team_alias,
        } => Operation::ResumeTeamMemberEdit {
            profile,
            team_alias,
        },
        Command::TeamAdmitRemote {
            local_profile,
            local_team_alias,
            remote_profile,
            remote_team_alias,
            role,
            visibility,
        } => Operation::AdmitFederatedTeam {
            local_profile,
            local_team_alias,
            remote_profile,
            remote_team_alias,
            role: match role {
                FederationRoleArgument::Member => FederationRole::Member,
                FederationRoleArgument::Admin => FederationRole::Admin,
                FederationRoleArgument::Owner => FederationRole::Owner,
            },
            visibility,
        },
        Command::TeamListRemote {
            profile,
            team_alias,
        } => Operation::ListFederatedTeams {
            profile,
            team_alias,
        },
        Command::RunJobs { profile } => Operation::RunDueJobs { profile },
    };
    Ok(BackendCall::Operation(operation))
}

impl KvStoreArguments {
    fn kv_store_ref(self) -> Result<KvStoreRef, Box<dyn std::error::Error>> {
        let profile = checked_local_name(self.profile, "profile")?;
        let account_alias = checked_local_name(self.account_alias, "account alias")?;
        match (self.team_alias, self.team_id) {
            (None, None) => Ok(KvStoreRef::Account(AccountStoreRef {
                profile,
                account_alias,
            })),
            (Some(team_alias), Some(team_id)) => {
                let team_alias = checked_local_name(team_alias, "team alias")?;
                if !valid_team_id_hex(&team_id) {
                    return Err(
                        "Group ID must be a valid lowercase hexadecimal group identifier".into(),
                    );
                }
                Ok(KvStoreRef::Team(TeamStoreRef {
                    profile,
                    account_alias,
                    team_alias,
                    team_id,
                }))
            }
            _ => Err("both --team-alias and --team-id are required to select a team store".into()),
        }
    }

    fn catalog_store_ref(self) -> Result<CatalogStoreRef, Box<dyn std::error::Error>> {
        Ok(match self.kv_store_ref()? {
            KvStoreRef::Account(store) => CatalogStoreRef::Account(store),
            KvStoreRef::Team(store) => CatalogStoreRef::Team(store),
        })
    }
}

impl ProfileTrustArguments {
    #[cfg(test)]
    fn web_pki() -> Self {
        Self {
            certificate_der: None,
        }
    }

    fn profile_trust(self) -> Result<ProfileTrust, Box<dyn std::error::Error>> {
        let Some(path) = self.certificate_der else {
            return Ok(ProfileTrust::WebPki);
        };
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > 1024 * 1024
        {
            return Err("Certificate file must be a non-empty regular DER file under 1 MiB".into());
        }
        let path = std::fs::canonicalize(path)?;
        let path = path
            .to_str()
            .ok_or("certificate DER path is not valid UTF-8")?
            .to_owned();
        Ok(ProfileTrust::CertificateDer { path })
    }
}

impl KvRoleArguments {
    fn owner() -> Self {
        Self {
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
        }
    }
}

fn checked_local_name(value: String, label: &str) -> Result<String, Box<dyn std::error::Error>> {
    if (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(value)
    } else {
        Err(format!(
            "{label} must be between 1 and 64 alphanumeric characters, hyphens, or underscores"
        )
        .into())
    }
}

fn valid_team_id_hex(value: &str) -> bool {
    value.len() == 66
        && (value.starts_with("03") || value.starts_with("14"))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn checked_kv_path(path: String) -> Result<String, Box<dyn std::error::Error>> {
    if path.starts_with('/') && path != "/" && !path.contains(['\0', '\r', '\n']) {
        Ok(path)
    } else {
        Err("item path must start with '/' and cannot be empty or contain newlines".into())
    }
}

fn parse_kv_role(value: &str) -> Result<KvRole, String> {
    match value {
        "owner" => Ok(KvRole::Owner),
        "admin" => Ok(KvRole::Admin),
        _ => {
            let visibility = value
                .strip_prefix("member:")
                .ok_or_else(|| "role must be owner, admin, or member:<visibility>".to_owned())?
                .parse::<i16>()
                .map_err(|_| "member visibility must be a signed 16-bit integer".to_owned())?;
            Ok(KvRole::Member { visibility })
        }
    }
}

fn catalog_item(
    store: CatalogStoreRef,
    path: String,
    node_type: &str,
    version: u64,
    roles: KvRoleArguments,
) -> CatalogItem {
    CatalogItem {
        store,
        metadata: KvEntryMetadata {
            path,
            node_type: node_type.to_owned(),
            version,
            size: None,
            read_role: roles.read_role,
            write_role: roles.write_role,
        },
    }
}

fn read_private_bytes(
    path: &std::path::Path,
    maximum: u64,
) -> Result<zeroize::Zeroizing<Vec<u8>>, Box<dyn std::error::Error>> {
    use std::io::Read as _;

    let (mut file, length) = open_private_file(path, Some(maximum), "value")?;
    let capacity =
        usize::try_from(length).map_err(|_| "value file size exceeds system address space")?;
    let mut value = zeroize::Zeroizing::new(Vec::with_capacity(capacity));
    file.read_to_end(&mut value)?;
    if value.len() != capacity {
        return Err("value file was modified concurrently while being read".into());
    }
    Ok(value)
}

fn open_private_upload(path: &std::path::Path) -> Result<(File, u64), Box<dyn std::error::Error>> {
    open_private_file(path, None, "upload source")
}

fn open_private_file(
    path: &std::path::Path,
    maximum: Option<u64>,
    label: &str,
) -> Result<(File, u64), Box<dyn std::error::Error>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || maximum.is_some_and(|maximum| metadata.len() > maximum) {
        return Err(
            format!("{label} is not a valid regular file within supported size limits").into(),
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!("{label} permissions allow group or other access").into());
        }
    }
    Ok((file, metadata.len()))
}

fn checked_user_party_id(party_id_hex: String) -> Result<String, &'static str> {
    if party_id_hex.len() == 66
        && party_id_hex.starts_with("01")
        && party_id_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(party_id_hex)
    } else {
        Err("party ID must be a 66-character lowercase hex string starting with 01")
    }
}

fn positive_device_serial() -> Result<u64, Box<dyn std::error::Error>> {
    for _ in 0..4 {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).map_err(|_| "system entropy source unavailable")?;
        let serial = u64::from_le_bytes(bytes);
        if serial != 0 {
            return Ok(serial);
        }
    }
    Err("failed to generate non-zero random device serial".into())
}

fn validate_yubi_configuration(
    card_serial: u32,
    signing_slot: u8,
    pq_slot: u8,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let retired = |slot: u8| (0x82..=0x95).contains(&slot);
    if card_serial == 0 {
        return Err("hardware key card serial must be positive".into());
    }
    if signing_slot == pq_slot || !retired(signing_slot) || !retired(pq_slot) {
        return Err("hardware key slots must be distinct valid PIV key slots (0x82-0x95)".into());
    }
    if pin_attempts == 0 || puk_attempts == 0 {
        return Err("hardware key retry counts must be positive".into());
    }
    Ok(())
}

fn desktop_team_role(role: FederationRoleArgument) -> TeamRole {
    match role {
        FederationRoleArgument::Member => TeamRole::Member,
        FederationRoleArgument::Admin => TeamRole::Admin,
        FederationRoleArgument::Owner => TeamRole::Owner,
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
        return Err("secret file is not a bounded regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("secret file permissions allow group or other access".into());
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
        return Err("secret must be one nonempty line of at most 1024 bytes".into());
    }
    Ok(SecretString::new(value.as_str()))
}

fn read_invite(path: &std::path::Path) -> Result<SecretString, Box<dyn std::error::Error>> {
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
    Ok(SecretString::new(value.as_str()))
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
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

        let invite = directory.path().join("invite");
        std::fs::write(&invite, "s.desktop-invite\n").unwrap();
        std::fs::set_permissions(&invite, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_invite(&invite).unwrap().expose(), "s.desktop-invite");
        std::fs::set_permissions(&invite, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_invite(&invite).is_err());

        let invite_link = directory.path().join("invite-link");
        symlink(&invite, &invite_link).unwrap();
        assert!(read_invite(&invite_link).is_err());
    }

    #[test]
    fn backend_bootstrap_and_testkit_trust_are_explicit_and_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let certificate = directory.path().join("testkit-ca.der");
        std::fs::write(&certificate, [0x30, 0x01, 0x00]).unwrap();

        let initialized = Arguments::try_parse_from([
            "foks-desktop-backend",
            "--agent-socket",
            "/tmp/agent.sock",
            "initialize",
            "--credential-backend",
            "private-file",
        ])
        .unwrap();
        assert_eq!(
            operation_for_command(initialized.command).unwrap(),
            Operation::InitializeState {
                backend: CredentialBackend::PrivateFile,
            }
        );

        let checked = Arguments::try_parse_from([
            OsString::from("foks-desktop-backend"),
            OsString::from("--agent-socket"),
            OsString::from("/tmp/agent.sock"),
            OsString::from("check-profile"),
            OsString::from("testkit"),
            OsString::from("127.0.0.1:4433"),
            OsString::from("--certificate-der"),
            certificate.clone().into_os_string(),
        ])
        .unwrap();
        assert!(matches!(
            operation_for_command(checked.command).unwrap(),
            Operation::CheckAndAddProfile {
                trust: ProfileTrust::CertificateDer { path },
                ..
            } if path == std::fs::canonicalize(&certificate).unwrap().to_str().unwrap()
        ));

        let added = Arguments::try_parse_from([
            OsString::from("foks-desktop-backend"),
            OsString::from("--agent-socket"),
            OsString::from("/tmp/agent.sock"),
            OsString::from("profile-add"),
            OsString::from("testkit"),
            OsString::from("127.0.0.1:4433"),
            OsString::from("--certificate-der"),
            certificate.clone().into_os_string(),
        ])
        .unwrap();
        assert!(matches!(
            operation_for_command(added.command).unwrap(),
            Operation::AddProfile {
                trust: ProfileTrust::CertificateDer { .. },
                ..
            }
        ));

        let default = Arguments::try_parse_from([
            "foks-desktop-backend",
            "--agent-socket",
            "/tmp/agent.sock",
            "profile-add",
            "work",
            "foks.example",
        ])
        .unwrap();
        assert!(matches!(
            operation_for_command(default.command).unwrap(),
            Operation::AddProfile {
                trust: ProfileTrust::WebPki,
                ..
            }
        ));

        let oversized = directory.path().join("too-large.der");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(1024 * 1024 + 1)
            .unwrap();
        assert!(ProfileTrustArguments {
            certificate_der: Some(oversized)
        }
        .profile_trust()
        .is_err());
        let link = directory.path().join("ca-link.der");
        symlink(&certificate, &link).unwrap();
        assert!(ProfileTrustArguments {
            certificate_der: Some(link)
        }
        .profile_trust()
        .is_err());
    }

    #[test]
    fn backend_kv_selector_requires_complete_authenticated_team_identity() {
        let read = |team_alias: Option<&str>, team_id: Option<&str>| {
            operation_for_command(Command::KvRead {
                store: KvStoreArguments {
                    profile: "work".to_owned(),
                    account_alias: "personal".to_owned(),
                    team_alias: team_alias.map(str::to_owned),
                    team_id: team_id.map(str::to_owned),
                },
                path: "/password".to_owned(),
                version: 9,
            })
        };
        assert!(matches!(
            read(None, None).unwrap(),
            Operation::ReadKv {
                store: KvStoreRef::Account(_),
                version: 9,
                ..
            }
        ));
        assert!(read(Some("household"), None).is_err());
        assert!(read(None, Some("14bad")).is_err());
        assert!(read(Some("household"), Some(&format!("03{}", "A".repeat(64)))).is_err());
        assert!(read(Some("household"), Some(&format!("01{}", "0".repeat(64)))).is_err());
        assert!(matches!(
            read(Some("household"), Some(&format!("14{}", "0".repeat(64)))).unwrap(),
            Operation::ReadKv {
                store: KvStoreRef::Team(_),
                version: 9,
                ..
            }
        ));

        let missing_roles = Arguments::try_parse_from([
            "foks-desktop-backend",
            "--agent-socket",
            "/tmp/agent.sock",
            "kv-edit-text",
            "work",
            "personal",
            "/password",
            "9",
            "--value-file",
            "/tmp/private-value",
        ]);
        assert!(
            missing_roles.is_err(),
            "guarded edits must carry both catalog roles"
        );
    }

    #[test]
    fn backend_kv_plaintext_files_are_private_bounded_and_not_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let value = directory.path().join("value");
        std::fs::write(&value, b"secret bytes").unwrap();
        std::fs::set_permissions(&value, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(&*read_private_bytes(&value, 32).unwrap(), b"secret bytes");

        assert!(read_private_bytes(&value, 4).is_err());
        std::fs::set_permissions(&value, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private_bytes(&value, 32).is_err());
        std::fs::set_permissions(&value, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("value-link");
        symlink(&value, &link).unwrap();
        assert!(read_private_bytes(&link, 32).is_err());
        assert!(open_private_upload(&link).is_err());
    }

    #[test]
    fn first_run_has_a_process_safe_command_and_resume_for_every_step() {
        let directory = tempfile::tempdir().unwrap();
        let phrase = directory.path().join("recovery-phrase");
        std::fs::write(
            &phrase,
            "abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon\n",
        )
        .unwrap();
        std::fs::set_permissions(&phrase, std::fs::Permissions::from_mode(0o600)).unwrap();

        let commands = vec![
            Command::Initialize {
                credential_backend: CredentialBackendArgument::Native,
            },
            Command::CheckProfile {
                name: "work".to_owned(),
                probe: "foks.example".to_owned(),
                trust: ProfileTrustArguments::web_pki(),
            },
            Command::Pending {
                profile: "work".to_owned(),
            },
            Command::CreateAccount {
                profile: "work".to_owned(),
                alias: "personal".to_owned(),
                username: "sol".to_owned(),
                device_name: "Sol's laptop".to_owned(),
                email: String::new(),
                invite_file: None,
                passphrase_file: None,
                passphrase_confirmation_file: None,
            },
            Command::ResumeAccount {
                profile: "work".to_owned(),
                alias: "personal".to_owned(),
            },
            Command::BackupPrepare {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
                backup_alias: "paper".to_owned(),
            },
            Command::BackupCommit {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
                backup_alias: "paper".to_owned(),
                phrase_file: phrase.clone(),
            },
            Command::RecoverAccount {
                profile: "work".to_owned(),
                target_alias: "recovered".to_owned(),
                phrase_file: phrase.clone(),
                device_name: "Sol's laptop".to_owned(),
            },
            Command::ResumeRecovery {
                profile: "work".to_owned(),
                target_alias: "recovered".to_owned(),
                phrase_file: phrase,
                device_name: "Sol's laptop".to_owned(),
            },
            Command::DiscoverTeams {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
            },
            Command::TeamCreate {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "engineering".to_owned(),
                kind: TeamKindArgument::Named,
                name: "Engineering".to_owned(),
            },
            Command::TeamResume {
                profile: "work".to_owned(),
                team_alias: "engineering".to_owned(),
            },
        ];
        let operations = commands
            .into_iter()
            // Each conversion is independent: the backend persists no
            // renderer-owned workflow state between process invocations.
            .map(|command| operation_for_command(command).unwrap())
            .collect::<Vec<_>>();

        assert!(matches!(
            operations.as_slice(),
            [
                Operation::InitializeState { .. },
                Operation::CheckAndAddProfile { .. },
                Operation::ListPendingOperations { .. },
                Operation::CreateAccount { .. },
                Operation::ResumeAccount { .. },
                Operation::PrepareOwnerBackup { .. },
                Operation::CommitOwnerBackup { .. },
                Operation::RecoverOwnerAccount { serial, .. },
                Operation::ResumeOwnerRecovery { .. },
                Operation::DiscoverTeams { .. },
                Operation::CreateTeam { .. },
                Operation::ResumeTeamCreation { .. },
            ] if *serial > 0
        ));
    }

    #[test]
    fn backend_maps_status_devices_pairing_and_one_use_reset() {
        let directory = tempfile::tempdir().unwrap();
        let secret = directory.path().join("secret");
        std::fs::write(&secret, "one-use-token\n").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let phrase = directory.path().join("pairing-phrase");
        std::fs::write(
            &phrase,
            "cage 32 advice 4 letter 128 avoid 16 acoustic 2 doctor 64 amount\n",
        )
        .unwrap();
        std::fs::set_permissions(&phrase, std::fs::Permissions::from_mode(0o600)).unwrap();

        let commands = vec![
            Command::ProfileAdd {
                name: "partner".to_owned(),
                probe: "foks.partner.example".to_owned(),
                trust: ProfileTrustArguments::web_pki(),
            },
            Command::ProfileForget {
                name: "partner".to_owned(),
            },
            Command::ServerStatus {
                profile: "work".to_owned(),
            },
            Command::ServerCheck {
                profile: "work".to_owned(),
            },
            Command::BackupList {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
            },
            Command::DeviceList {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
            },
            Command::DeviceRemove {
                profile: "work".to_owned(),
                signer_alias: "personal".to_owned(),
                device_id: "04".repeat(33),
            },
            Command::DevicePairOffer {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
            },
            Command::DevicePairRepublish {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
            },
            Command::DevicePairFinish {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
            },
            Command::DevicePairAccept {
                profile: "work".to_owned(),
                target_alias: "paired".to_owned(),
                phrase_file: phrase,
                device_name: "New device".to_owned(),
            },
            Command::DevicePairResumeAccept {
                profile: "work".to_owned(),
                target_alias: "paired".to_owned(),
            },
            Command::ResetDescribe {
                profile: "work".to_owned(),
            },
            Command::ResetExecute {
                profile: "work".to_owned(),
                token_file: secret,
            },
        ];
        let operations = commands
            .into_iter()
            .map(|command| operation_for_command(command).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            operations.as_slice(),
            [
                Operation::AddProfile { .. },
                Operation::RemoveProfile { .. },
                Operation::DescribeServerStatus { .. },
                Operation::Probe { .. },
                Operation::ListBackupEnrollments { .. },
                Operation::ListDevices { .. },
                Operation::RemoveDevice { .. },
                Operation::StartDevicePairing { .. },
                Operation::RepublishDevicePairing { .. },
                Operation::FinishDevicePairing { .. },
                Operation::AcceptDevicePairing { serial, .. },
                Operation::ResumeDevicePairingAcceptance { .. },
                Operation::DescribeResetHardState { .. },
                Operation::ResetHardState { token, .. },
            ] if *serial > 0 && token.expose() == "one-use-token"
        ));
    }

    #[test]
    fn security_key_transcript_uses_private_files_and_generated_device_serials() {
        let directory = tempfile::tempdir().unwrap();
        let pin = directory.path().join("pin");
        let puk = directory.path().join("puk");
        let passphrase = directory.path().join("passphrase");
        let confirmation = directory.path().join("passphrase-confirmation");
        for (path, value) in [
            (&pin, "123456\n"),
            (&puk, "12345678\n"),
            (&passphrase, "correct horse battery staple\n"),
            (&confirmation, "correct horse battery staple\n"),
        ] {
            std::fs::write(path, value).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let commands = vec![
            Command::YubiCards {
                profile: "work".to_owned(),
            },
            Command::YubiAccounts {
                profile: "work".to_owned(),
            },
            Command::YubiCreate(YubiCreateArguments {
                profile: "work".to_owned(),
                alias: "work_key".to_owned(),
                username: "satoshi".to_owned(),
                device_name: "YubiKey 42".to_owned(),
                email: String::new(),
                invite_file: None,
                passphrase_file: None,
                passphrase_confirmation_file: None,
                card_serial: 42,
                signing_slot: 0x82,
                pq_slot: 0x83,
                pin_file: pin.clone(),
                puk_file: puk.clone(),
                pin_attempts: 3,
                puk_attempts: 3,
            }),
            Command::YubiResume(YubiPinAliasArguments {
                profile: "work".to_owned(),
                alias: "work_key".to_owned(),
                pin_file: pin.clone(),
            }),
            Command::YubiProvision(YubiProvisionArguments {
                profile: "work".to_owned(),
                source_alias: "personal".to_owned(),
                target_alias: "spare_key".to_owned(),
                device_name: "YubiKey 43".to_owned(),
                card_serial: 43,
                signing_slot: 0x82,
                pq_slot: 0x83,
                pin_file: pin.clone(),
                puk_file: puk,
                pin_attempts: 3,
                puk_attempts: 3,
            }),
            Command::YubiSync {
                profile: "work".to_owned(),
                alias: "work_key".to_owned(),
                pin_file: pin.clone(),
                with_federation: true,
            },
            Command::YubiPassphraseSet(YubiPassphraseChange {
                profile: "work".to_owned(),
                alias: "work_key".to_owned(),
                pin_file: pin.clone(),
                passphrase_file: passphrase.clone(),
                passphrase_confirmation_file: confirmation,
            }),
            Command::YubiPassphraseVerify(YubiPassphraseVerify {
                profile: "work".to_owned(),
                alias: "work_key".to_owned(),
                pin_file: pin,
                passphrase_file: passphrase,
            }),
            Command::YubiRevoke {
                profile: "work".to_owned(),
                yubi_alias: "work_key".to_owned(),
                software_alias: "personal".to_owned(),
                confirm_alias: "work_key".to_owned(),
            },
        ];
        let operations = commands
            .into_iter()
            .map(|command| operation_for_command(command).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            operations.as_slice(),
            [
                Operation::ListYubiCards { .. },
                Operation::ListYubiAccounts { .. },
                Operation::CreateYubiAccount { retry_configuration: Some(create_retry), .. },
                Operation::ResumeYubiAccount { .. },
                Operation::ProvisionYubiDevice { serial, retry_configuration: Some(provision_retry), .. },
                Operation::SyncYubiAccount { with_federation: true, .. },
                Operation::SetYubiPassphrase { .. },
                Operation::VerifyYubiPassphrase { .. },
                Operation::RevokeYubiDevice { .. },
            ] if *serial > 0
                && create_retry.pin_attempts == 3
                && provision_retry.puk_attempts == 3
        ));
    }

    #[test]
    fn backend_rejects_invalid_security_key_hardware_configuration() {
        assert!(validate_yubi_configuration(0, 0x82, 0x83, 3, 3).is_err());
        assert!(validate_yubi_configuration(42, 0x82, 0x82, 3, 3).is_err());
        assert!(validate_yubi_configuration(42, 0x81, 0x83, 3, 3).is_err());
        assert!(validate_yubi_configuration(42, 0x82, 0x83, 0, 3).is_err());
        assert!(validate_yubi_configuration(42, 0x82, 0x83, 3, 3).is_ok());
        assert!(operation_for_command(Command::YubiRevoke {
            profile: "work".to_owned(),
            yubi_alias: "work_key".to_owned(),
            software_alias: "personal".to_owned(),
            confirm_alias: "other".to_owned(),
        })
        .is_err());
    }

    #[test]
    fn backend_member_changes_require_an_authenticated_user_party_id() {
        assert!(operation_for_command(Command::TeamRemoveMember {
            profile: "work".to_owned(),
            team_alias: "engineering".to_owned(),
            party_id_hex: "satoshi".to_owned(),
        })
        .is_err());
        assert!(matches!(
            operation_for_command(Command::TeamRemoveMember {
                profile: "work".to_owned(),
                team_alias: "engineering".to_owned(),
                party_id_hex:
                    "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
            })
            .unwrap(),
            Operation::RemoveTeamMember { party_id_hex, .. }
                if party_id_hex == "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
    }
}
