use std::io::{Read as _, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use foks_server_db::Database;
use zeroize::Zeroizing;

use crate::host::{load_or_bootstrap, BootstrapEndpoints, BootstrapInput, BootstrapState};
use crate::keys::DirectoryKeyProvider;
use crate::maintenance::Maintenance;
use crate::net::{bind_addresses, RunningServer, ServerAddresses};
use crate::pki::build_host_tls;
use crate::Writer;
use crate::{Config, ReadDatabaseConfig, Result, SessionLimits};

pub struct StandaloneConfig {
    pub database_path: PathBuf,
    pub key_directory: PathBuf,
    pub root_key: Zeroizing<[u8; 32]>,
    pub canonical_name: String,
    pub probe_address: SocketAddr,
    pub public_address: SocketAddr,
    pub authenticated_address: SocketAddr,
    pub management_address: SocketAddr,
    pub probe_tls: Arc<rustls::ServerConfig>,
    pub database: foks_server_db::Config,
    pub limits: SessionLimits,
    pub rate_limits: crate::RateLimitConfig,
    pub backup: Option<BackupSchedule>,
    pub clock: Arc<dyn foks_server_db::Clock>,
    pub entropy: Arc<dyn crate::Entropy>,
    #[doc(hidden)]
    pub session_faults: Option<Arc<crate::SessionFaults>>,
    pub diagnostics: Option<Arc<dyn crate::SessionDiagnostics>>,
    pub ttl_seconds: i64,
    pub now_microseconds: u64,
    pub maximum_pending_writes: usize,
}

pub struct RunningStandaloneServer {
    server: RunningServer,
    bootstrap: BootstrapState,
    delegated_roots: rustls::RootCertStore,
    client_roots: rustls::RootCertStore,
    writer: Writer,
    maintenance: Maintenance,
    database_path: PathBuf,
    key_directory: PathBuf,
    database_config: foks_server_db::Config,
    clock: Arc<dyn foks_server_db::Clock>,
    metrics: Arc<crate::ServerMetrics>,
    management: crate::operations::ManagementServer,
    backup: Option<crate::operations::BackupScheduler>,
}

#[derive(Clone, Debug)]
pub struct BackupSchedule {
    pub directory: PathBuf,
    pub interval: std::time::Duration,
    pub retain: usize,
}

impl BackupSchedule {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.interval.is_zero() || self.retain == 0 {
            return Err(crate::Error::Config("invalid backup schedule"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupArtifacts {
    pub database: PathBuf,
    pub key_directory: PathBuf,
    pub key_manifest: PathBuf,
}

/// Validates a completed backup and installs its database and encrypted keys
/// into an empty standalone-server installation. The operator-provided root
/// key is deliberately not part of the backup and is checked at startup.
pub fn restore_backup(
    artifacts: &BackupArtifacts,
    database_path: impl AsRef<Path>,
    key_directory: impl AsRef<Path>,
    database_config: foks_server_db::Config,
) -> Result<()> {
    const MAXIMUM_MANIFEST_BYTES: u64 = 4 * 1024;
    let database_path = database_path.as_ref();
    let key_directory = key_directory.as_ref();
    if database_path.exists() || directory_has_entries(key_directory)? {
        return Err(crate::Error::Config("restore destination is not empty"));
    }

    let manifest_metadata = regular_file_metadata(&artifacts.key_manifest)?;
    if manifest_metadata.len() == 0 || manifest_metadata.len() > MAXIMUM_MANIFEST_BYTES {
        return Err(crate::Error::Key("invalid backup key manifest size"));
    }
    let mut manifest = Vec::with_capacity(manifest_metadata.len() as usize);
    std::fs::File::open(&artifacts.key_manifest)?
        .take(MAXIMUM_MANIFEST_BYTES + 1)
        .read_to_end(&mut manifest)?;
    let decoded_manifest = crate::keys::KeyGenerationManifest::decode(&manifest)?;

    regular_file_metadata(&artifacts.database)?;
    let backup = foks_server_db::ReadDatabase::open(&artifacts.database, database_config.clone())?;
    if !backup.integrity_check()? {
        return Err(crate::Error::Database(foks_server_db::Error::Invalid(
            "backup integrity check",
        )));
    }
    let stored =
        backup
            .host_bootstrap()?
            .ok_or(crate::Error::Database(foks_server_db::Error::Invalid(
                "backup host bootstrap",
            )))?;
    if stored.key_manifest != manifest {
        return Err(crate::Error::Key("backup key manifest mismatch"));
    }
    drop(backup);

    let wrapping_key = artifacts.key_directory.join(crate::keys::WRAPPING_KEY_FILE);
    regular_file_metadata(&wrapping_key)?;

    for purpose in crate::keys::MANIFEST_PURPOSES {
        let expected = decoded_manifest
            .generation(purpose)
            .ok_or(crate::Error::Key("incomplete backup key manifest"))?;
        let source = artifacts
            .key_directory
            .join(format!("{}.key", purpose.label()));
        regular_file_metadata(&source)?;
        // The generation is authenticated only after the operator supplies
        // the root key at startup. Here the manifest still defines the exact
        // set of files that may be restored.
        let _ = expected;
    }

    if let Some(parent) = database_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !key_directory.exists() {
        std::fs::create_dir_all(key_directory)?;
    }
    // Publish the authoritative database first. If a later key copy fails,
    // startup observes its manifest and fails closed rather than bootstrapping
    // a replacement host over a key-only partial restore.
    copy_new_file(&artifacts.database, database_path)?;
    if let Some(parent) = database_path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    copy_new_file(
        &artifacts.key_directory.join(crate::keys::WRAPPING_KEY_FILE),
        &key_directory.join(crate::keys::WRAPPING_KEY_FILE),
    )?;
    for purpose in crate::keys::MANIFEST_PURPOSES {
        let name = format!("{}.key", purpose.label());
        copy_new_file(
            &artifacts.key_directory.join(&name),
            &key_directory.join(name),
        )?;
    }
    std::fs::File::open(key_directory)?.sync_all()?;
    Ok(())
}

fn directory_has_entries(path: &Path) -> std::io::Result<bool> {
    match std::fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn regular_file_metadata(path: &Path) -> Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(crate::Error::Key("backup artifact is not a regular file"));
    }
    Ok(metadata)
}

fn copy_new_file(source: &Path, destination: &Path) -> Result<()> {
    let mut source = std::fs::File::open(source)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut destination = options.open(destination)?;
    std::io::copy(&mut source, &mut destination)?;
    destination.flush()?;
    destination.sync_all()?;
    Ok(())
}

/// Creates an online backup directly from a standalone installation after
/// authenticating every encrypted key against the operator's root key and
/// the database's immutable generation manifest.
pub fn backup_standalone_installation(
    database_path: impl AsRef<Path>,
    key_directory: impl AsRef<Path>,
    root_key: [u8; 32],
    destination: impl AsRef<Path>,
    database_config: foks_server_db::Config,
) -> Result<BackupArtifacts> {
    let source = foks_server_db::ReadDatabase::open(&database_path, database_config.clone())?;
    if !source.integrity_check()? {
        return Err(crate::Error::Database(foks_server_db::Error::Invalid(
            "source integrity check",
        )));
    }
    let stored =
        source
            .host_bootstrap()?
            .ok_or(crate::Error::Database(foks_server_db::Error::Invalid(
                "source host bootstrap",
            )))?;
    let manifest = crate::keys::KeyGenerationManifest::decode(&stored.key_manifest)?;
    let key_directory_metadata = std::fs::symlink_metadata(&key_directory)?;
    if !key_directory_metadata.file_type().is_dir()
        || key_directory_metadata.file_type().is_symlink()
    {
        return Err(crate::Error::Key(
            "source key directory is not a regular directory",
        ));
    }
    let provider = crate::keys::DirectoryKeyProvider::open(&key_directory, root_key)?;
    for purpose in crate::keys::MANIFEST_PURPOSES {
        if provider.load_existing(purpose)?.generation()
            != manifest.generation(purpose).ok_or(crate::Error::Key(
                "incomplete source key generation manifest",
            ))?
        {
            return Err(crate::Error::Key("source key generation mismatch"));
        }
    }
    create_backup(
        destination.as_ref(),
        key_directory.as_ref(),
        &stored.key_manifest,
        database_config,
        |backup_database| {
            source.online_backup(backup_database)?;
            Ok(())
        },
    )
}

pub(crate) fn create_backup(
    destination: &Path,
    source_key_directory: &Path,
    manifest: &[u8],
    database_config: foks_server_db::Config,
    backup_database: impl FnOnce(&Path) -> Result<()>,
) -> Result<BackupArtifacts> {
    std::fs::create_dir(destination)?;
    let database = destination.join("foks-server.sqlite");
    let key_directory = destination.join("keys");
    std::fs::create_dir(&key_directory)?;
    let wrapping_key = source_key_directory.join(crate::keys::WRAPPING_KEY_FILE);
    regular_file_metadata(&wrapping_key)?;
    copy_new_file(
        &wrapping_key,
        &key_directory.join(crate::keys::WRAPPING_KEY_FILE),
    )?;
    for purpose in crate::keys::MANIFEST_PURPOSES {
        let name = format!("{}.key", purpose.label());
        let source = source_key_directory.join(&name);
        regular_file_metadata(&source)?;
        copy_new_file(&source, &key_directory.join(name))?;
    }
    std::fs::File::open(&key_directory)?.sync_all()?;
    backup_database(&database)?;
    let backup_check = foks_server_db::ReadDatabase::open(&database, database_config)?;
    if !backup_check.integrity_check()? {
        return Err(crate::Error::Database(foks_server_db::Error::Invalid(
            "online backup integrity check",
        )));
    }
    drop(backup_check);
    // Publish the manifest last; its presence is the durable completeness
    // marker for an otherwise possibly partial crash artifact.
    let key_manifest = destination.join("key-manifest.txt");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut manifest_file = options.open(&key_manifest)?;
    manifest_file.write_all(manifest)?;
    manifest_file.sync_all()?;
    std::fs::File::open(destination)?.sync_all()?;
    Ok(BackupArtifacts {
        database,
        key_directory,
        key_manifest,
    })
}

impl RunningStandaloneServer {
    pub fn addresses(&self) -> ServerAddresses {
        self.server.addresses()
    }

    pub fn bootstrap(&self) -> &BootstrapState {
        &self.bootstrap
    }

    pub fn delegated_roots(&self) -> rustls::RootCertStore {
        self.delegated_roots.clone()
    }

    pub fn client_roots(&self) -> rustls::RootCertStore {
        self.client_roots.clone()
    }

    /// Runs the same bounded reclamation and WAL checkpoint used by the
    /// background maintenance loop, returning non-secret operator metrics.
    pub fn run_maintenance(
        &self,
    ) -> Result<(
        foks_server_db::MaintenanceReport,
        foks_server_db::CheckpointReport,
    )> {
        let now = self.clock.now_micros()?;
        let cutoff = now.saturating_sub(crate::maintenance::ABANDONED_UPLOAD_AGE_MICROS);
        self.writer.handle().call(move |database| {
            let maintenance = database.run_maintenance(now, cutoff)?;
            let checkpoint = database.checkpoint()?;
            Ok((maintenance, checkpoint))
        })
    }

    #[doc(hidden)]
    pub fn writer_handle(&self) -> crate::WriterHandle {
        self.writer.handle()
    }

    pub fn metrics(&self) -> crate::ServerMetricsSnapshot {
        self.metrics.snapshot()
    }

    pub fn management_address(&self) -> SocketAddr {
        self.management.address()
    }

    pub fn storage_report(&self) -> Result<foks_server_db::StorageReport> {
        self.writer
            .handle()
            .call(|database| Ok(database.storage_report()?))
    }

    /// Creates a self-consistent SQLite backup and immutable encrypted-key
    /// snapshot in a new directory. The operator root key is intentionally
    /// never copied and must be restored through its original secret channel.
    pub fn backup(&self, destination: impl AsRef<std::path::Path>) -> Result<BackupArtifacts> {
        let manifest = self.bootstrap.key_manifest.encode();
        let source =
            foks_server_db::ReadDatabase::open(&self.database_path, self.database_config.clone())?;
        create_backup(
            destination.as_ref(),
            &self.key_directory,
            &manifest,
            self.database_config.clone(),
            |backup_database| {
                source.online_backup(backup_database)?;
                Ok(())
            },
        )
    }

    pub fn shutdown(self) -> Result<()> {
        self.management.mark_not_ready();
        self.server.shutdown()?;
        self.management.shutdown()?;
        if let Some(backup) = self.backup {
            backup.shutdown()?;
        }
        self.maintenance.shutdown()?;
        self.writer.shutdown()
    }
}

pub fn start_standalone(config: StandaloneConfig) -> Result<RunningStandaloneServer> {
    config.limits.validate()?;
    let key_directory = config.key_directory.clone();
    let keys = Arc::new(DirectoryKeyProvider::open(
        &config.key_directory,
        *config.root_key,
    )?);
    let tls = build_host_tls(keys.as_ref(), &config.canonical_name)?;
    let listeners = bind_addresses(
        config.probe_address,
        config.public_address,
        config.authenticated_address,
    )?;
    let addresses = listeners.addresses();
    let endpoint = |address: SocketAddr| {
        if config.canonical_name.contains(':') {
            format!("[{}]:{}", config.canonical_name, address.port())
        } else {
            format!("{}:{}", config.canonical_name, address.port())
        }
    };
    let endpoints = BootstrapEndpoints {
        probe: endpoint(addresses.probe),
        public_services: endpoint(addresses.public_services),
        authenticated: endpoint(addresses.authenticated),
    };
    let input = BootstrapInput {
        canonical_name: config.canonical_name,
        endpoints,
        ttl_seconds: config.ttl_seconds,
        now_microseconds: config.now_microseconds,
    };
    let mut database = Database::open(&config.database_path, config.database.clone())?;
    let bootstrap = load_or_bootstrap(&mut database, keys.as_ref(), &input)?;
    drop(database);
    let database_config = config.database;
    let writer = Writer::start(
        config.database_path.clone(),
        database_config.clone(),
        config.maximum_pending_writes,
    )?;
    let writer_handle = writer.handle();
    let maintenance = Maintenance::start(writer_handle.clone(), Arc::clone(&config.clock));
    let metrics = Arc::new(crate::ServerMetrics::default());
    let backup = config
        .backup
        .map(|schedule| {
            crate::operations::BackupScheduler::start(
                schedule,
                config.database_path.clone(),
                key_directory.clone(),
                bootstrap.key_manifest.encode(),
                database_config.clone(),
                Arc::clone(&config.clock),
                Arc::clone(&metrics),
            )
        })
        .transpose()?;

    let server = RunningServer::start_bound(
        Config {
            probe_address: config.probe_address,
            public_address: config.public_address,
            authenticated_address: config.authenticated_address,
            probe_tls: config.probe_tls,
            public_tls: tls.public,
            authenticated_tls: tls.authenticated,
            probe_response: Arc::from(bootstrap.probe_response.clone()),
            read_database: Some(ReadDatabaseConfig {
                path: config.database_path.clone(),
                database: database_config.clone(),
            }),
            writer: Some(writer_handle.clone()),
            clock: Arc::clone(&config.clock),
            entropy: config.entropy,
            key_provider: Some(keys),
            session_faults: config.session_faults,
            diagnostics: config.diagnostics,
            metrics: Arc::clone(&metrics),
            rate_limits: config.rate_limits,
            limits: config.limits,
        },
        listeners,
    )?;
    let management = crate::operations::ManagementServer::start(
        config.management_address,
        writer_handle,
        Arc::clone(&metrics),
        server.liveness(),
    )?;
    management.mark_ready();
    Ok(RunningStandaloneServer {
        server,
        bootstrap,
        delegated_roots: tls.delegated_ca,
        client_roots: tls.client_ca,
        writer,
        maintenance,
        database_path: config.database_path,
        key_directory,
        database_config,
        clock: config.clock,
        metrics,
        management,
        backup,
    })
}
