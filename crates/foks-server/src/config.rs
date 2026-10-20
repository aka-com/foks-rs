use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::{Entropy, RateLimitConfig, WriterHandle};

#[derive(Clone)]
pub struct Config {
    pub chunk_store: Option<Arc<dyn crate::chunks::ChunkStore>>,
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
    /// Encoded response/envelope buffers, retained through socket transmission.
    pub maximum_response_memory_bytes: usize,
    /// Conservative codec, database-result and encrypted-transfer allowances.
    /// Includes 16 MiB reserved for small requests. This is a concurrency cap,
    /// not eagerly allocated memory; the minimum admits a maximum-size chunk.
    pub maximum_working_memory_bytes: usize,
    /// Separate bounded capacity for small status/void responses.
    pub maximum_control_memory_bytes: usize,
    pub maximum_requests: usize,
    pub worker_threads: usize,
    pub maximum_read_connections: usize,
    pub maximum_active_connections: usize,
    /// Outstanding anonymous KEX receives. Effective capacity is capped at half
    /// the active connection limit; one active connection admits no receives.
    pub maximum_kex_receive_polls: usize,
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
            maximum_response_memory_bytes: 128 * 1024 * 1024,
            maximum_working_memory_bytes: 512 * 1024 * 1024,
            maximum_control_memory_bytes: 1024 * 1024,
            maximum_requests: 4096,
            worker_threads: 4,
            maximum_read_connections: 32,
            maximum_active_connections: 256,
            maximum_kex_receive_polls: 32,
            maximum_pending_connections: 32,
            maximum_in_flight_requests: 64,
            io_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(30),
        }
    }
}

impl SessionLimits {
    pub(crate) fn kex_receive_capacity(self) -> usize {
        self.maximum_kex_receive_polls
            .min(self.maximum_active_connections / 2)
    }

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
            || self.maximum_response_memory_bytes < 2 * (foks_rpc::DEFAULT_MAX_FRAME_LENGTH + 128)
            || self.maximum_response_memory_bytes > maximum_reservable
            || self.maximum_working_memory_bytes < 512 * 1024 * 1024
            || self.maximum_working_memory_bytes > maximum_reservable
            || self.maximum_control_memory_bytes < 16384
            || self.maximum_control_memory_bytes > maximum_reservable
            || self.maximum_requests == 0
            || self.worker_threads == 0
            || self.maximum_read_connections == 0
            || self.maximum_active_connections == 0
            || self.maximum_pending_connections == 0
            || self.maximum_in_flight_requests < 2
            || self.maximum_active_connections > tokio::sync::Semaphore::MAX_PERMITS
            || self.maximum_kex_receive_polls > tokio::sync::Semaphore::MAX_PERMITS
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
    fn kex_capacity_reserves_ordinary_connections_for_small_configurations() {
        let defaults = SessionLimits::default();
        assert_eq!(defaults.kex_receive_capacity(), 32);
        for (connections, expected) in [(1, 0), (2, 1), (3, 1), (4, 2), (64, 32)] {
            let limits = SessionLimits {
                maximum_active_connections: connections,
                ..defaults
            };
            assert_eq!(limits.kex_receive_capacity(), expected);
        }
        assert_eq!(
            SessionLimits {
                maximum_kex_receive_polls: 1,
                ..defaults
            }
            .kex_receive_capacity(),
            1
        );
        assert_eq!(
            SessionLimits {
                maximum_kex_receive_polls: 0,
                ..defaults
            }
            .kex_receive_capacity(),
            0
        );
        assert!(SessionLimits {
            maximum_kex_receive_polls: tokio::sync::Semaphore::MAX_PERMITS + 1,
            ..defaults
        }
        .validate()
        .is_err());
    }

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
        let single = SessionLimits {
            maximum_in_flight_requests: 1,
            ..SessionLimits::default()
        };
        assert!(single.validate().is_err());
        let too_small = SessionLimits {
            maximum_working_memory_bytes: 512 * 1024 * 1024 - 1,
            ..SessionLimits::default()
        };
        assert!(too_small.validate().is_err());
        let memory = SessionLimits {
            maximum_request_memory_bytes: 2 * foks_rpc::DEFAULT_MAX_FRAME_LENGTH - 1,
            ..SessionLimits::default()
        };
        assert!(memory.validate().is_err());
    }
}
