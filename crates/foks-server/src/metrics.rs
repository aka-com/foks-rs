use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerMetricsSnapshot {
    pub requests_started: u64,
    pub responses_completed: u64,
    pub connections_accepted: u64,
    pub connections_rejected: u64,
    pub active_connections: u64,
    pub rate_limited_connections: u64,
    pub rate_limited_requests: u64,
    pub backup_attempts: u64,
    pub backup_successes: u64,
    pub backup_failures: u64,
    pub last_backup_success_unixtime: u64,
}

#[derive(Default)]
pub struct ServerMetrics {
    requests_started: AtomicU64,
    responses_completed: AtomicU64,
    connections_accepted: AtomicU64,
    connections_rejected: AtomicU64,
    active_connections: AtomicU64,
    rate_limited_connections: AtomicU64,
    rate_limited_requests: AtomicU64,
    backup_attempts: AtomicU64,
    backup_successes: AtomicU64,
    backup_failures: AtomicU64,
    last_backup_success_unixtime: AtomicU64,
}

impl ServerMetrics {
    pub fn snapshot(&self) -> ServerMetricsSnapshot {
        ServerMetricsSnapshot {
            requests_started: self.requests_started.load(Ordering::Relaxed),
            responses_completed: self.responses_completed.load(Ordering::Relaxed),
            connections_accepted: self.connections_accepted.load(Ordering::Relaxed),
            connections_rejected: self.connections_rejected.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            rate_limited_connections: self.rate_limited_connections.load(Ordering::Relaxed),
            rate_limited_requests: self.rate_limited_requests.load(Ordering::Relaxed),
            backup_attempts: self.backup_attempts.load(Ordering::Relaxed),
            backup_successes: self.backup_successes.load(Ordering::Relaxed),
            backup_failures: self.backup_failures.load(Ordering::Relaxed),
            last_backup_success_unixtime: self.last_backup_success_unixtime.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn request_started(&self) {
        self.requests_started.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn response_completed(&self) {
        self.responses_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_accepted(&self) {
        self.connections_accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_rejected(&self) {
        self.connections_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_opened(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_closed(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_rate_limited(&self) {
        self.rate_limited_connections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn request_rate_limited(&self) {
        self.rate_limited_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn backup_attempted(&self) {
        self.backup_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn backup_succeeded(&self, unixtime: u64) {
        self.backup_successes.fetch_add(1, Ordering::Relaxed);
        self.last_backup_success_unixtime
            .store(unixtime, Ordering::Relaxed);
    }

    pub(crate) fn backup_failed(&self) {
        self.backup_failures.fetch_add(1, Ordering::Relaxed);
    }
}
