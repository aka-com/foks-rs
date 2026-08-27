use std::net::IpAddr;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimitConfig {
    pub connection_burst: u32,
    pub connections_per_second: u32,
    pub request_burst: u32,
    pub requests_per_second: u32,
    pub maximum_tracked_ips: usize,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            connection_burst: 512,
            connections_per_second: 256,
            request_burst: 2_000,
            requests_per_second: 1_000,
            maximum_tracked_ips: 4_096,
        }
    }
}

impl RateLimitConfig {
    pub(crate) fn validate(self) -> crate::Result<()> {
        if self.connection_burst == 0
            || self.connections_per_second == 0
            || self.request_burst == 0
            || self.requests_per_second == 0
            || self.maximum_tracked_ips == 0
        {
            return Err(crate::Error::Config("zero rate limit"));
        }
        Ok(())
    }
}

pub(crate) struct RateLimiter {
    config: RateLimitConfig,
    clients: Mutex<lru::LruCache<IpAddr, ClientBuckets>>,
}

struct ClientBuckets {
    connection: Bucket,
    request: Bucket,
}

struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl RateLimiter {
    pub(crate) fn new(config: RateLimitConfig) -> crate::Result<Self> {
        config.validate()?;
        let capacity = NonZeroUsize::new(config.maximum_tracked_ips)
            .ok_or(crate::Error::Config("zero rate limit"))?;
        Ok(Self {
            config,
            clients: Mutex::new(lru::LruCache::new(capacity)),
        })
    }

    pub(crate) fn allow_connection(&self, address: IpAddr) -> bool {
        self.allow(address, LimitKind::Connection)
    }

    pub(crate) fn allow_request(&self, address: IpAddr) -> bool {
        self.allow(address, LimitKind::Request)
    }

    fn allow(&self, address: IpAddr, kind: LimitKind) -> bool {
        let now = Instant::now();
        let Ok(mut clients) = self.clients.lock() else {
            return false;
        };
        let client = clients.get_or_insert_mut(address, || ClientBuckets {
            connection: Bucket::full(self.config.connection_burst, now),
            request: Bucket::full(self.config.request_burst, now),
        });
        match kind {
            LimitKind::Connection => client.connection.take(
                self.config.connection_burst,
                self.config.connections_per_second,
                now,
            ),
            LimitKind::Request => client.request.take(
                self.config.request_burst,
                self.config.requests_per_second,
                now,
            ),
        }
    }
}

impl Bucket {
    fn full(burst: u32, now: Instant) -> Self {
        Self {
            tokens: f64::from(burst),
            updated: now,
        }
    }

    fn take(&mut self, burst: u32, per_second: u32, now: Instant) -> bool {
        self.tokens = (self.tokens
            + now.duration_since(self.updated).as_secs_f64() * f64::from(per_second))
        .min(f64::from(burst));
        self.updated = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

enum LimitKind {
    Connection,
    Request,
}

#[cfg(test)]
mod tests {
    use super::{RateLimitConfig, RateLimiter};

    #[test]
    fn bursts_are_bounded_and_ip_tracking_evicts() {
        let limiter = RateLimiter::new(RateLimitConfig {
            connection_burst: 1,
            connections_per_second: 1,
            request_burst: 1,
            requests_per_second: 1,
            maximum_tracked_ips: 2,
        })
        .unwrap();
        let first = "127.0.0.1".parse().unwrap();
        assert!(limiter.allow_connection(first));
        assert!(!limiter.allow_connection(first));
        assert!(limiter.allow_request(first));
        assert!(!limiter.allow_request(first));
        let second = "127.0.0.2".parse().unwrap();
        assert!(limiter.allow_connection(second));
        assert!(!limiter.allow_connection(first));
        let third = "127.0.0.3".parse().unwrap();
        assert!(limiter.allow_connection(third));
        assert!(!limiter.allow_connection(first));
        assert!(limiter.allow_connection(second));
    }
}
