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
    }
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
    println!("FOKS server configuration and artifacts are valid");
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
