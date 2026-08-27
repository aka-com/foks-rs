use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use foks_server_db::Database;
use zeroize::Zeroizing;

use crate::host::{bootstrap, BootstrapEndpoints, BootstrapInput, BootstrapState};
use crate::keys::DirectoryKeyProvider;
use crate::net::{bind_addresses, RunningServer, ServerAddresses};
use crate::pki::build_host_tls;
use crate::Writer;
use crate::{Config, OsEntropy, ReadDatabaseConfig, Result, SessionLimits};

pub struct StandaloneConfig {
    pub database_path: PathBuf,
    pub key_directory: PathBuf,
    pub root_key: Zeroizing<[u8; 32]>,
    pub canonical_name: String,
    pub probe_address: SocketAddr,
    pub public_address: SocketAddr,
    pub authenticated_address: SocketAddr,
    pub probe_tls: Arc<rustls::ServerConfig>,
    pub database: foks_server_db::Config,
    pub limits: SessionLimits,
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

    pub fn shutdown(self) -> Result<()> {
        self.server.shutdown()?;
        self.writer.shutdown()
    }
}

pub fn start_standalone(config: StandaloneConfig) -> Result<RunningStandaloneServer> {
    config.limits.validate()?;
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
    let bootstrap = bootstrap(&mut database, keys.as_ref(), &input)?;
    drop(database);
    let database_config = config.database;
    let writer = Writer::start(
        config.database_path.clone(),
        database_config.clone(),
        config.maximum_pending_writes,
    )?;
    let writer_handle = writer.handle();

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
                path: config.database_path,
                database: database_config,
            }),
            writer: Some(writer_handle),
            clock: Arc::new(foks_server_db::SystemClock),
            entropy: Arc::new(OsEntropy),
            key_provider: Some(keys),
            limits: config.limits,
        },
        listeners,
    )?;
    Ok(RunningStandaloneServer {
        server,
        bootstrap,
        delegated_roots: tls.delegated_ca,
        client_roots: tls.client_ca,
        writer,
    })
}
