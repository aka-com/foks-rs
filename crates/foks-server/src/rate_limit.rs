use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Per-source admission limits: the refill rates and bursts of the connection
/// and request token buckets, the number of sources they track, the IPv6
/// prefix length that identifies one source, the number of connections one
/// source can hold open on each listener, and the sources exempt from that
/// count. It is also the `[rate_limits]` table of an installation
/// configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
    /// Accepted and active connections one source can hold on each listener;
    /// 0 removes the limit. Loopback peers and `trusted_sources` are exempt.
    pub maximum_connections_per_source: usize,
    /// Addresses or prefixes, such as `192.0.2.10` or `10.0.0.0/8`, of
    /// forwarders that rewrite the client's source address: a load balancer
    /// or TCP proxy that uses source NAT. Behind one, every client shares the
    /// forwarder's address, so these peers are exempt from the per-source
    /// limits that loopback peers are exempt from: the connection count
    /// above and the KEX relay's per-source share. The token buckets still
    /// count such a peer as one source.
    pub trusted_sources: Vec<SourcePrefix>,
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
            trusted_sources: Vec::new(),
        }
    }
}

/// The most `trusted_sources` entries a configuration may list. Each
/// admission checks them in turn.
const MAXIMUM_TRUSTED_SOURCES: usize = 256;

impl RateLimitConfig {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.connection_burst == 0
            || self.connections_per_second == 0
            || self.request_burst == 0
            || self.requests_per_second == 0
            || self.maximum_tracked_ips == 0
        {
            return Err(crate::Error::Config(
                "rate limit parameters and tracked IP capacity must be non-zero",
            ));
        }
        if !(1..=128).contains(&self.ipv6_source_prefix) {
            return Err(crate::Error::Config(
                "ipv6_source_prefix must be between 1 and 128",
            ));
        }
        if self.trusted_sources.len() > MAXIMUM_TRUSTED_SOURCES {
            return Err(crate::Error::Config(
                "trusted_sources lists more than 256 entries",
            ));
        }
        Ok(())
    }

    pub(crate) fn source(&self, address: IpAddr) -> SourceKey {
        SourceKey::new(address, self.ipv6_source_prefix)
    }

    /// Whether `address` is exempt from per-source limits: a loopback peer
    /// or a configured trusted source.
    pub(crate) fn is_exempt(&self, address: IpAddr) -> bool {
        address.to_canonical().is_loopback()
            || self
                .trusted_sources
                .iter()
                .any(|prefix| prefix.contains(address))
    }

    /// The source that per-source limits count `address` against, or `None`
    /// when it is exempt.
    pub(crate) fn limited_source(&self, address: IpAddr) -> Option<SourceKey> {
        (!self.is_exempt(address)).then(|| self.source(address))
    }
}

/// An IPv4 or IPv6 address prefix, written as an address with an optional
/// `/length` (a bare address is a full-length prefix). An IPv4-mapped IPv6
/// prefix of at least 96 bits is its IPv4 prefix, so that it matches the
/// same peers. Bits beyond the length must be zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourcePrefix {
    address: IpAddr,
    length: u8,
}

impl SourcePrefix {
    /// Whether `address`, or the IPv4 address it maps, is in the prefix.
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.address, address.to_canonical()) {
            (IpAddr::V4(prefix), IpAddr::V4(address)) => {
                let mask = prefix_mask_32(self.length);
                u32::from(address) & mask == u32::from(prefix)
            }
            (IpAddr::V6(prefix), IpAddr::V6(address)) => {
                let mask = prefix_mask_128(self.length);
                u128::from(address) & mask == u128::from(prefix)
            }
            _ => false,
        }
    }
}

fn prefix_mask_32(length: u8) -> u32 {
    u32::MAX.checked_shl(32 - u32::from(length)).unwrap_or(0)
}

fn prefix_mask_128(length: u8) -> u128 {
    u128::MAX.checked_shl(128 - u32::from(length)).unwrap_or(0)
}

impl std::str::FromStr for SourcePrefix {
    type Err = crate::Error;

    fn from_str(text: &str) -> crate::Result<Self> {
        const INVALID: crate::Error =
            crate::Error::Config("a trusted source must be an IP address or address/length");
        let (address, length) = match text.split_once('/') {
            Some((address, length)) => (address, Some(length.parse::<u8>().map_err(|_| INVALID)?)),
            None => (text, None),
        };
        let address: IpAddr = address.parse().map_err(|_| INVALID)?;
        let (address, length) = match (address, length) {
            (IpAddr::V4(_), length) => (address, length.unwrap_or(32)),
            (IpAddr::V6(v6), length) => {
                let length = length.unwrap_or(128);
                match v6.to_ipv4_mapped() {
                    Some(v4) if length >= 96 => (IpAddr::V4(v4), length - 96),
                    _ => (address, length),
                }
            }
        };
        let in_range = match address {
            IpAddr::V4(v4) => length <= 32 && u32::from(v4) & !prefix_mask_32(length) == 0,
            IpAddr::V6(v6) => length <= 128 && u128::from(v6) & !prefix_mask_128(length) == 0,
        };
        if !in_range {
            return Err(crate::Error::Config(
                "a trusted source prefix is longer than its address or has bits set beyond its length",
            ));
        }
        Ok(Self { address, length })
    }
}

impl std::fmt::Display for SourcePrefix {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let full = match self.address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if self.length == full {
            write!(formatter, "{}", self.address)
        } else {
            write!(formatter, "{}/{}", self.address, self.length)
        }
    }
}

impl Serialize for SourcePrefix {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SourcePrefix {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
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
    /// `None` when `maximum_connections_per_source` is 0 (no limit).
    open: Option<Arc<KeyedLimit<SourceKey>>>,
    refusals: Mutex<RefusalLog>,
}

/// Throttles the stderr line for refused connections to one per
/// [`REFUSAL_LOG_INTERVAL`] per listener.
#[derive(Default)]
struct RefusalLog {
    logged_at: Option<Instant>,
    unlogged: u64,
}

/// The shortest interval between two stderr lines about connections refused
/// by the per-source limit on one listener.
const REFUSAL_LOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Holds one connection of a source's allowance until dropped. An exempt
/// peer, or any peer when the allowance is unlimited, holds no slot.
pub(crate) struct SourceConnection {
    _permit: Option<KeyedPermit<SourceKey>>,
}

impl SourceConnections {
    pub(crate) fn new(config: RateLimitConfig) -> Self {
        let open = (config.maximum_connections_per_source != 0)
            .then(|| Arc::new(KeyedLimit::new(config.maximum_connections_per_source)));
        Self {
            config,
            open,
            refusals: Mutex::new(RefusalLog::default()),
        }
    }

    /// Returns `None` when the peer's source already holds its maximum. The
    /// caller counts the refusal; a stderr line naming the source is written
    /// at most once a minute, with the number of refusals since the last one.
    pub(crate) fn try_admit(&self, address: IpAddr) -> Option<SourceConnection> {
        let Some(open) = self.open.as_ref() else {
            return Some(SourceConnection { _permit: None });
        };
        let Some(source) = self.config.limited_source(address) else {
            return Some(SourceConnection { _permit: None });
        };
        let admitted = open.try_acquire(source).map(|permit| SourceConnection {
            _permit: Some(permit),
        });
        if admitted.is_none() {
            if let Some(count) = self.count_refusal(Instant::now()) {
                eprintln!(
                    "FOKS connection refused: reason=source_connections, source={}, \
                     maximum_connections_per_source={}, count_since_last_line={count}; \
                     if clients reach this server through a forwarder or NAT that \
                     rewrites their address, list it in trusted_sources",
                    source.0, self.config.maximum_connections_per_source,
                );
            }
        }
        admitted
    }

    /// Counts one refusal and returns the number the next stderr line
    /// reports, or `None` while the interval since the last line runs.
    fn count_refusal(&self, now: Instant) -> Option<u64> {
        let mut log = self.refusals.lock().unwrap_or_else(PoisonError::into_inner);
        log.unlogged = log.unlogged.saturating_add(1);
        if log
            .logged_at
            .is_some_and(|at| now.saturating_duration_since(at) < REFUSAL_LOG_INTERVAL)
        {
            return None;
        }
        log.logged_at = Some(now);
        Some(std::mem::take(&mut log.unlogged))
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

    pub(crate) fn config(&self) -> &RateLimitConfig {
        &self.config
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

    use super::{KeyedLimit, RateLimitConfig, RateLimiter, SourceConnections, SourcePrefix};

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
                trusted_sources: vec!["192.0.2.1".parse().unwrap(); 257],
                ..RateLimitConfig::default()
            },
        ] {
            assert!(config.validate().is_err(), "{config:?}");
        }
        // 0 connections per source means no limit.
        RateLimitConfig {
            maximum_connections_per_source: 0,
            ..RateLimitConfig::default()
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn a_zero_connection_allowance_admits_every_source() {
        let sources = SourceConnections::new(RateLimitConfig {
            maximum_connections_per_source: 0,
            ..RateLimitConfig::default()
        });
        let held = (0..100)
            .map(|_| sources.try_admit("192.0.2.1".parse().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(held.len(), 100);
    }

    #[test]
    fn trusted_sources_are_exempt_from_the_connection_allowance() {
        let sources = SourceConnections::new(RateLimitConfig {
            maximum_connections_per_source: 1,
            trusted_sources: ["192.0.2.10", "10.0.0.0/8", "2001:db8:1::/48"]
                .map(|prefix| prefix.parse().unwrap())
                .to_vec(),
            ..RateLimitConfig::default()
        });
        let exempt = [
            "192.0.2.10",
            "::ffff:192.0.2.10",
            "10.1.2.3",
            "10.255.0.1",
            "2001:db8:1:ffff::1",
        ];
        let held = (0..4)
            .flat_map(|_| exempt)
            .map(|address| sources.try_admit(address.parse().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(held.len(), 20);
        // Neighbouring addresses are still one source each.
        for address in ["192.0.2.11", "11.0.0.1", "2001:db8:2::1"] {
            let _first = sources.try_admit(address.parse().unwrap()).unwrap();
            assert!(
                sources.try_admit(address.parse().unwrap()).is_none(),
                "{address}"
            );
        }
    }

    #[test]
    fn source_prefixes_parse_canonically_and_reject_host_bits() {
        for (text, display) in [
            ("192.0.2.10", "192.0.2.10"),
            ("192.0.2.10/32", "192.0.2.10"),
            ("10.0.0.0/8", "10.0.0.0/8"),
            ("0.0.0.0/0", "0.0.0.0/0"),
            ("2001:db8::/32", "2001:db8::/32"),
            ("::1", "::1"),
            ("::ffff:10.0.0.0/104", "10.0.0.0/8"),
        ] {
            let prefix: SourcePrefix = text.parse().unwrap();
            assert_eq!(prefix.to_string(), display, "{text}");
        }
        for text in [
            "10.0.0.1/8",
            "10.0.0.0/33",
            "2001:db8::/129",
            "2001:db8::1/32",
            "example.test",
            "10.0.0.0/",
            "",
        ] {
            assert!(text.parse::<SourcePrefix>().is_err(), "{text}");
        }
        let all: SourcePrefix = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains("203.0.113.9".parse().unwrap()));
        assert!(!all.contains("2001:db8::1".parse().unwrap()));
        let config: RateLimitConfig =
            toml::from_str("trusted_sources = [\"10.0.0.0/8\"]\n").unwrap();
        assert_eq!(
            toml::to_string(&config).unwrap(),
            toml::to_string(&RateLimitConfig {
                trusted_sources: vec!["10.0.0.0/8".parse().unwrap()],
                ..RateLimitConfig::default()
            })
            .unwrap()
        );
        assert!(toml::from_str::<RateLimitConfig>("trusted_sources = [\"10.0.0.1/8\"]\n").is_err());
    }

    #[test]
    fn refusal_lines_are_throttled_and_carry_the_count() {
        let sources = SourceConnections::new(RateLimitConfig::default());
        let start = std::time::Instant::now();
        assert_eq!(sources.count_refusal(start), Some(1));
        assert_eq!(sources.count_refusal(start), None);
        assert_eq!(sources.count_refusal(start), None);
        assert_eq!(
            sources.count_refusal(start + std::time::Duration::from_secs(61)),
            Some(3)
        );
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
