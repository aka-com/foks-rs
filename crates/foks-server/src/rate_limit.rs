use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Per-source admission limits: the refill rates and bursts of the connection
/// and request token buckets, the number of sources they track, the IPv6
/// prefix length that identifies one source, and the number of connections
/// one source can hold open on each listener. It is also the `[rate_limits]`
/// table of an installation configuration.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitConfig {
    pub connection_burst: u32,
    pub connections_per_second: u32,
    pub request_burst: u32,
    pub requests_per_second: u32,
    /// Sources with a token bucket. The least recently used source is
    /// evicted when a new one arrives at capacity.
    pub maximum_tracked_ips: usize,
    /// Leading bits of an IPv6 address that identify its source, 1 to 128.
    /// An IPv4 address, including an IPv4-mapped IPv6 address, is always one
    /// source.
    pub ipv6_source_prefix: u8,
    /// Accepted and active connections one source can hold on each listener.
    /// Loopback peers are exempt: they are local processes or a local proxy
    /// that the operator controls.
    pub maximum_connections_per_source: usize,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            connection_burst: 512,
            connections_per_second: 256,
            request_burst: 2_000,
            requests_per_second: 1_000,
            maximum_tracked_ips: 4_096,
            ipv6_source_prefix: 64,
            maximum_connections_per_source: 32,
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
            || self.maximum_connections_per_source == 0
        {
            return Err(crate::Error::Config(
                "rate limit parameters, tracked IP capacity and connections per source \
                 must be non-zero",
            ));
        }
        if !(1..=128).contains(&self.ipv6_source_prefix) {
            return Err(crate::Error::Config(
                "ipv6_source_prefix must be between 1 and 128",
            ));
        }
        Ok(())
    }

    pub(crate) fn source(&self, address: IpAddr) -> SourceKey {
        SourceKey::new(address, self.ipv6_source_prefix)
    }
}

/// The address block that admission limits treat as one client: an IPv4
/// address, or an IPv6 address truncated to the configured prefix. A host
/// with an IPv6 prefix can choose any address in it, so keying on the full
/// address would give it one bucket and one connection allowance per
/// address.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceKey(IpAddr);

impl SourceKey {
    /// `ipv6_prefix` is 1 to 128, as `RateLimitConfig::validate` requires.
    fn new(address: IpAddr, ipv6_prefix: u8) -> Self {
        match address.to_canonical() {
            IpAddr::V4(address) => Self(IpAddr::V4(address)),
            IpAddr::V6(address) => {
                let mask = u128::MAX << (128 - u32::from(ipv6_prefix.clamp(1, 128)));
                Self(IpAddr::V6(Ipv6Addr::from(u128::from(address) & mask)))
            }
        }
    }
}

/// Counts concurrent holders per key and refuses a key at its maximum. A
/// key's entry exists only while a permit for it is alive, so memory is
/// bounded by the number of live permits.
pub(crate) struct KeyedLimit<K> {
    maximum: usize,
    held: Mutex<HashMap<K, usize>>,
}

/// A slot of a `KeyedLimit`, released when dropped.
pub(crate) struct KeyedPermit<K: Eq + Hash> {
    limit: Arc<KeyedLimit<K>>,
    key: K,
}

impl<K: Eq + Hash + Clone> KeyedLimit<K> {
    pub(crate) fn new(maximum: usize) -> Self {
        Self {
            maximum,
            held: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn try_acquire(self: &Arc<Self>, key: K) -> Option<KeyedPermit<K>> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        // Checked before insertion, so that a refusal never leaves an entry.
        if held.get(&key).copied().unwrap_or(0) >= self.maximum {
            return None;
        }
        *held.entry(key.clone()).or_default() += 1;
        Some(KeyedPermit {
            limit: Arc::clone(self),
            key,
        })
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl<K: Eq + Hash> Drop for KeyedPermit<K> {
    fn drop(&mut self) {
        let mut held = self
            .limit
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = held.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                held.remove(&self.key);
            }
        }
    }
}

/// The per-source connection allowance of one listener.
pub(crate) struct SourceConnections {
    config: RateLimitConfig,
    open: Arc<KeyedLimit<SourceKey>>,
}

/// Holds one connection of a source's allowance until dropped. A loopback
/// peer holds no slot.
pub(crate) struct SourceConnection {
    _permit: Option<KeyedPermit<SourceKey>>,
}

impl SourceConnections {
    pub(crate) fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            open: Arc::new(KeyedLimit::new(config.maximum_connections_per_source)),
        }
    }

    /// Returns `None` when the peer's source already holds its maximum.
    pub(crate) fn try_admit(&self, address: IpAddr) -> Option<SourceConnection> {
        if address.to_canonical().is_loopback() {
            return Some(SourceConnection { _permit: None });
        }
        self.open
            .try_acquire(self.config.source(address))
            .map(|permit| SourceConnection {
                _permit: Some(permit),
            })
    }
}

pub(crate) struct RateLimiter {
    config: RateLimitConfig,
    clients: Mutex<lru::LruCache<SourceKey, ClientBuckets>>,
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

    pub(crate) fn config(&self) -> RateLimitConfig {
        self.config
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
        let client = clients.get_or_insert_mut(self.config.source(address), || ClientBuckets {
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
    use std::sync::Arc;

    use super::{KeyedLimit, RateLimitConfig, RateLimiter, SourceConnections};

    #[test]
    fn bursts_are_bounded_and_ip_tracking_evicts() {
        let limiter = RateLimiter::new(RateLimitConfig {
            connection_burst: 1,
            connections_per_second: 1,
            request_burst: 1,
            requests_per_second: 1,
            maximum_tracked_ips: 2,
            ..RateLimitConfig::default()
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

    #[test]
    fn ipv6_addresses_share_a_bucket_within_the_source_prefix() {
        let limiter = RateLimiter::new(RateLimitConfig {
            connection_burst: 1,
            connections_per_second: 1,
            ..RateLimitConfig::default()
        })
        .unwrap();
        assert!(limiter.allow_connection("2001:db8:1:2::1".parse().unwrap()));
        assert!(!limiter.allow_connection("2001:db8:1:2:ffff::7".parse().unwrap()));
        assert!(limiter.allow_connection("2001:db8:1:3::1".parse().unwrap()));
        // An IPv4-mapped address is the IPv4 source, not a member of ::/64.
        assert!(limiter.allow_connection("192.0.2.1".parse().unwrap()));
        assert!(!limiter.allow_connection("::ffff:192.0.2.1".parse().unwrap()));
        assert!(limiter.allow_connection("::ffff:192.0.2.2".parse().unwrap()));

        let config = RateLimitConfig {
            ipv6_source_prefix: 48,
            ..RateLimitConfig::default()
        };
        assert_eq!(
            config.source("2001:db8:1:2::1".parse().unwrap()),
            config.source("2001:db8:1:ffff::1".parse().unwrap())
        );
        assert_ne!(
            config.source("2001:db8:1::1".parse().unwrap()),
            config.source("2001:db8:2::1".parse().unwrap())
        );
        let full = RateLimitConfig {
            ipv6_source_prefix: 128,
            ..RateLimitConfig::default()
        };
        assert_ne!(
            full.source("2001:db8::1".parse().unwrap()),
            full.source("2001:db8::2".parse().unwrap())
        );
    }

    #[test]
    fn source_prefix_and_connection_allowance_are_validated() {
        for config in [
            RateLimitConfig {
                ipv6_source_prefix: 0,
                ..RateLimitConfig::default()
            },
            RateLimitConfig {
                ipv6_source_prefix: 129,
                ..RateLimitConfig::default()
            },
            RateLimitConfig {
                maximum_connections_per_source: 0,
                ..RateLimitConfig::default()
            },
        ] {
            assert!(config.validate().is_err(), "{config:?}");
        }
    }

    #[test]
    fn keyed_limit_bounds_each_key_and_forgets_released_keys() {
        let limit = Arc::new(KeyedLimit::new(2));
        let first = limit.try_acquire(1_u8).unwrap();
        let second = limit.try_acquire(1).unwrap();
        assert!(limit.try_acquire(1).is_none());
        let other = limit.try_acquire(2).unwrap();
        drop(first);
        let third = limit.try_acquire(1).unwrap();
        assert!(limit.try_acquire(1).is_none());
        drop((second, third, other));
        assert_eq!(limit.tracked(), 0);
    }

    #[test]
    fn source_connections_cap_a_prefix_and_exempt_loopback() {
        let sources = SourceConnections::new(RateLimitConfig {
            maximum_connections_per_source: 2,
            ..RateLimitConfig::default()
        });
        let first = sources.try_admit("2001:db8::1".parse().unwrap()).unwrap();
        let _second = sources.try_admit("2001:db8::2".parse().unwrap()).unwrap();
        assert!(sources.try_admit("2001:db8::3".parse().unwrap()).is_none());
        let _other = sources
            .try_admit("2001:db8:0:1::1".parse().unwrap())
            .unwrap();
        drop(first);
        assert!(sources.try_admit("2001:db8::3".parse().unwrap()).is_some());
        let loopback = (0..4)
            .flat_map(|_| ["127.0.0.1", "127.0.0.2", "::1", "::ffff:127.0.0.1"])
            .map(|address| sources.try_admit(address.parse().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(loopback.len(), 16);
    }
}
