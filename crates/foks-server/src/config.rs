use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::{Entropy, RateLimitConfig, WriterHandle};

#[derive(Clone)]
pub struct Config {
    pub vhost_management_host: String,
    pub web_admin: Option<Arc<crate::web_admin::WebAdminService>>,
    pub sso: Option<Arc<crate::sso::SsoService>>,
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
    /// Shared across all listeners. Each frame reserves twice its encoded
    /// length for the input buffer and decoded request representation.
    pub maximum_request_memory_bytes: usize,
    pub maximum_requests: usize,
    pub worker_threads: usize,
    pub maximum_read_connections: usize,
    pub maximum_active_connections: usize,
    pub maximum_pending_connections: usize,
    pub maximum_in_flight_requests: usize,
    pub io_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            maximum_frame_bytes: foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            maximum_request_memory_bytes: 64 * 1024 * 1024,
            maximum_requests: 4096,
            worker_threads: 4,
            maximum_read_connections: 32,
            maximum_active_connections: 256,
            maximum_pending_connections: 32,
            maximum_in_flight_requests: 64,
            io_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(30),
        }
    }
}

impl SessionLimits {
    pub(crate) fn validate(self) -> crate::Result<()> {
        let Some(maximum_frame_memory) = self.maximum_frame_bytes.checked_mul(2) else {
            return Err(crate::Error::Config(
                "maximum frame memory calculation overflow",
            ));
        };
        let maximum_reservable = usize::try_from(u32::MAX)
            .unwrap_or(usize::MAX)
            .min(tokio::sync::Semaphore::MAX_PERMITS);
        if self.maximum_frame_bytes == 0
            || self.maximum_request_memory_bytes < maximum_frame_memory
            || self.maximum_request_memory_bytes > maximum_reservable
            || self.maximum_requests == 0
            || self.worker_threads == 0
            || self.maximum_read_connections == 0
            || self.maximum_active_connections == 0
            || self.maximum_pending_connections == 0
            || self.maximum_in_flight_requests == 0
            || self.maximum_active_connections > tokio::sync::Semaphore::MAX_PERMITS
            || self.maximum_in_flight_requests > tokio::sync::Semaphore::MAX_PERMITS
            || self.io_timeout.is_zero()
            || self.request_timeout.is_zero()
        {
            return Err(crate::Error::Config("invalid session limit"));
        }
        Ok(())
    }
}

/// The advertised authority is configuration, not an arbitrary browser URL.
pub(crate) fn validate_vhost_management_host(value: &str) -> crate::Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    let url = url::Url::parse(&format!("https://{value}"))
        .map_err(|_| crate::Error::Config("invalid vhost management authority"))?;
    if value.len() > 1024
        || value.chars().any(char::is_whitespace)
        || value.contains(['/', '?', '#', '@'])
        || url.host_str().is_none()
        || value
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<u16>().ok())
            .is_none_or(|p| p == 0)
    {
        return Err(crate::Error::Config("invalid vhost management authority"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semaphore_limits_are_rejected_before_runtime_construction() {
        let active = SessionLimits {
            maximum_active_connections: tokio::sync::Semaphore::MAX_PERMITS + 1,
            ..SessionLimits::default()
        };
        assert!(active.validate().is_err());
        let execution = SessionLimits {
            maximum_in_flight_requests: tokio::sync::Semaphore::MAX_PERMITS + 1,
            ..SessionLimits::default()
        };
        assert!(execution.validate().is_err());
        let memory = SessionLimits {
            maximum_request_memory_bytes: 2 * foks_rpc::DEFAULT_MAX_FRAME_LENGTH - 1,
            ..SessionLimits::default()
        };
        assert!(memory.validate().is_err());
    }
}
