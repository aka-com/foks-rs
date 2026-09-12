use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
struct DurationMetric {
    observations: AtomicU64,
    microseconds_total: AtomicU64,
    microseconds_max: AtomicU64,
}

impl DurationMetric {
    fn observe(&self, duration: Duration) {
        let microseconds = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        self.observations.fetch_add(1, Ordering::Relaxed);
        self.microseconds_total
            .fetch_add(microseconds, Ordering::Relaxed);
        self.microseconds_max
            .fetch_max(microseconds, Ordering::Relaxed);
    }

    fn snapshot(&self) -> DurationMetricSnapshot {
        DurationMetricSnapshot {
            observations: self.observations.load(Ordering::Relaxed),
            microseconds_total: self.microseconds_total.load(Ordering::Relaxed),
            microseconds_max: self.microseconds_max.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DurationMetricSnapshot {
    observations: u64,
    microseconds_total: u64,
    microseconds_max: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerMetricsSnapshot {
    pub requests_started: u64,
    pub responses_completed: u64,
    pub connections_accepted: u64,
    pub connections_rejected: u64,
    pub active_connections: u64,
    pub active_realtime_polls: u64,
    pub rate_limited_connections: u64,
    pub rate_limited_requests: u64,
    pub request_duration_observations: u64,
    pub request_duration_microseconds_total: u64,
    pub request_duration_microseconds_max: u64,
    pub handler_duration_observations: u64,
    pub handler_duration_microseconds_total: u64,
    pub handler_duration_microseconds_max: u64,
    pub backup_attempts: u64,
    pub backup_successes: u64,
    pub backup_failures: u64,
    pub last_backup_success_unixtime: u64,
    pub backup_duration_observations: u64,
    pub backup_duration_microseconds_total: u64,
    pub backup_duration_microseconds_max: u64,
    pub storage_sample_failures: u64,
}

#[derive(Default)]
pub struct ServerMetrics {
    requests_started: AtomicU64,
    responses_completed: AtomicU64,
    connections_accepted: AtomicU64,
    connections_rejected: AtomicU64,
    active_connections: AtomicU64,
    active_realtime_polls: AtomicU64,
    rate_limited_connections: AtomicU64,
    rate_limited_requests: AtomicU64,
    request_duration: DurationMetric,
    handler_duration: DurationMetric,
    backup_attempts: AtomicU64,
    backup_successes: AtomicU64,
    backup_failures: AtomicU64,
    last_backup_success_unixtime: AtomicU64,
    backup_duration: DurationMetric,
    storage_sample_failures: AtomicU64,
}

impl ServerMetrics {
    pub fn snapshot(&self) -> ServerMetricsSnapshot {
        let request_duration = self.request_duration.snapshot();
        let handler_duration = self.handler_duration.snapshot();
        let backup_duration = self.backup_duration.snapshot();
        ServerMetricsSnapshot {
            requests_started: self.requests_started.load(Ordering::Relaxed),
            responses_completed: self.responses_completed.load(Ordering::Relaxed),
            connections_accepted: self.connections_accepted.load(Ordering::Relaxed),
            connections_rejected: self.connections_rejected.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            active_realtime_polls: self.active_realtime_polls.load(Ordering::Relaxed),
            rate_limited_connections: self.rate_limited_connections.load(Ordering::Relaxed),
            rate_limited_requests: self.rate_limited_requests.load(Ordering::Relaxed),
            request_duration_observations: request_duration.observations,
            request_duration_microseconds_total: request_duration.microseconds_total,
            request_duration_microseconds_max: request_duration.microseconds_max,
            handler_duration_observations: handler_duration.observations,
            handler_duration_microseconds_total: handler_duration.microseconds_total,
            handler_duration_microseconds_max: handler_duration.microseconds_max,
            backup_attempts: self.backup_attempts.load(Ordering::Relaxed),
            backup_successes: self.backup_successes.load(Ordering::Relaxed),
            backup_failures: self.backup_failures.load(Ordering::Relaxed),
            last_backup_success_unixtime: self.last_backup_success_unixtime.load(Ordering::Relaxed),
            backup_duration_observations: backup_duration.observations,
            backup_duration_microseconds_total: backup_duration.microseconds_total,
            backup_duration_microseconds_max: backup_duration.microseconds_max,
            storage_sample_failures: self.storage_sample_failures.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn request_started(&self) {
        self.requests_started.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn request_timer(metrics: Arc<Self>) -> RequestTimer {
        RequestTimer {
            metrics,
            started: Instant::now(),
        }
    }

    pub(crate) fn handler_timer(metrics: Arc<Self>) -> HandlerTimer {
        HandlerTimer {
            metrics,
            started: Instant::now(),
        }
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

    pub(crate) fn realtime_poll_guard(metrics: Arc<Self>) -> RealtimePollGuard {
        metrics
            .active_realtime_polls
            .fetch_add(1, Ordering::Relaxed);
        RealtimePollGuard { metrics }
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

    pub(crate) fn backup_succeeded(&self, unixtime: u64, duration: Duration) {
        self.backup_duration.observe(duration);
        self.last_backup_success_unixtime
            .store(unixtime, Ordering::Relaxed);
        self.backup_successes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn backup_failed(&self, duration: Duration) {
        self.backup_duration.observe(duration);
        self.backup_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn storage_sample_failed(&self) {
        self.storage_sample_failures.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) struct RealtimePollGuard {
    metrics: Arc<ServerMetrics>,
}

impl Drop for RealtimePollGuard {
    fn drop(&mut self) {
        self.metrics
            .active_realtime_polls
            .fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) struct RequestTimer {
    metrics: Arc<ServerMetrics>,
    started: Instant,
}

impl Drop for RequestTimer {
    fn drop(&mut self) {
        self.metrics
            .request_duration
            .observe(self.started.elapsed());
    }
}

pub(crate) struct HandlerTimer {
    metrics: Arc<ServerMetrics>,
    started: Instant,
}

impl Drop for HandlerTimer {
    fn drop(&mut self) {
        self.metrics
            .handler_duration
            .observe(self.started.elapsed());
    }
}
