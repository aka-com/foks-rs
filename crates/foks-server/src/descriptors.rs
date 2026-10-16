//! File-descriptor budget for the standalone server.
//!
//! systemd starts services with a soft RLIMIT_NOFILE of 1,024 unless the unit
//! sets LimitNOFILE. The default session limits need more than that, and a
//! process at its limit fails accept() with EMFILE. The listeners retry that
//! error, but connections are refused until descriptors are released.

use crate::SessionLimits;

/// The probe, public-services and authenticated RPC listeners.
const RPC_LISTENERS: u64 = 3;
/// SQLite keeps the database file and its WAL open for each connection. The
/// shared-memory index is counted per connection as an upper bound.
const DESCRIPTORS_PER_DATABASE_CONNECTION: u64 = 3;
/// SQLite connections outside the read pool: the writer, the `/metrics` OIDC
/// sample reader kept open when OIDC is configured, and the source and
/// destination of one online backup.
const OTHER_DATABASE_CONNECTIONS: u64 = 4;
/// The management, administration and OIDC listeners, each admitting at most
/// 32 concurrent connections.
const HTTP_DESCRIPTORS: u64 = 3 * (1 + 32);
/// Standard streams, lock and key files, runtime event descriptors, and
/// short-lived descriptors such as backup copies and outbound OIDC provider
/// requests.
const BASE_DESCRIPTORS: u64 = 128;

/// An estimate of the descriptors the standalone server holds when every
/// configured connection limit is reached.
pub fn descriptor_budget(limits: &SessionLimits) -> u64 {
    let per_listener = (limits.maximum_active_connections as u64)
        .saturating_add(limits.maximum_pending_connections as u64)
        .saturating_add(1);
    let database_connections =
        (limits.maximum_read_connections as u64).saturating_add(OTHER_DATABASE_CONNECTIONS);
    RPC_LISTENERS
        .saturating_mul(per_listener)
        .saturating_add(DESCRIPTORS_PER_DATABASE_CONNECTION.saturating_mul(database_connections))
        .saturating_add(HTTP_DESCRIPTORS)
        .saturating_add(BASE_DESCRIPTORS)
}

/// The process descriptor limit compared with `descriptor_budget`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DescriptorLimit {
    /// The `descriptor_budget` for the configured session limits.
    pub required: u64,
    /// The soft RLIMIT_NOFILE in effect after `raise_descriptor_limit`.
    pub soft: u64,
    /// The hard RLIMIT_NOFILE, which an unprivileged process cannot raise.
    pub hard: u64,
}

impl DescriptorLimit {
    pub fn sufficient(&self) -> bool {
        self.soft >= self.required
    }
}

/// Raises the soft RLIMIT_NOFILE to the hard limit, as the Go runtime does at
/// startup, and reports the result against the budget for `limits`. When the
/// hard limit is unlimited or the kernel rejects it (macOS caps descriptors
/// per process below RLIM_INFINITY), the soft limit is raised only to the
/// budget. The limit is process-wide, so only the `foks-server` binary calls
/// this; a process that embeds the server keeps control of its own limits.
#[cfg(unix)]
pub fn raise_descriptor_limit(limits: &SessionLimits) -> crate::Result<DescriptorLimit> {
    use nix::sys::resource::{getrlimit, setrlimit, Resource, RLIM_INFINITY};

    let required = descriptor_budget(limits);
    let (soft, hard) = getrlimit(Resource::RLIMIT_NOFILE).map_err(std::io::Error::from)?;
    for target in [hard, required.min(hard)] {
        if target <= soft || target == RLIM_INFINITY {
            continue;
        }
        if setrlimit(Resource::RLIMIT_NOFILE, target, hard).is_ok() {
            break;
        }
    }
    let (soft, hard) = getrlimit(Resource::RLIMIT_NOFILE).map_err(std::io::Error::from)?;
    Ok(DescriptorLimit {
        required,
        soft,
        hard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_exceed_the_systemd_soft_descriptor_limit() {
        let limits = SessionLimits::default();
        assert_eq!(
            descriptor_budget(&limits),
            3 * (256 + 32 + 1) + 3 * (32 + 4) + 3 * 33 + 128
        );
        assert!(descriptor_budget(&limits) > 1024);
        let smaller = SessionLimits {
            maximum_active_connections: 16,
            maximum_pending_connections: 4,
            maximum_read_connections: 4,
            ..SessionLimits::default()
        };
        assert_eq!(
            descriptor_budget(&smaller),
            3 * (16 + 4 + 1) + 3 * (4 + 4) + 3 * 33 + 128
        );
    }

    #[cfg(unix)]
    #[test]
    fn soft_descriptor_limit_is_raised_to_the_hard_limit() {
        use nix::sys::resource::{getrlimit, Resource, RLIM_INFINITY};

        let (before, _) = getrlimit(Resource::RLIMIT_NOFILE).unwrap();
        let limit = raise_descriptor_limit(&SessionLimits::default()).unwrap();
        let (soft, hard) = getrlimit(Resource::RLIMIT_NOFILE).unwrap();
        assert_eq!((limit.soft, limit.hard), (soft, hard));
        assert!(soft >= before);
        if hard != RLIM_INFINITY && cfg!(target_os = "linux") {
            assert_eq!(soft, hard);
        }
        assert_eq!(limit.sufficient(), soft >= limit.required);
    }
}
