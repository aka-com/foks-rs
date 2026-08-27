use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::{Entropy, RateLimitConfig, WriterHandle};

#[derive(Clone)]
pub struct Config {
    pub probe_address: SocketAddr,
    pub public_address: SocketAddr,
    pub authenticated_address: SocketAddr,
    pub probe_tls: Arc<rustls::ServerConfig>,
    pub public_tls: Arc<rustls::ServerConfig>,
    pub authenticated_tls: Arc<rustls::ServerConfig>,
    pub probe_response: Arc<[u8]>,
    pub read_database: Option<ReadDatabaseConfig>,
    pub writer: Option<WriterHandle>,
    pub clock: Arc<dyn foks_server_db::Clock>,
    pub entropy: Arc<dyn Entropy>,
    pub key_provider: Option<Arc<dyn crate::keys::HostKeyProvider>>,
    #[doc(hidden)]
    pub session_faults: Option<Arc<crate::SessionFaults>>,
    pub diagnostics: Option<Arc<dyn crate::SessionDiagnostics>>,
    pub metrics: Arc<crate::ServerMetrics>,
    pub rate_limits: RateLimitConfig,
    pub limits: SessionLimits,
}

#[derive(Clone)]
pub struct ReadDatabaseConfig {
    pub path: PathBuf,
    pub database: foks_server_db::Config,
}

#[derive(Clone, Copy, Debug)]
pub struct SessionLimits {
    pub maximum_frame_bytes: usize,
    pub maximum_requests: usize,
    pub worker_threads: usize,
    pub maximum_read_connections: usize,
    pub maximum_active_connections: usize,
    pub maximum_pending_connections: usize,
    pub io_timeout: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            maximum_frame_bytes: foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            maximum_requests: 4096,
            worker_threads: 4,
            maximum_read_connections: 32,
            maximum_active_connections: 256,
            maximum_pending_connections: 32,
            io_timeout: Duration::from_secs(15),
        }
    }
}

impl SessionLimits {
    pub(crate) fn validate(self) -> crate::Result<()> {
        if self.maximum_frame_bytes == 0
            || self.maximum_requests == 0
            || self.worker_threads == 0
            || self.maximum_read_connections == 0
            || self.maximum_active_connections == 0
            || self.maximum_pending_connections == 0
            || self.io_timeout.is_zero()
        {
            return Err(crate::Error::Config("zero session limit"));
        }
        Ok(())
    }
}
