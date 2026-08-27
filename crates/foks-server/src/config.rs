use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct Config {
    pub probe_address: SocketAddr,
    pub public_address: SocketAddr,
    pub authenticated_address: SocketAddr,
    pub probe_tls: Arc<rustls::ServerConfig>,
    pub public_tls: Arc<rustls::ServerConfig>,
    pub authenticated_tls: Arc<rustls::ServerConfig>,
    pub probe_response: Arc<[u8]>,
    pub limits: SessionLimits,
}

#[derive(Clone, Copy, Debug)]
pub struct SessionLimits {
    pub maximum_frame_bytes: usize,
    pub maximum_requests: usize,
    pub worker_threads: usize,
    pub maximum_pending_connections: usize,
    pub io_timeout: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            maximum_frame_bytes: foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            maximum_requests: 64,
            worker_threads: 4,
            maximum_pending_connections: 32,
            io_timeout: Duration::from_secs(15),
        }
    }
}
