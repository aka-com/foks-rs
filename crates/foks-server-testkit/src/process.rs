use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use foks_server::{RunningStandaloneServer, ServerAddresses, SessionLimits, StandaloneConfig};

use crate::certs::make_tls;
use crate::config::IsolatedPaths;

pub struct IsolatedTestServer {
    paths: IsolatedPaths,
    server: RunningStandaloneServer,
    probe_roots: rustls::RootCertStore,
}

impl IsolatedTestServer {
    pub fn start() -> foks_server::Result<Self> {
        let paths = IsolatedPaths::create()?;
        let tls = make_tls();
        let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let server = foks_server::start_standalone(StandaloneConfig {
            database_path: paths.database().to_path_buf(),
            key_directory: paths.keys().to_path_buf(),
            root_key: zeroize::Zeroizing::new([0x51; 32]),
            canonical_name: "localhost".to_owned(),
            probe_address: loopback,
            public_address: loopback,
            authenticated_address: loopback,
            probe_tls: Arc::clone(&tls.public),
            database: foks_server_db::Config::default(),
            limits: SessionLimits::default(),
            ttl_seconds: 60,
            now_microseconds: 1_700_000_000_000_000,
            maximum_pending_writes: 16,
        })?;
        Ok(Self {
            paths,
            server,
            probe_roots: tls.roots,
        })
    }

    pub fn addresses(&self) -> ServerAddresses {
        self.server.addresses()
    }

    pub fn probe_roots(&self) -> rustls::RootCertStore {
        self.probe_roots.clone()
    }

    pub fn service_roots(&self) -> rustls::RootCertStore {
        self.server.delegated_roots()
    }

    pub fn root(&self) -> &Path {
        self.paths.root()
    }

    pub fn owned_paths(&self) -> [&Path; 4] {
        self.paths.all()
    }

    pub fn shutdown(self) -> foks_server::Result<()> {
        self.server.shutdown()
    }
}
