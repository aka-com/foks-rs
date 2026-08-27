use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use foks_server::{Config, RunningServer, ServerAddresses, SessionLimits};

use crate::certs::make_tls;
use crate::config::IsolatedPaths;

const PROBE_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
));

pub struct IsolatedTestServer {
    paths: IsolatedPaths,
    server: RunningServer,
    roots: rustls::RootCertStore,
}

impl IsolatedTestServer {
    pub fn start() -> foks_server::Result<Self> {
        let paths = IsolatedPaths::create()?;
        let tls = make_tls();
        let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let server = foks_server::start(Config {
            probe_address: loopback,
            public_address: loopback,
            authenticated_address: loopback,
            probe_tls: Arc::clone(&tls.public),
            public_tls: tls.public,
            authenticated_tls: tls.authenticated,
            probe_response: Arc::from(PROBE_RESPONSE),
            limits: SessionLimits {
                maximum_requests: 1,
                ..SessionLimits::default()
            },
        })?;
        Ok(Self {
            paths,
            server,
            roots: tls.roots,
        })
    }

    pub fn addresses(&self) -> ServerAddresses {
        self.server.addresses()
    }

    pub fn probe_roots(&self) -> rustls::RootCertStore {
        self.roots.clone()
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
