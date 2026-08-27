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
    /// Runs the standalone network service.
    Serve(ServeArguments),
    /// Creates a validated online backup of an installation.
    Backup(BackupArguments),
    /// Installs a validated backup into an empty installation.
    Restore(RestoreArguments),
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
    #[arg(long, default_value_t = 60)]
    ttl_seconds: i64,
    #[arg(long, default_value_t = 4)]
    worker_threads: usize,
    #[arg(long, default_value_t = 32)]
    maximum_pending_connections: usize,
    #[arg(long, default_value_t = 64)]
    maximum_pending_writes: usize,
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Arguments::parse().command {
        Command::Serve(arguments) => serve(arguments),
        Command::Backup(arguments) => backup(arguments),
        Command::Restore(arguments) => restore(arguments),
    }
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
        probe_tls,
        database: foks_server_db::Config::default(),
        limits: SessionLimits {
            worker_threads: arguments.worker_threads,
            maximum_pending_connections: arguments.maximum_pending_connections,
            ..SessionLimits::default()
        },
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
        "FOKS server ready: probe={}, public={}, authenticated={}",
        addresses.probe, addresses.public_services, addresses.authenticated
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
