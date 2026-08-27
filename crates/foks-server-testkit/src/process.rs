use std::path::Path;
use std::sync::Arc;

use foks_server::{RunningStandaloneServer, ServerAddresses, SessionLimits, StandaloneConfig};

use crate::environment::TestEnvironment;

pub struct InProcessServer {
    environment: TestEnvironment,
    server: Option<RunningStandaloneServer>,
}

impl InProcessServer {
    pub(crate) fn start(environment: TestEnvironment) -> foks_server::Result<Self> {
        {
            let mut running = environment
                .inner
                .running
                .lock()
                .expect("test server lifecycle lock");
            if *running {
                return Err(foks_server::Error::Config("test server already running"));
            }
            *running = true;
        }
        let [probe_address, public_address, authenticated_address] =
            environment.configured_addresses();
        let result = foks_server::start_standalone(StandaloneConfig {
            database_path: environment.inner.paths.database().to_path_buf(),
            key_directory: environment.inner.paths.keys().to_path_buf(),
            root_key: zeroize::Zeroizing::new(environment.inner.root_key),
            canonical_name: "localhost".to_owned(),
            probe_address,
            public_address,
            authenticated_address,
            management_address: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
            probe_tls: Arc::clone(&environment.inner.tls.public),
            database: environment.inner.database_config.clone(),
            limits: environment.inner.session_limits,
            rate_limits: environment.inner.rate_limits,
            backup: environment.inner.backup.clone(),
            clock: environment.inner.clock.clone(),
            entropy: Arc::new(foks_server::OsEntropy),
            session_faults: Some(Arc::clone(&environment.inner.session_faults)),
            diagnostics: None,
            ttl_seconds: 60,
            now_microseconds: environment.inner.clock.now(),
            maximum_pending_writes: environment.inner.maximum_pending_writes,
        });
        let server = match result {
            Ok(server) => server,
            Err(error) => {
                *environment
                    .inner
                    .running
                    .lock()
                    .expect("test server lifecycle lock") = false;
                return Err(error);
            }
        };
        let addresses = server.addresses();
        let mut retained = environment
            .inner
            .addresses
            .lock()
            .expect("test address lock");
        if retained.is_some_and(|former| former != addresses) {
            drop(retained);
            let shutdown = server.shutdown();
            *environment
                .inner
                .running
                .lock()
                .expect("test server lifecycle lock") = false;
            shutdown?;
            return Err(foks_server::Error::Config("test server address changed"));
        }
        *retained = Some(addresses);
        drop(retained);
        Ok(Self {
            environment,
            server: Some(server),
        })
    }

    pub fn addresses(&self) -> ServerAddresses {
        self.server().addresses()
    }

    pub fn probe_roots(&self) -> rustls::RootCertStore {
        self.environment.probe_roots()
    }

    pub fn service_roots(&self) -> rustls::RootCertStore {
        self.server().delegated_roots()
    }

    #[doc(hidden)]
    pub fn client_roots(&self) -> rustls::RootCertStore {
        self.server().client_roots()
    }

    pub fn probe_response(&self) -> Vec<u8> {
        self.server().bootstrap().probe_response.clone()
    }

    pub fn root(&self) -> &Path {
        self.environment.root()
    }

    pub fn identity(
        &self,
        uid: &[u8],
    ) -> foks_server_db::Result<Option<foks_server_db::IdentitySnapshot>> {
        foks_server_db::ReadDatabase::open(
            self.environment.inner.paths.database(),
            foks_server_db::Config::default(),
        )?
        .identity(uid)
    }

    pub fn current_root(&self) -> foks_server_db::Result<Option<foks_server_db::RootSnapshot>> {
        foks_server_db::ReadDatabase::open(
            self.environment.inner.paths.database(),
            foks_server_db::Config::default(),
        )?
        .current_root()
    }

    pub fn request_receipt(
        &self,
        idempotency_key: &[u8],
        request_hash: &[u8; 32],
    ) -> foks_server_db::Result<Option<foks_server_db::Receipt>> {
        foks_server_db::ReadDatabase::open(
            self.environment.inner.paths.database(),
            foks_server_db::Config::default(),
        )?
        .request_receipt(
            idempotency_key,
            request_hash,
            self.environment.inner.clock.now(),
        )
    }

    pub fn owned_paths(&self) -> [&Path; 4] {
        self.environment.inner.paths.all()
    }

    pub fn backup_named(&self, name: &str) -> foks_server::Result<foks_server::BackupArtifacts> {
        if !valid_name(name) {
            return Err(foks_server::Error::Config("invalid test backup name"));
        }
        self.server()
            .backup(self.environment.inner.paths.backup().join(name))
    }

    pub fn operator_backup_named(
        &self,
        name: &str,
    ) -> foks_server::Result<foks_server::BackupArtifacts> {
        if !valid_name(name) {
            return Err(foks_server::Error::Config("invalid test backup name"));
        }
        foks_server::backup_standalone_installation(
            self.environment.inner.paths.database(),
            self.environment.inner.paths.keys(),
            self.environment.inner.root_key,
            self.environment.inner.paths.backup().join(name),
            foks_server_db::Config::default(),
        )
    }

    pub fn backup_is_valid(
        &self,
        artifacts: &foks_server::BackupArtifacts,
    ) -> foks_server_db::Result<bool> {
        foks_server_db::Database::open(&artifacts.database, foks_server_db::Config::default())?
            .integrity_check()
    }

    pub fn run_maintenance(
        &self,
    ) -> foks_server::Result<(
        foks_server_db::MaintenanceReport,
        foks_server_db::CheckpointReport,
    )> {
        self.server().run_maintenance()
    }

    pub fn writer_handle(&self) -> foks_server::WriterHandle {
        self.server().writer_handle()
    }

    pub fn saturate_writer_queue(&self) -> foks_server::Result<crate::WriterQueuePressure> {
        crate::WriterQueuePressure::start(self.writer_handle())
    }

    pub fn metrics(&self) -> foks_server::ServerMetricsSnapshot {
        self.server().metrics()
    }

    pub fn management_address(&self) -> std::net::SocketAddr {
        self.server().management_address()
    }

    pub fn storage_report(&self) -> foks_server::Result<foks_server_db::StorageReport> {
        self.server().storage_report()
    }

    pub fn shutdown(mut self) -> foks_server::Result<()> {
        self.shutdown_inner()
    }

    fn server(&self) -> &RunningStandaloneServer {
        self.server.as_ref().expect("test server is running")
    }

    fn shutdown_inner(&mut self) -> foks_server::Result<()> {
        let result = self
            .server
            .take()
            .map(RunningStandaloneServer::shutdown)
            .unwrap_or(Ok(()));
        *self
            .environment
            .inner
            .running
            .lock()
            .expect("test server lifecycle lock") = false;
        result
    }
}

impl Drop for InProcessServer {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

pub struct ProbeOverrideServer {
    environment: TestEnvironment,
    server: Option<foks_server::net::RunningServer>,
}

impl ProbeOverrideServer {
    pub(crate) fn start(
        environment: TestEnvironment,
        probe_response: Vec<u8>,
    ) -> foks_server::Result<Self> {
        {
            let mut running = environment
                .inner
                .running
                .lock()
                .expect("test server lifecycle lock");
            if *running {
                return Err(foks_server::Error::Config("test server already running"));
            }
            *running = true;
        }
        let [probe_address, public_address, authenticated_address] =
            environment.configured_addresses();
        let tls = Arc::clone(&environment.inner.tls.public);
        let result = foks_server::net::start(foks_server::Config {
            probe_address,
            public_address,
            authenticated_address,
            probe_tls: Arc::clone(&tls),
            public_tls: Arc::clone(&tls),
            authenticated_tls: tls,
            probe_response: Arc::from(probe_response),
            read_database: None,
            writer: None,
            clock: environment.inner.clock.clone(),
            entropy: Arc::new(foks_server::OsEntropy),
            key_provider: None,
            session_faults: Some(Arc::clone(&environment.inner.session_faults)),
            diagnostics: None,
            metrics: Arc::new(foks_server::ServerMetrics::default()),
            rate_limits: foks_server::RateLimitConfig::default(),
            limits: SessionLimits {
                worker_threads: 4,
                ..SessionLimits::default()
            },
        });
        let server = match result {
            Ok(server) => server,
            Err(error) => {
                *environment
                    .inner
                    .running
                    .lock()
                    .expect("test server lifecycle lock") = false;
                return Err(error);
            }
        };
        Ok(Self {
            environment,
            server: Some(server),
        })
    }

    pub fn shutdown(mut self) -> foks_server::Result<()> {
        self.shutdown_inner()
    }

    fn shutdown_inner(&mut self) -> foks_server::Result<()> {
        let result = self
            .server
            .take()
            .map(foks_server::net::RunningServer::shutdown)
            .unwrap_or(Ok(()));
        *self
            .environment
            .inner
            .running
            .lock()
            .expect("test server lifecycle lock") = false;
        result
    }
}

impl Drop for ProbeOverrideServer {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

pub struct IsolatedTestServer {
    environment: TestEnvironment,
    server: InProcessServer,
}

impl IsolatedTestServer {
    pub fn start() -> foks_server::Result<Self> {
        let environment = TestEnvironment::new()?;
        let server = environment.start_server()?;
        Ok(Self {
            environment,
            server,
        })
    }

    pub fn environment(&self) -> TestEnvironment {
        self.environment.clone()
    }

    pub fn addresses(&self) -> ServerAddresses {
        self.server.addresses()
    }

    pub fn management_address(&self) -> std::net::SocketAddr {
        self.server.management_address()
    }

    pub fn probe_roots(&self) -> rustls::RootCertStore {
        self.server.probe_roots()
    }

    pub fn service_roots(&self) -> rustls::RootCertStore {
        self.server.service_roots()
    }

    pub fn root(&self) -> &Path {
        self.server.root()
    }

    pub fn identity(
        &self,
        uid: &[u8],
    ) -> foks_server_db::Result<Option<foks_server_db::IdentitySnapshot>> {
        self.server.identity(uid)
    }

    pub fn owned_paths(&self) -> [&Path; 4] {
        self.server.owned_paths()
    }

    pub fn backup_named(&self, name: &str) -> foks_server::Result<foks_server::BackupArtifacts> {
        self.server.backup_named(name)
    }

    pub fn backup_is_valid(
        &self,
        artifacts: &foks_server::BackupArtifacts,
    ) -> foks_server_db::Result<bool> {
        self.server.backup_is_valid(artifacts)
    }

    pub fn shutdown(self) -> foks_server::Result<()> {
        self.server.shutdown()
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
