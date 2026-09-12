use std::io::Read as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser as _;
use foks_server::keys::{read_root_key_file, read_secret_file};
use foks_server::{SessionLimits, StandaloneConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

#[derive(clap::Parser)]
#[command(name = "foks-server", about = "Standalone SQLite FOKS v0.1.9 server")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Creates a non-overwriting standalone installation layout.
    Init(InitArguments),
    /// Runs the standalone network service.
    Serve(Box<ServeArguments>),
    /// Runs an installation from its versioned TOML configuration.
    ServeConfig {
        #[arg(long)]
        config: PathBuf,
    },
    /// Validates an installation configuration and its key/certificate artifacts.
    ConfigCheck {
        #[arg(long)]
        config: PathBuf,
    },
    /// Reports offline database/bootstrap/integrity status.
    Status {
        #[arg(long)]
        config: PathBuf,
    },
    /// Writes a pinned-client import bundle after the server has bootstrapped.
    ClientBootstrap {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Creates a validated online backup of an installation.
    Backup(BackupArguments),
    /// Installs a validated backup into an empty installation.
    Restore(RestoreArguments),
    /// Atomically rewraps installation keys under a new operator root key.
    RotateOperatorRoot(RotateOperatorRootArguments),
    /// Publishes a new host signing key while retaining the old signer.
    BeginHostKeyRotation {
        #[arg(long)]
        config: PathBuf,
    },
    /// Revokes the prior host signing key after the observation interval.
    CompleteHostKeyRotation(CompleteHostKeyRotationArguments),
    /// Selects a new local capability-encryption key generation.
    RotateCapabilityKey {
        #[arg(long)]
        config: PathBuf,
    },
    /// Erases retired capability keys after every dependent token has expired.
    RetireCapabilityKeys {
        #[arg(long)]
        config: PathBuf,
    },
    #[command(subcommand)]
    Invite(InviteCommand),
    #[command(subcommand)]
    Oidc(OidcCommand),
    #[command(subcommand)]
    Admin(AdminCommand),
}

#[derive(clap::Subcommand)]
enum OidcCommand {
    MigrationStatus {
        #[arg(long)]
        config: PathBuf,
    },
    MigrationStart {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        confirm_host: String,
    },
    MigrationEnforce {
        #[command(flatten)]
        expected: OidcExpected,
        #[arg(long)]
        expected_unlinked: u64,
        #[arg(long)]
        accept_lockout: bool,
    },
    ProviderReenable(OidcExpected),
    ProviderReplace {
        #[command(flatten)]
        expected: OidcExpected,
        #[arg(long)]
        accept_provider_hash: String,
    },
}
#[derive(clap::Args)]
struct OidcExpected {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    confirm_host: String,
    #[arg(long)]
    expected_rollout_id: String,
    #[arg(long)]
    expected_provider_hash: String,
    #[arg(long)]
    expected_policy_revision: u64,
    #[arg(long)]
    expected_authorization_epoch: u64,
}
fn oidc_hex<const N: usize>(value: &str) -> Result<[u8; N], Box<dyn std::error::Error>> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("expected canonical lowercase hex".into());
    }
    let mut out = [0; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}
fn oidc_command(command: OidcCommand) -> Result<(), Box<dyn std::error::Error>> {
    use foks_server::sso::operator;
    use foks_server_db::SsoPolicyTransition as T;
    let (expected, transition) = match command {
        OidcCommand::MigrationStatus { config } => {
            let config = foks_server::installation::load_config(config)?;
            if let Some(status) = operator::status(&config)? {
                print_oidc_policy(&status.policy);
                println!(
                    "cohort={} linked={} unlinked={}",
                    status.cohort, status.linked, status.unlinked
                );
            } else {
                println!("policy=never-activated");
            }
            return Ok(());
        }
        OidcCommand::MigrationStart {
            config,
            confirm_host,
        } => {
            let config = foks_server::installation::load_config(config)?;
            print_oidc_policy(&operator::start(&config, &oidc_hex(&confirm_host)?)?);
            return Ok(());
        }
        OidcCommand::MigrationEnforce {
            expected,
            expected_unlinked,
            accept_lockout,
        } => (
            expected,
            T::Enforce {
                expected_unlinked,
                accept_lockout,
            },
        ),
        OidcCommand::ProviderReenable(expected) => (expected, T::Reenable),
        OidcCommand::ProviderReplace {
            expected,
            accept_provider_hash,
        } => {
            let hash = oidc_hex(&accept_provider_hash)?;
            (
                expected,
                T::ReplaceProvider {
                    config_hash: hash,
                    issuer: String::new(),
                },
            )
        }
    };
    let config = foks_server::installation::load_config(&expected.config)?;
    let mut observed = operator::status(&config)?
        .ok_or("OIDC policy has not been activated")?
        .policy;
    if observed.host != oidc_hex::<33>(&expected.confirm_host)?
        || observed.rollout_id != oidc_hex::<16>(&expected.expected_rollout_id)?
        || observed.config_hash != oidc_hex::<32>(&expected.expected_provider_hash)?
        || observed.revision != expected.expected_policy_revision
        || observed.authorization_epoch != expected.expected_authorization_epoch
    {
        return Err("OIDC policy changed; obtain a fresh status and approval".into());
    }
    let transition = match transition {
        T::ReplaceProvider { config_hash, .. } => T::ReplaceProvider {
            config_hash,
            issuer: observed.issuer.clone(),
        },
        other => other,
    };
    observed = operator::transition(&config, &observed, &transition)?;
    print_oidc_policy(&observed);
    Ok(())
}
fn print_oidc_policy(p: &foks_server_db::SsoPolicy) {
    println!("host={} rollout_id={} provider_hash={} mode={:?} fence={:?} policy_revision={} authorization_epoch={}",encode_hex(&p.host),encode_hex(&p.rollout_id),encode_hex(&p.config_hash),p.mode,p.fence,p.revision,p.authorization_epoch);
}

#[derive(clap::Subcommand)]
enum InviteCommand {
    /// Issues a signup invite while the service is stopped and prints the code once.
    Issue(InviteIssueArguments),
    /// Disables a signup invite while the service is stopped.
    Disable {
        #[arg(long)]
        config: PathBuf,
        code: String,
    },
    /// Lists invite metadata; plaintext codes are not stored in SQLite.
    List {
        #[arg(long)]
        config: PathBuf,
    },
    /// Selects whether signup requires a code.
    Policy {
        #[arg(long)]
        config: PathBuf,
        #[arg(value_enum)]
        regime: InviteRegimeArgument,
    },
}

#[derive(clap::Args)]
struct InviteIssueArguments {
    #[arg(long)]
    config: PathBuf,
    /// Omit for a generated standard single-use code; set for a named multi-use code.
    #[arg(long)]
    multiuse_code: Option<String>,
    /// Maximum redemptions for a multi-use code. Omit for no fixed maximum.
    #[arg(long)]
    max_uses: Option<u64>,
    #[arg(long)]
    expires_in_seconds: Option<u64>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum InviteRegimeArgument {
    Required,
    Optional,
}

#[derive(clap::Args)]
struct InitArguments {
    #[arg(long)]
    directory: PathBuf,
    #[arg(long)]
    canonical_name: String,
    #[arg(long, default_value = "127.0.0.1:4430")]
    probe_address: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:4431")]
    public_address: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:4432")]
    authenticated_address: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:9090")]
    management_address: SocketAddr,
}

#[derive(clap::Args)]
struct ServeArguments {
    #[arg(skip)]
    web_admin: Option<foks_server::web_admin::WebAdminConfig>,
    #[arg(long, default_value = "")]
    vhost_management_host: String,
    #[arg(skip)]
    oidc: Option<foks_server::sso::OidcOperatorConfig>,
    #[arg(long)]
    canonical_name: String,
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    key_directory: PathBuf,
    #[arg(long)]
    root_key_file: PathBuf,
    #[arg(long, required = true, num_args = 1..=8)]
    probe_certificate_der: Vec<PathBuf>,
    #[arg(long)]
    probe_private_key_der: PathBuf,
    #[arg(long, default_value = "127.0.0.1:4430")]
    probe_address: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:4431")]
    public_address: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:4432")]
    authenticated_address: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:9090")]
    management_address: SocketAddr,
    #[arg(long, default_value_t = 60)]
    ttl_seconds: i64,
    #[arg(long, default_value_t = 4)]
    worker_threads: usize,
    #[arg(long, default_value_t = 32)]
    maximum_read_connections: usize,
    #[arg(long, default_value_t = 256)]
    maximum_active_connections: usize,
    #[arg(long, default_value_t = 32)]
    maximum_pending_connections: usize,
    #[arg(long, default_value_t = 64)]
    maximum_in_flight_requests: usize,
    #[arg(long, default_value_t = 30)]
    request_timeout_seconds: u64,
    #[arg(long, default_value_t = 64)]
    maximum_pending_writes: usize,
    #[arg(long, default_value_t = 512)]
    connection_rate_burst: u32,
    #[arg(long, default_value_t = 256)]
    connections_per_second: u32,
    #[arg(long, default_value_t = 2_000)]
    request_rate_burst: u32,
    #[arg(long, default_value_t = 1_000)]
    requests_per_second: u32,
    #[arg(long, default_value_t = 4_096)]
    maximum_rate_limit_ips: usize,
    #[arg(long)]
    automatic_backup_directory: Option<PathBuf>,
    #[arg(long, default_value_t = 86_400)]
    automatic_backup_interval_seconds: u64,
    #[arg(long, default_value_t = 7)]
    automatic_backup_retain: usize,
}

#[derive(clap::Args)]
struct BackupArguments {
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    key_directory: PathBuf,
    #[arg(long)]
    root_key_file: PathBuf,
    #[arg(long)]
    destination: PathBuf,
}

#[derive(clap::Args)]
struct RestoreArguments {
    #[arg(long)]
    backup_directory: PathBuf,
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    key_directory: PathBuf,
}

#[derive(clap::Args)]
struct RotateOperatorRootArguments {
    #[arg(long)]
    key_directory: PathBuf,
    #[arg(long)]
    old_root_key_file: PathBuf,
    #[arg(long)]
    new_root_key_file: PathBuf,
}

#[derive(clap::Args)]
struct CompleteHostKeyRotationArguments {
    #[arg(long)]
    config: PathBuf,
    /// The 32-hex-character operation ID returned by begin-host-key-rotation.
    #[arg(long)]
    operation_id: String,
    /// Add-key hostchain sequence observed by an independently syncing client.
    #[arg(long)]
    observed_addition_seqno: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Arguments::parse().command {
        Command::Init(arguments) => initialize(arguments),
        Command::Serve(arguments) => serve(*arguments),
        Command::ServeConfig { config } => serve_config(config),
        Command::ConfigCheck { config } => config_check(config),
        Command::Status { config } => status(config),
        Command::ClientBootstrap { config, output } => client_bootstrap(config, output),
        Command::Backup(arguments) => backup(arguments),
        Command::Restore(arguments) => restore(arguments),
        Command::RotateOperatorRoot(arguments) => rotate_operator_root(arguments),
        Command::BeginHostKeyRotation { config } => rotate_host_key(config),
        Command::CompleteHostKeyRotation(arguments) => complete_host_key_rotation(arguments),
        Command::RotateCapabilityKey { config } => rotate_capability_key(config),
        Command::RetireCapabilityKeys { config } => retire_capability_keys(config),
        Command::Invite(command) => invite_command(command),
        Command::Oidc(command) => oidc_command(command),
        Command::Admin(command) => admin_command(command),
    }
}

fn invite_command(command: InviteCommand) -> Result<(), Box<dyn std::error::Error>> {
    let now = now_microseconds()?;
    match command {
        InviteCommand::Issue(arguments) => {
            let config = foks_server::installation::load_config(arguments.config)?;
            foks_server::installation::validate_artifacts(&config)?;
            let expires_at = arguments
                .expires_in_seconds
                .map(|seconds| {
                    seconds
                        .checked_mul(1_000_000)
                        .and_then(|duration| now.checked_add(duration))
                        .ok_or("invite expiry overflows the supported timestamp")
                })
                .transpose()?;
            let issued = match arguments.multiuse_code {
                Some(code) => foks_server::invites::issue_multiuse_invite(
                    &config.database,
                    foks_server_db::Config::default(),
                    &code,
                    arguments.max_uses,
                    expires_at,
                    now,
                )?,
                None if arguments.max_uses.is_none() => {
                    foks_server::invites::issue_standard_invite(
                        &config.database,
                        foks_server_db::Config::default(),
                        expires_at,
                        now,
                    )?
                }
                None => return Err("--max-uses requires --multiuse-code".into()),
            };
            println!("{}", issued.code);
            Ok(())
        }
        InviteCommand::Disable { config, code } => {
            let config = foks_server::installation::load_config(config)?;
            foks_server::installation::validate_artifacts(&config)?;
            if !foks_server::invites::disable_invite(
                &config.database,
                foks_server_db::Config::default(),
                &code,
                now,
            )? {
                return Err("invite is unknown or already disabled".into());
            }
            println!("invite disabled");
            Ok(())
        }
        InviteCommand::List { config } => {
            let config = foks_server::installation::load_config(config)?;
            foks_server::installation::validate_artifacts(&config)?;
            for invite in foks_server::invites::list_invites(
                &config.database,
                foks_server_db::Config::default(),
            )? {
                println!(
                    "id={} kind={:?} active={} uses={}/{} expires_at={}",
                    encode_hex(&invite.invite_id),
                    invite.kind,
                    invite.active,
                    invite.use_count,
                    invite
                        .max_uses
                        .map_or_else(|| "unlimited".to_owned(), |value| value.to_string()),
                    invite
                        .expires_at
                        .map_or_else(|| "never".to_owned(), |value| value.to_string()),
                );
            }
            Ok(())
        }
        InviteCommand::Policy { config, regime } => {
            let config = foks_server::installation::load_config(config)?;
            foks_server::installation::validate_artifacts(&config)?;
            let regime = match regime {
                InviteRegimeArgument::Required => foks_server_db::InviteRegime::Required,
                InviteRegimeArgument::Optional => foks_server_db::InviteRegime::Optional,
            };
            foks_server::invites::set_invite_regime(
                &config.database,
                foks_server_db::Config::default(),
                regime,
            )?;
            println!("signup invite policy set to {regime:?}");
            Ok(())
        }
    }
}

fn now_microseconds() -> Result<u64, Box<dyn std::error::Error>> {
    Ok(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros(),
    )?)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn initialize(arguments: InitArguments) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::initialize(
        arguments.directory,
        &arguments.canonical_name,
        arguments.probe_address,
        arguments.public_address,
        arguments.authenticated_address,
        arguments.management_address,
    )?;
    println!(
        "initialized FOKS server configuration: {}",
        config.display()
    );
    Ok(())
}

fn serve_config(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(path)?;
    foks_server::installation::validate_artifacts(&config)?;
    serve(ServeArguments {
        oidc: config.oidc,
        web_admin: config.web_admin,
        vhost_management_host: config.vhost_management_host,
        canonical_name: config.canonical_name,
        database: config.database,
        key_directory: config.key_directory,
        root_key_file: config.root_key_file,
        probe_certificate_der: vec![config.probe_certificate_der],
        probe_private_key_der: config.probe_private_key_der,
        probe_address: config.probe_address,
        public_address: config.public_address,
        authenticated_address: config.authenticated_address,
        management_address: config.management_address,
        ttl_seconds: 60,
        worker_threads: 4,
        maximum_read_connections: 32,
        maximum_active_connections: 256,
        maximum_pending_connections: 32,
        maximum_in_flight_requests: 64,
        request_timeout_seconds: 30,
        maximum_pending_writes: 64,
        connection_rate_burst: 512,
        connections_per_second: 256,
        request_rate_burst: 2_000,
        requests_per_second: 1_000,
        maximum_rate_limit_ips: 4_096,
        automatic_backup_directory: Some(config.backup_directory),
        automatic_backup_interval_seconds: config.backup_interval_seconds,
        automatic_backup_retain: config.backup_retain,
    })
}

fn config_check(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(path)?;
    foks_server::installation::validate_artifacts(&config)?;
    if let Some(admin) = &config.web_admin {
        match std::net::TcpListener::bind(admin.listen) {
            Ok(listener) => {
                drop(listener);
                println!("admin_listener_bind=available");
            }
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                println!("admin_listener_bind=busy (stop the running service before rebinding)")
            }
            Err(error) => return Err(error.into()),
        }
        if config.database.exists() {
            let count = foks_server_db::ReadDatabase::open(&config.database, Default::default())?
                .admin_operator_count()?;
            println!("admin_active_operators={count}");
            if count == 0 {
                println!("No host operator configured; only self-session pages are available.");
            }
        }
    }
    println!("FOKS server configuration and artifacts are valid");
    if let Some(oidc) = &config.oidc {
        println!(
            "configured_provider_hash={}",
            encode_hex(&oidc.configured_fingerprint()?)
        );
    }
    Ok(())
}

fn status(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(path)?;
    let status = foks_server::installation::status(&config)?;
    println!(
        "initialized={} canonical_name={} host_id={} database_bytes={} wal_bytes={} integrity={}",
        status.initialized,
        status.canonical_name,
        status.host_id_hex.as_deref().unwrap_or("unavailable"),
        status.database_bytes,
        status.wal_bytes,
        status
            .integrity_ok
            .map_or("not-run".to_owned(), |value| value.to_string()),
    );
    Ok(())
}

fn client_bootstrap(
    config_path: PathBuf,
    output: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(config_path)?;
    foks_server::installation::write_client_bootstrap(&config, &output)?;
    println!("wrote FOKS client bootstrap: {}", output.display());
    Ok(())
}

fn serve(arguments: ServeArguments) -> Result<(), Box<dyn std::error::Error>> {
    let root_key = read_root_key_file(&arguments.root_key_file)?;
    let probe_tls = probe_tls(&arguments)?;
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let now_microseconds =
        u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;
    let server = foks_server::start_standalone(StandaloneConfig {
        vhost_management_host: arguments.vhost_management_host,
        web_admin: arguments.web_admin,
        oidc: arguments
            .oidc
            .clone()
            .map(|config| (config, foks_oidc::NetworkPolicy::default())),
        database_path: arguments.database,
        key_directory: arguments.key_directory,
        root_key,
        canonical_name: arguments.canonical_name,
        probe_address: arguments.probe_address,
        public_address: arguments.public_address,
        authenticated_address: arguments.authenticated_address,
        management_address: arguments.management_address,
        probe_tls,
        database: foks_server_db::Config::default(),
        limits: SessionLimits {
            worker_threads: arguments.worker_threads,
            maximum_read_connections: arguments.maximum_read_connections,
            maximum_active_connections: arguments.maximum_active_connections,
            maximum_pending_connections: arguments.maximum_pending_connections,
            maximum_in_flight_requests: arguments.maximum_in_flight_requests,
            request_timeout: std::time::Duration::from_secs(arguments.request_timeout_seconds),
            ..SessionLimits::default()
        },
        rate_limits: foks_server::RateLimitConfig {
            connection_burst: arguments.connection_rate_burst,
            connections_per_second: arguments.connections_per_second,
            request_burst: arguments.request_rate_burst,
            requests_per_second: arguments.requests_per_second,
            maximum_tracked_ips: arguments.maximum_rate_limit_ips,
        },
        backup: arguments
            .automatic_backup_directory
            .map(|directory| foks_server::BackupSchedule {
                directory,
                interval: std::time::Duration::from_secs(
                    arguments.automatic_backup_interval_seconds,
                ),
                retain: arguments.automatic_backup_retain,
            }),
        clock: Arc::new(foks_server_db::SystemClock),
        entropy: Arc::new(foks_server::OsEntropy),
        session_faults: None,
        diagnostics: Some(Arc::new(foks_server::StderrSessionDiagnostics)),
        ttl_seconds: arguments.ttl_seconds,
        now_microseconds,
        maximum_pending_writes: arguments.maximum_pending_writes,
    })?;
    let addresses = server.addresses();
    eprintln!(
        "FOKS server ready: probe={}, public={}, authenticated={}, management={}",
        addresses.probe,
        addresses.public_services,
        addresses.authenticated,
        server.management_address(),
    );
    let _ = signals.forever().next();
    server.shutdown()?;
    Ok(())
}

fn backup(arguments: BackupArguments) -> Result<(), Box<dyn std::error::Error>> {
    let root_key = read_root_key_file(&arguments.root_key_file)?;
    foks_server::backup_standalone_installation(
        arguments.database,
        arguments.key_directory,
        *root_key,
        arguments.destination,
        foks_server_db::Config::default(),
    )?;
    Ok(())
}

fn restore(arguments: RestoreArguments) -> Result<(), Box<dyn std::error::Error>> {
    foks_server::restore_backup(
        &foks_server::BackupArtifacts {
            database: arguments.backup_directory.join("foks-server.sqlite"),
            key_directory: arguments.backup_directory.join("keys"),
            key_manifest: arguments.backup_directory.join("key-manifest.txt"),
        },
        arguments.database,
        arguments.key_directory,
        foks_server_db::Config::default(),
    )?;
    Ok(())
}

fn rotate_operator_root(
    arguments: RotateOperatorRootArguments,
) -> Result<(), Box<dyn std::error::Error>> {
    let old_root_key = read_root_key_file(arguments.old_root_key_file)?;
    let new_root_key = read_root_key_file(arguments.new_root_key_file)?;
    foks_server::keys::DirectoryKeyProvider::rotate_operator_root(
        arguments.key_directory,
        *old_root_key,
        *new_root_key,
    )?;
    Ok(())
}

fn rotate_host_key(config_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(config_path)?;
    foks_server::installation::validate_artifacts(&config)?;
    let root_key = read_root_key_file(&config.root_key_file)?;
    let provider = foks_server::keys::DirectoryKeyProvider::open_for_rotation(
        &config.key_directory,
        *root_key,
    )?;
    let mut database =
        foks_server_db::Database::open(&config.database, foks_server_db::Config::default())?;
    let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;
    let state = foks_server::host::begin_host_key_rotation(&mut database, &provider, now)?;
    print_host_rotation(&state);
    Ok(())
}

fn complete_host_key_rotation(
    arguments: CompleteHostKeyRotationArguments,
) -> Result<(), Box<dyn std::error::Error>> {
    let operation_id = decode_operation_id(&arguments.operation_id)?;
    let config = foks_server::installation::load_config(arguments.config)?;
    foks_server::installation::validate_artifacts(&config)?;
    let root_key = read_root_key_file(&config.root_key_file)?;
    let provider = foks_server::keys::DirectoryKeyProvider::open_for_rotation(
        &config.key_directory,
        *root_key,
    )?;
    let mut database =
        foks_server_db::Database::open(&config.database, foks_server_db::Config::default())?;
    let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;
    let state = foks_server::host::complete_host_key_rotation(
        &mut database,
        &provider,
        operation_id,
        foks_server::host::HostKeyRotationObservation {
            add_link_seqno: arguments.observed_addition_seqno,
        },
        now,
    )?;
    print_host_rotation(&state);
    Ok(())
}

fn print_host_rotation(state: &foks_server::host::HostKeyRotationState) {
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    println!(
        "host-key rotation operation={} phase={:?} old_generation={} new_generation={} \
         old_public_entity={} new_public_entity={} add_seqno={:?} revoke_seqno={:?} \
         observation_not_before_micros={:?}",
        hex(&state.operation_id),
        state.phase,
        hex(&state.old_generation_id),
        hex(&state.new_generation_id),
        hex(&state.old_public_entity_id),
        hex(&state.new_public_entity_id),
        state.add_link_seqno,
        state.revoke_link_seqno,
        state.observation_not_before,
    );
}

fn rotate_capability_key(config_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(config_path)?;
    foks_server::installation::validate_artifacts(&config)?;
    let mut database = foks_server_db::Database::open_existing(
        &config.database,
        foks_server_db::Config::default(),
    )?;
    let root_key = read_root_key_file(&config.root_key_file)?;
    let provider = foks_server::keys::DirectoryKeyProvider::open_for_rotation(
        &config.key_directory,
        *root_key,
    )?;
    let state =
        foks_server::keys::rotate_capability_key(&mut database, &provider, now_microseconds()?)?;
    print_capability_rotation(&state);
    Ok(())
}

fn retire_capability_keys(config_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(config_path)?;
    foks_server::installation::validate_artifacts(&config)?;
    let mut database = foks_server_db::Database::open_existing(
        &config.database,
        foks_server_db::Config::default(),
    )?;
    let root_key = read_root_key_file(&config.root_key_file)?;
    let provider = foks_server::keys::DirectoryKeyProvider::open_for_rotation(
        &config.key_directory,
        *root_key,
    )?;
    let state =
        foks_server::keys::retire_capability_keys(&mut database, &provider, now_microseconds()?)?;
    print_capability_rotation(&state);
    Ok(())
}

fn print_capability_rotation(state: &foks_server::keys::CapabilityKeyRotationState) {
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    println!(
        "capability-key active_generation={} retiring_generations={}",
        hex(&state.active_generation_id),
        state
            .retiring_generation_ids
            .iter()
            .map(|generation| hex(generation))
            .collect::<Vec<_>>()
            .join(",")
    );
}

fn decode_operation_id(encoded: &str) -> Result<[u8; 16], Box<dyn std::error::Error>> {
    if encoded.len() != 32 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("operation ID must contain exactly 32 hexadecimal characters".into());
    }
    let mut decoded = [0; 16];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16)?;
    }
    Ok(decoded)
}

fn probe_tls(
    arguments: &ServeArguments,
) -> Result<Arc<rustls::ServerConfig>, Box<dyn std::error::Error>> {
    let certificates = arguments
        .probe_certificate_der
        .iter()
        .map(|path| read_public_certificate(path))
        .collect::<Result<Vec<_>, _>>()?;
    let private_key = read_secret_file(&arguments.probe_private_key_der, 64 * 1024)?;
    let private_key = PrivateKeyDer::try_from(private_key.as_slice())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?
        .clone_key();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Ok(Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)?,
    ))
}

fn read_public_certificate(path: &Path) -> std::io::Result<CertificateDer<'static>> {
    const MAXIMUM_CERTIFICATE_BYTES: u64 = 1024 * 1024;
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAXIMUM_CERTIFICATE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAXIMUM_CERTIFICATE_BYTES as usize {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "probe certificate DER has an invalid size",
        ));
    }
    Ok(CertificateDer::from(bytes))
}

#[derive(clap::Subcommand)]
enum AdminCommand {
    Grant(AdminGrantArguments),
    Revoke(AdminGrantArguments),
    List {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        after: Option<String>,
    },
}
#[derive(clap::Args)]
struct AdminGrantArguments {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    uid: String,
    #[arg(long)]
    reason: String,
}
fn admin_command(command: AdminCommand) -> Result<(), Box<dyn std::error::Error>> {
    use foks_server::web_admin::operator;
    let (args, active) = match command {
        AdminCommand::List { config, after } => {
            let config = foks_server::installation::load_config(config)?;
            let after = after.as_deref().map(oidc_hex::<33>).transpose()?;
            let grants =
                operator::grants(&config, after.as_ref().map(|v| v.as_slice()).unwrap_or(&[]))?;
            if grants.is_empty() {
                println!("No host operator grants in this page.");
            }
            for g in grants {
                println!(
                    "host={} uid={} account={} active={} revision={}",
                    encode_hex(&g.host),
                    encode_hex(&g.uid),
                    g.username,
                    g.active,
                    g.revision
                );
            }
            return Ok(());
        }
        AdminCommand::Grant(args) => (args, true),
        AdminCommand::Revoke(args) => (args, false),
    };
    let config = foks_server::installation::load_config(args.config)?;
    let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;
    let g = operator::set_grant(&config, &oidc_hex(&args.uid)?, active, &args.reason, now)?;
    println!(
        "host={} uid={} account={} active={} revision={}",
        encode_hex(&g.host),
        encode_hex(&g.uid),
        g.username,
        g.active,
        g.revision
    );
    Ok(())
}
