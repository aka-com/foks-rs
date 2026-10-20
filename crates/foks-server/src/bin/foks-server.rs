use std::io::Read as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Parser as _;
use foks_server::installation::LimitsConfig;
use foks_server::keys::{read_root_key_file, read_secret_file};
use foks_server::{
    ChunkCapacityConfig, RateLimitConfig, RunningStandaloneServer, StandaloneConfig,
};
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
        /// Prints the configuration that serve-config would run, including
        /// the defaults of omitted keys and tables, as TOML on standard
        /// output; the check report then goes to standard error.
        #[arg(long)]
        print_effective: bool,
    },
    /// Reports offline database/bootstrap/integrity status.
    Status {
        #[arg(long)]
        config: PathBuf,
    },
    /// Reports stored usage against the permanent and storage limits, the
    /// largest KV namespaces and the teams with the most realtime message
    /// bytes. Reads a snapshot, so the server may be running.
    Usage {
        #[arg(long)]
        config: PathBuf,
        /// Prints one JSON object instead of `key=value` lines.
        #[arg(long)]
        json: bool,
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
    /// Moves inline encrypted chunks to the configured store; stop the server first.
    ExternalizeChunks {
        #[arg(long)]
        config: PathBuf,
    },
    /// Verifies and binds legacy external chunks; stop the server first.
    AdoptChunkStore {
        #[arg(long)]
        config: PathBuf,
    },
    /// Offline report-only object inventory; never deletes objects.
    InventoryChunkStore {
        #[arg(long)]
        config: PathBuf,
        /// Restart enumeration after correcting missing objects or a stale cursor.
        #[arg(long)]
        restart: bool,
    },
    /// Reclaim one offline batch of unreferenced, terminal filesystem objects.
    /// S3 backends remain report-only until mutation termination is qualified.
    CollectChunkStore {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, default_value_t = 128)]
        maximum_objects: usize,
        #[arg(long,default_value_t=1024*1024*1024)]
        maximum_bytes: u64,
        #[arg(long)]
        dry_run: bool,
    },
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
    #[command(subcommand)]
    Waitlist(WaitlistCommand),
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
    println!("host={} rollout_id={} provider_hash={} mode={:?} blocked_reason={:?} policy_revision={} authorization_epoch={}",encode_hex(&p.host),encode_hex(&p.rollout_id),encode_hex(&p.config_hash),p.mode,p.blocked_reason,p.revision,p.authorization_epoch);
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
    #[arg(long, default_value_t = ChunkCapacityConfig::default().maximum_bytes)]
    maximum_chunk_bytes: u64,
    #[arg(long, default_value_t = ChunkCapacityConfig::default().maximum_objects)]
    maximum_chunk_objects: u64,
    #[arg(long, default_value_t = ChunkCapacityConfig::default().maximum_unresolved_attempts)]
    maximum_chunk_unresolved_attempts: u64,
    #[arg(long, default_value_t = ChunkCapacityConfig::default().maximum_uncertain_bytes)]
    maximum_chunk_uncertain_bytes: u64,
    #[arg(skip)]
    chunk_store: Option<foks_server::chunks::ChunkStoreConfig>,
    /// TOML chunk-store settings (the contents of an installation's chunk_store table).
    #[arg(long)]
    chunk_store_config: Option<PathBuf>,
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
    #[arg(long, default_value_t = LimitsConfig::default().worker_threads)]
    worker_threads: usize,
    #[arg(long, default_value_t = LimitsConfig::default().maximum_read_connections)]
    maximum_read_connections: usize,
    #[arg(long, default_value_t = LimitsConfig::default().maximum_active_connections)]
    maximum_active_connections: usize,
    #[arg(long, default_value_t = LimitsConfig::default().maximum_pending_connections)]
    maximum_pending_connections: usize,
    /// Anonymous KEX receive limit, capped at half the active connections
    /// (zero disables).
    #[arg(long, default_value_t = LimitsConfig::default().maximum_kex_receive_polls)]
    maximum_kex_receive_polls: usize,
    #[arg(long, default_value_t = LimitsConfig::default().maximum_in_flight_requests)]
    maximum_in_flight_requests: usize,
    #[arg(long, default_value_t = LimitsConfig::default().maximum_realtime_polls_per_account)]
    maximum_realtime_polls_per_account: usize,
    #[arg(long, default_value_t = LimitsConfig::default().request_timeout_seconds)]
    request_timeout_seconds: u64,
    #[arg(long, default_value_t = LimitsConfig::default().maximum_pending_writes)]
    maximum_pending_writes: usize,
    #[arg(long, default_value_t = RateLimitConfig::default().connection_burst)]
    connection_rate_burst: u32,
    #[arg(long, default_value_t = RateLimitConfig::default().connections_per_second)]
    connections_per_second: u32,
    #[arg(long, default_value_t = RateLimitConfig::default().request_burst)]
    request_rate_burst: u32,
    #[arg(long, default_value_t = RateLimitConfig::default().requests_per_second)]
    requests_per_second: u32,
    #[arg(long, default_value_t = RateLimitConfig::default().maximum_tracked_ips)]
    maximum_rate_limit_ips: usize,
    #[arg(long, default_value_t = RateLimitConfig::default().ipv6_source_prefix)]
    ipv6_source_prefix: u8,
    #[arg(long, default_value_t = RateLimitConfig::default().maximum_connections_per_source)]
    maximum_connections_per_source: usize,
    #[arg(long)]
    automatic_backup_directory: Option<PathBuf>,
    #[arg(long, default_value_t = 86_400)]
    automatic_backup_interval_seconds: u64,
    #[arg(long, default_value_t = 7)]
    automatic_backup_retain: usize,
    /// Serves the unauthenticated Reg.joinWaitList route, which stores email addresses.
    #[arg(long)]
    enable_waitlist: bool,
    #[arg(long, default_value_t = foks_server::waitlist::WaitlistConfig::default().maximum_entries)]
    maximum_waitlist_entries: u64,
    #[arg(long, default_value_t = foks_server::waitlist::WaitlistConfig::default().retention_days)]
    waitlist_retention_days: u64,
    /// Database limits; `serve-config` sets them from the `[storage]` table.
    #[arg(skip)]
    database_limits: foks_server_db::Config,
}

#[derive(clap::Args)]
struct BackupArguments {
    /// Required when the source database contains external chunk references.
    #[arg(long)]
    chunk_store_config: Option<PathBuf>,
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    key_directory: PathBuf,
    #[arg(long)]
    root_key_file: PathBuf,
    #[arg(long)]
    destination: PathBuf,
    /// server.toml whose `[storage]` and `[waitlist]` tables set the database
    /// limits. Without it the compiled defaults apply, including the 64 GiB
    /// `maximum_database_bytes`.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(clap::Args)]
struct RestoreArguments {
    #[arg(long)]
    backup_directory: PathBuf,
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    key_directory: PathBuf,
    /// server.toml whose `[storage]` and `[waitlist]` tables set the database
    /// limits. Without it the compiled defaults apply, including the 64 GiB
    /// `maximum_database_bytes`.
    #[arg(long)]
    config: Option<PathBuf>,
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

fn main() -> std::process::ExitCode {
    // Report through Display: the Debug form of a decoding error embeds the
    // whole server.toml.
    match run(Arguments::parse().command) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Init(arguments) => initialize(arguments),
        Command::Serve(arguments) => serve(*arguments),
        Command::ServeConfig { config } => serve_config(config),
        Command::ConfigCheck {
            config,
            print_effective,
        } => config_check(config, print_effective),
        Command::Status { config } => status(config),
        Command::Usage { config, json } => usage(config, json),
        Command::ClientBootstrap { config, output } => client_bootstrap(config, output),
        Command::Backup(arguments) => backup(arguments),
        Command::ExternalizeChunks { config } => externalize_chunks(config),
        Command::AdoptChunkStore { config } => adopt_chunk_store(config),
        Command::InventoryChunkStore { config, restart } => inventory_chunk_store(config, restart),
        Command::CollectChunkStore {
            config,
            maximum_objects,
            maximum_bytes,
            dry_run,
        } => collect_chunk_store(config, maximum_objects, maximum_bytes, dry_run),
        Command::Restore(arguments) => restore(arguments),
        Command::RotateOperatorRoot(arguments) => rotate_operator_root(arguments),
        Command::BeginHostKeyRotation { config } => rotate_host_key(config),
        Command::CompleteHostKeyRotation(arguments) => complete_host_key_rotation(arguments),
        Command::RotateCapabilityKey { config } => rotate_capability_key(config),
        Command::RetireCapabilityKeys { config } => retire_capability_keys(config),
        Command::Invite(command) => invite_command(command),
        Command::Oidc(command) => oidc_command(command),
        Command::Admin(command) => admin_command(command),
        Command::Waitlist(command) => waitlist_command(command),
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
                    config.database_config(),
                    &code,
                    arguments.max_uses,
                    expires_at,
                    now,
                )?,
                None if arguments.max_uses.is_none() => {
                    foks_server::invites::issue_standard_invite(
                        &config.database,
                        config.database_config(),
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
                config.database_config(),
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
            for invite in
                foks_server::invites::list_invites(&config.database, config.database_config())?
            {
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
                config.database_config(),
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
    let database_limits = config.database_config();
    let limits = config.limits;
    let rate_limits = config.rate_limits;
    serve(ServeArguments {
        maximum_chunk_bytes: config.chunk_capacity.maximum_bytes,
        maximum_chunk_objects: config.chunk_capacity.maximum_objects,
        maximum_chunk_unresolved_attempts: config.chunk_capacity.maximum_unresolved_attempts,
        maximum_chunk_uncertain_bytes: config.chunk_capacity.maximum_uncertain_bytes,
        chunk_store: config.chunk_store,
        chunk_store_config: None,
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
        // The first start signs this TTL into the stored probe response, and
        // later starts refuse a different value, so it is not a setting.
        ttl_seconds: 60,
        worker_threads: limits.worker_threads,
        maximum_read_connections: limits.maximum_read_connections,
        maximum_active_connections: limits.maximum_active_connections,
        maximum_pending_connections: limits.maximum_pending_connections,
        maximum_kex_receive_polls: limits.maximum_kex_receive_polls,
        maximum_in_flight_requests: limits.maximum_in_flight_requests,
        maximum_realtime_polls_per_account: limits.maximum_realtime_polls_per_account,
        request_timeout_seconds: limits.request_timeout_seconds,
        maximum_pending_writes: limits.maximum_pending_writes,
        connection_rate_burst: rate_limits.connection_burst,
        connections_per_second: rate_limits.connections_per_second,
        request_rate_burst: rate_limits.request_burst,
        requests_per_second: rate_limits.requests_per_second,
        maximum_rate_limit_ips: rate_limits.maximum_tracked_ips,
        ipv6_source_prefix: rate_limits.ipv6_source_prefix,
        maximum_connections_per_source: rate_limits.maximum_connections_per_source,
        automatic_backup_directory: Some(config.backup_directory),
        automatic_backup_interval_seconds: config.backup_interval_seconds,
        automatic_backup_retain: config.backup_retain,
        enable_waitlist: config.waitlist.enabled,
        maximum_waitlist_entries: config.waitlist.maximum_entries,
        waitlist_retention_days: config.waitlist.retention_days,
        database_limits,
    })
}

fn config_check(path: PathBuf, print_effective: bool) -> Result<(), Box<dyn std::error::Error>> {
    // With --print-effective, standard output carries only the TOML, so that
    // a redirect produces a loadable server.toml; the report goes to stderr.
    let report = |line: &str| {
        if print_effective {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    };
    let config = foks_server::installation::load_config(path)?;
    foks_server::installation::validate_artifacts(&config)?;
    if let Some(admin) = &config.web_admin {
        match std::net::TcpListener::bind(admin.listen) {
            Ok(listener) => {
                drop(listener);
                report("admin_listener_bind=available");
            }
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                report("admin_listener_bind=busy (stop the running service before rebinding)")
            }
            Err(error) => return Err(error.into()),
        }
        if config.database.exists() {
            let count =
                foks_server_db::ReadDatabase::open(&config.database, config.database_config())?
                    .admin_operator_count()?;
            report(&format!("admin_active_operators={count}"));
            if count == 0 {
                report("No host operator configured; only self-session pages are available.");
            }
        }
    }
    report("FOKS server configuration and artifacts are valid");
    if let Some(oidc) = &config.oidc {
        report(&format!(
            "configured_provider_hash={}",
            encode_hex(&oidc.configured_fingerprint()?)
        ));
    }
    if print_effective {
        print!("{}", toml::to_string_pretty(&config)?);
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

fn usage(path: PathBuf, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    // load_config validates the configuration. The report reads only the
    // database, so unlike `config-check` it does not read the key, TLS or OIDC
    // files.
    let config = foks_server::installation::load_config(path)?;
    let report = foks_server::usage::usage_report(
        &config.database,
        config.database_config(),
        now_microseconds()?,
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&foks_server::usage::report_json(&report))?
        );
    } else {
        print!("{}", foks_server::usage::report_text(&report));
    }
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
    let waitlist = foks_server::waitlist::WaitlistConfig {
        enabled: arguments.enable_waitlist,
        maximum_entries: arguments.maximum_waitlist_entries,
        retention_days: arguments.waitlist_retention_days,
    };
    waitlist.validate()?;
    let root_key = read_root_key_file(&arguments.root_key_file)?;
    let probe_tls = probe_tls(&arguments)?;
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    // A second registration, read only after a core thread exit, so that a
    // stop request during that shutdown is never absorbed.
    let shutdown_signals = Signals::new([SIGINT, SIGTERM])?;
    // The same mapping that server.toml validation applies to `[limits]`.
    let limits = LimitsConfig {
        worker_threads: arguments.worker_threads,
        maximum_read_connections: arguments.maximum_read_connections,
        maximum_active_connections: arguments.maximum_active_connections,
        maximum_pending_connections: arguments.maximum_pending_connections,
        maximum_kex_receive_polls: arguments.maximum_kex_receive_polls,
        maximum_in_flight_requests: arguments.maximum_in_flight_requests,
        maximum_realtime_polls_per_account: arguments.maximum_realtime_polls_per_account,
        request_timeout_seconds: arguments.request_timeout_seconds,
        maximum_pending_writes: arguments.maximum_pending_writes,
    }
    .session_limits();
    match foks_server::raise_descriptor_limit(&limits) {
        Ok(limit) if !limit.sufficient() => eprintln!(
            "FOKS server warning: the descriptor limit {} is below the {} descriptors the \
             configured connection limits can use; raise LimitNOFILE or the hard RLIMIT_NOFILE",
            limit.soft, limit.required,
        ),
        Ok(_) => {}
        Err(error) => eprintln!("FOKS server warning: cannot raise the descriptor limit: {error}"),
    }
    let now_microseconds =
        u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;
    let server = foks_server::start_standalone(StandaloneConfig {
        chunk_store: match (&arguments.chunk_store, &arguments.chunk_store_config) {
            (Some(config), None) => Some(config.build()?),
            (None, Some(path)) => Some(load_chunk_store(path)?.build()?),
            (None, None) => None,
            (Some(_), Some(_)) => return Err("specify one chunk-store configuration".into()),
        },
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
        database: waitlist.database_config(
            ChunkCapacityConfig {
                maximum_bytes: arguments.maximum_chunk_bytes,
                maximum_objects: arguments.maximum_chunk_objects,
                maximum_unresolved_attempts: arguments.maximum_chunk_unresolved_attempts,
                maximum_uncertain_bytes: arguments.maximum_chunk_uncertain_bytes,
            }
            .database_config_from(arguments.database_limits)?,
        ),
        limits,
        rate_limits: foks_server::RateLimitConfig {
            connection_burst: arguments.connection_rate_burst,
            connections_per_second: arguments.connections_per_second,
            request_burst: arguments.request_rate_burst,
            requests_per_second: arguments.requests_per_second,
            maximum_tracked_ips: arguments.maximum_rate_limit_ips,
            ipv6_source_prefix: arguments.ipv6_source_prefix,
            maximum_connections_per_source: arguments.maximum_connections_per_source,
        },
        backup: arguments
            .automatic_backup_directory
            .map(|directory| foks_server::BackupSchedule {
                directory,
                interval: Duration::from_secs(arguments.automatic_backup_interval_seconds),
                retain: arguments.automatic_backup_retain,
            }),
        clock: Arc::new(foks_server_db::SystemClock),
        entropy: Arc::new(foks_server::OsEntropy),
        session_faults: None,
        diagnostics: Some(Arc::new(foks_server::StderrSessionDiagnostics)),
        ttl_seconds: arguments.ttl_seconds,
        now_microseconds,
        maximum_pending_writes: arguments.maximum_pending_writes,
        waitlist_enabled: waitlist.enabled,
    })?;
    let addresses = server.addresses();
    eprintln!(
        "FOKS server ready: probe={}, public={}, authenticated={}, management={}",
        addresses.probe,
        addresses.public_services,
        addresses.authenticated,
        server.management_address(),
    );
    // A core thread exit ends the wait as a signal would. The process then
    // exits non-zero, so a service manager with Restart=on-failure starts a
    // new one instead of keeping a process that no longer serves.
    let wake = signals.handle();
    server.on_fatal_exit(move |_| wake.close());
    let _ = signals.forever().next();
    if let Some(thread) = server.fatal_thread() {
        eprintln!(
            "FOKS server stopping: the {} thread exited unexpectedly",
            thread.label()
        );
        exit_after_fatal_thread(server, shutdown_signals, limits.request_timeout);
    }
    server.shutdown()?;
    Ok(())
}

/// Time allowed for the shutdown that follows a core thread exit, beyond the
/// request timeout that the RPC listener runtime waits for in-flight
/// requests. It covers the bounded waits of the other threads, which run one
/// after another.
const FATAL_SHUTDOWN_MARGIN: Duration = Duration::from_secs(60);

fn exit_after_fatal_thread(
    server: RunningStandaloneServer,
    mut signals: Signals,
    request_timeout: Duration,
) -> ! {
    // Stop the threads that still run so that sessions end and the database
    // closes in order, but never let a blocked thread keep the process alive.
    // An interrupted SQLite write is recovered from the WAL on the next start.
    let deadline = request_timeout.saturating_add(FATAL_SHUTDOWN_MARGIN);
    std::thread::spawn(move || {
        std::thread::sleep(deadline);
        eprintln!(
            "FOKS server shutdown did not finish within {} seconds",
            deadline.as_secs()
        );
        std::process::exit(1);
    });
    // The closed iterator of the main wait would absorb a stop request, so a
    // SIGINT or SIGTERM, including one already pending, now ends the process.
    std::thread::spawn(move || {
        if signals.forever().next().is_some() {
            eprintln!("FOKS server shutdown interrupted by a signal");
            std::process::exit(1);
        }
    });
    if let Err(error) = server.shutdown() {
        eprintln!("FOKS server shutdown after the thread exit failed: {error}");
    }
    std::process::exit(1);
}

fn backup(arguments: BackupArguments) -> Result<(), Box<dyn std::error::Error>> {
    let root_key = read_root_key_file(&arguments.root_key_file)?;
    let store = arguments
        .chunk_store_config
        .as_deref()
        .map(load_chunk_store)
        .transpose()?
        .map(|config| config.build())
        .transpose()?;
    foks_server::backup_standalone_installation_with_chunk_store(
        arguments.database,
        arguments.key_directory,
        *root_key,
        arguments.destination,
        backup_database_limits(arguments.config)?,
        store.as_deref(),
    )?;
    Ok(())
}

/// Database limits for `backup` and `restore`. Their paths come from flags;
/// an optional server.toml supplies only the limits, so that a database that
/// a raised `[storage]` ceiling let grow past the compiled one can still be
/// backed up and restored.
fn backup_database_limits(
    config: Option<PathBuf>,
) -> Result<foks_server_db::Config, Box<dyn std::error::Error>> {
    Ok(match config {
        Some(path) => foks_server::installation::load_config(path)?.database_config(),
        None => foks_server_db::Config::default(),
    })
}

fn load_chunk_store(
    path: &Path,
) -> Result<foks_server::chunks::ChunkStoreConfig, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(65_537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65_536 {
        return Err("chunk-store configuration is too large".into());
    }
    let config: foks_server::chunks::ChunkStoreConfig =
        toml::from_str(std::str::from_utf8(&bytes)?)?;
    config.validate()?;
    Ok(config)
}

fn externalize_chunks(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(path)?;
    let store = config
        .chunk_store
        .as_ref()
        .ok_or("installation has no chunk-store configuration")?
        .build()?;
    let converted = foks_server::externalize_chunks(
        &config.database,
        config.database_config(),
        store.as_ref(),
    )?;
    println!("externalized {converted} encrypted chunks");
    Ok(())
}

fn adopt_chunk_store(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Before first adoption, preserve a consistent offline SQLite backup (including committed WAL). Adoption pins the chosen store even if verification fails; there is no automatic reset or rebind.");
    let config = foks_server::installation::load_config(path)?;
    let store = config
        .chunk_store
        .as_ref()
        .ok_or("installation has no chunk-store configuration")?
        .build()?;
    let result = foks_server::adopt_chunk_store(
        &config.database,
        config.database_config(),
        store.as_ref(),
        |progress| {
            eprintln!(
                "verified {} external references ({} encoded bytes)",
                progress.verified_objects, progress.verified_bytes
            );
        },
    )?;
    println!(
        "chunk store bound; verified {} references ({} encoded bytes)",
        result.verified_objects, result.verified_bytes
    );
    Ok(())
}

fn inventory_chunk_store(path: PathBuf, restart: bool) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Inventory requires a stopped installation and drained legacy remote publishers. A timeout or missing object does not prove a prior PUT stopped. This command accounts objects and never deletes them.");
    let config = foks_server::installation::load_config(path)?;
    let store = config
        .chunk_store
        .as_ref()
        .ok_or("installation has no chunk store")?
        .build()?;
    let usage = foks_server::inventory_chunk_store(
        &config.database,
        config.database_config(),
        store.as_ref(),
        restart,
        |checkpoint| {
            eprintln!(
                "inventory generation={} continuation={}",
                checkpoint.generation,
                checkpoint.cursor.is_some()
            )
        },
    )?;
    println!("inventory_complete={} present_bytes={} reserved_bytes={} uncertain_bytes={} objects={} unresolved_attempts={} admission_rejections={} orphan_objects={} orphan_bytes={} unknown_objects={} unknown_bytes={} missing_objects={} missing_referenced_objects={} oldest_attempt_micros={:?}",usage.inventory_complete,usage.present_bytes,usage.reserved_bytes,usage.uncertain_bytes,usage.objects,usage.unresolved_attempts,usage.admission_rejections,usage.orphan_objects,usage.orphan_bytes,usage.unknown_objects,usage.unknown_bytes,usage.missing_objects,usage.missing_referenced_objects,usage.oldest_attempt_micros);
    println!("unknown_publication_attempts={} retrying_remote_publications={} acknowledged_remote_publications={} remote_termination_qualified=false",usage.unknown_publication_attempts,usage.retrying_remote_publications,usage.acknowledged_remote_publications);
    Ok(())
}

fn collect_chunk_store(
    path: PathBuf,
    maximum_objects: usize,
    maximum_bytes: u64,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = foks_server::installation::load_config(path)?;
    let store = config
        .chunk_store
        .as_ref()
        .ok_or("collection requires configured chunk storage")?
        .build()?;
    eprintln!("Offline collection requires exclusive installation ownership. Unknown objects and uncertain publishers remain charged. Remote backends are report-only.");
    let report = foks_server::collect_chunk_store(
        &config.database,
        config.database_config(),
        store.as_ref(),
        maximum_objects,
        maximum_bytes,
        dry_run,
    )?;
    println!("report_only={} backend_qualified={} ledger_candidates={} deleted_objects={} deleted_bytes={} removed_temporaries={} charged_objects={} deleting_objects={} deleting_bytes={} tombstones={} unresolved_attempts={} unknown_objects={}",report.report_only,report.backend_qualified,report.eligible_objects,report.deleted_objects,report.deleted_bytes,report.removed_temporaries,report.usage.objects,report.usage.deleting_objects,report.usage.deleting_bytes,report.usage.tombstones,report.usage.unresolved_attempts,report.usage.unknown_objects);
    println!("unknown_publication_attempts={} retrying_remote_publications={} acknowledged_remote_publications={} remote_termination_qualified=false",report.usage.unknown_publication_attempts,report.usage.retrying_remote_publications,report.usage.acknowledged_remote_publications);
    Ok(())
}

fn restore(arguments: RestoreArguments) -> Result<(), Box<dyn std::error::Error>> {
    let database_limits = backup_database_limits(arguments.config)?;
    foks_server::restore_backup(
        &foks_server::BackupArtifacts {
            database: arguments.backup_directory.join("foks-server.sqlite"),
            key_directory: arguments.backup_directory.join("keys"),
            key_manifest: arguments.backup_directory.join("key-manifest.txt"),
        },
        arguments.database,
        arguments.key_directory,
        database_limits,
    )?;
    eprintln!(
        "Restore complete. The restored database has no chunk store binding: if the installation uses a chunk store, configure a new empty directory or prefix. Objects in the previous store are not referenced by the restored database, and no collector of the restored installation deletes them."
    );
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
    let mut database = foks_server_db::Database::open(&config.database, config.database_config())?;
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
    let mut database = foks_server_db::Database::open(&config.database, config.database_config())?;
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
    let mut database =
        foks_server_db::Database::open_existing(&config.database, config.database_config())?;
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
    let mut database =
        foks_server_db::Database::open_existing(&config.database, config.database_config())?;
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

#[derive(clap::Subcommand)]
enum WaitlistCommand {
    /// Prints every waitlist entry, oldest first.
    List {
        #[arg(long)]
        config: PathBuf,
    },
    /// Writes every waitlist entry as JSON to a new file readable only by its owner.
    Export {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Deletes waitlist entries while the service is stopped.
    Purge(WaitlistPurgeArguments),
}

#[derive(clap::Args)]
struct WaitlistPurgeArguments {
    #[arg(long)]
    config: PathBuf,
    #[command(flatten)]
    selection: WaitlistPurgeSelection,
}

#[derive(clap::Args)]
#[group(required = true, multiple = false)]
struct WaitlistPurgeSelection {
    /// Deletes entries created before this Unix time.
    #[arg(long)]
    before_unix_seconds: Option<u64>,
    /// Deletes the entry for this address, compared under ASCII case folding.
    #[arg(long)]
    email: Option<String>,
    /// Deletes every entry.
    #[arg(long)]
    all: bool,
}

fn waitlist_command(command: WaitlistCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        WaitlistCommand::List { config } => {
            let config = foks_server::installation::load_config(config)?;
            foks_server::installation::validate_artifacts(&config)?;
            let entries =
                foks_server::waitlist::list_entries(&config.database, config.database_config())?;
            if entries.is_empty() {
                println!("No waitlist entries.");
            }
            for entry in entries {
                println!(
                    "id={} email={} created_at={}",
                    encode_hex(&entry.waitlist_id),
                    entry.email,
                    entry.created_at
                );
            }
        }
        WaitlistCommand::Export { config, output } => {
            let config = foks_server::installation::load_config(config)?;
            foks_server::installation::validate_artifacts(&config)?;
            let count = foks_server::waitlist::export_entries(
                &config.database,
                config.database_config(),
                &output,
            )?;
            println!("exported {count} waitlist entries to {}", output.display());
        }
        WaitlistCommand::Purge(arguments) => {
            let config = foks_server::installation::load_config(arguments.config)?;
            foks_server::installation::validate_artifacts(&config)?;
            let selection = &arguments.selection;
            let selection = match (selection.before_unix_seconds, &selection.email) {
                (Some(seconds), _) => foks_server_db::WaitlistPurge::CreatedBefore(
                    seconds
                        .checked_mul(1_000_000)
                        .ok_or("--before-unix-seconds exceeds the supported timestamp")?,
                ),
                (None, Some(email)) => foks_server_db::WaitlistPurge::Email(email),
                (None, None) => foks_server_db::WaitlistPurge::All,
            };
            let deleted = foks_server::waitlist::purge_entries(
                &config.database,
                config.database_config(),
                selection,
            )?;
            println!("deleted {deleted} waitlist entries");
        }
    }
    Ok(())
}
