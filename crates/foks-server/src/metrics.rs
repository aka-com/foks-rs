mod checkpoint;
mod expiry;
pub use expiry::{ExpiryKind, ExpiryMetricsSnapshot};
pub(crate) mod realtime;
pub use realtime::{FanoutMetricsSnapshot, RealtimeMetricsSnapshot, ReconcileMetricsSnapshot};

pub use checkpoint::CheckpointMetricsSnapshot;
pub(crate) use checkpoint::BUCKET_MICROS as CHECKPOINT_BUCKET_MICROS;

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
    pub checkpoint: CheckpointMetricsSnapshot,
    pub expiry: ExpiryMetricsSnapshot,
    pub realtime: RealtimeMetricsSnapshot,
    pub admin_cleanup_attempts: u64,
    pub admin_cleanup_failures: u64,
    pub admin_reclaimed_records: u64,

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
    pub maintenance_attempts: u64,
    pub maintenance_successes: u64,
    pub maintenance_failures: u64,
    pub maintenance_consecutive_failures: u64,
    pub last_maintenance_success_unixtime: u64,
    pub reclaimed_uploads: u64,
    pub reclaimed_upload_chunks: u64,
    pub reclaimed_upload_bytes: u64,
    pub upload_cleanup_deferred_passes: u64,
}

#[derive(Default)]
pub struct ServerMetrics {
    checkpoint: checkpoint::CheckpointMetrics,
    expiry: expiry::ExpiryMetrics,
    pub(crate) realtime: realtime::RealtimeMetrics,
    admin_cleanup_attempts: AtomicU64,
    admin_cleanup_failures: AtomicU64,
    admin_reclaimed_records: AtomicU64,

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
    maintenance_attempts: AtomicU64,
    maintenance_successes: AtomicU64,
    maintenance_failures: AtomicU64,
    maintenance_consecutive_failures: AtomicU64,
    last_maintenance_success_unixtime: AtomicU64,
    reclaimed_uploads: AtomicU64,
    reclaimed_upload_chunks: AtomicU64,
    reclaimed_upload_bytes: AtomicU64,
    upload_cleanup_deferred_passes: AtomicU64,
}

impl ServerMetrics {
    pub(crate) fn checkpoint_attempted(&self) {
        self.checkpoint.attempted();
    }

    pub(crate) fn checkpoint_finished(
        &self,
        result: &foks_server_db::Result<foks_server_db::CheckpointReport>,
        unixtime: u64,
        duration: Duration,
    ) {
        self.checkpoint.finished(result, unixtime, duration);
    }

    pub(crate) fn admin_cleanup_attempted(&self) {
        self.admin_cleanup_attempts.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn admin_cleanup_failed(&self) {
        self.admin_cleanup_failures.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn admin_cleanup_succeeded(&self, r: &foks_server_db::WebCleanup) {
        self.admin_reclaimed_records.fetch_add(
            r.tickets + r.confirmations + r.sessions + r.nonces + r.audit,
            Ordering::Relaxed,
        );
    }

    pub fn snapshot(&self) -> ServerMetricsSnapshot {
        let request_duration = self.request_duration.snapshot();
        let handler_duration = self.handler_duration.snapshot();
        let backup_duration = self.backup_duration.snapshot();
        ServerMetricsSnapshot {
            checkpoint: self.checkpoint.snapshot(),
            expiry: self.expiry.snapshot(),
            realtime: self.realtime.snapshot(),
            admin_cleanup_attempts: self.admin_cleanup_attempts.load(Ordering::Relaxed),
            admin_cleanup_failures: self.admin_cleanup_failures.load(Ordering::Relaxed),
            admin_reclaimed_records: self.admin_reclaimed_records.load(Ordering::Relaxed),

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
            maintenance_attempts: self.maintenance_attempts.load(Ordering::Relaxed),
            maintenance_successes: self.maintenance_successes.load(Ordering::Relaxed),
            maintenance_failures: self.maintenance_failures.load(Ordering::Relaxed),
            maintenance_consecutive_failures: self
                .maintenance_consecutive_failures
                .load(Ordering::Relaxed),
            last_maintenance_success_unixtime: self
                .last_maintenance_success_unixtime
                .load(Ordering::Relaxed),
            reclaimed_uploads: self.reclaimed_uploads.load(Ordering::Relaxed),
            reclaimed_upload_chunks: self.reclaimed_upload_chunks.load(Ordering::Relaxed),
            reclaimed_upload_bytes: self.reclaimed_upload_bytes.load(Ordering::Relaxed),
            upload_cleanup_deferred_passes: self
                .upload_cleanup_deferred_passes
                .load(Ordering::Relaxed),
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

    pub(crate) fn maintenance_attempted(&self) {
        self.maintenance_attempts.fetch_add(1, Ordering::Relaxed);
    }

    // Called only after the reclamation transaction commits, even when a later
    // checkpoint fails. Committed work must not disappear from the counters.
    pub(crate) fn expiry_reclaimed(&self, report: &foks_server_db::MaintenanceReport) {
        self.expiry.committed(report);
    }

    pub(crate) fn uploads_reclaimed(&self, report: &foks_server_db::MaintenanceReport) {
        self.reclaimed_uploads
            .fetch_add(report.uploads, Ordering::Relaxed);
        self.reclaimed_upload_chunks
            .fetch_add(report.upload_chunks, Ordering::Relaxed);
        self.reclaimed_upload_bytes
            .fetch_add(report.upload_bytes, Ordering::Relaxed);
        if report.upload_cleanup_deferred {
            self.upload_cleanup_deferred_passes
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.upload_cleanup_deferred_passes
                .store(0, Ordering::Relaxed);
        }
    }

    pub(crate) fn maintenance_succeeded(&self, unixtime: u64) {
        self.last_maintenance_success_unixtime
            .store(unixtime, Ordering::Relaxed);
        self.maintenance_successes.fetch_add(1, Ordering::Relaxed);
        self.maintenance_consecutive_failures
            .store(0, Ordering::Relaxed);
    }

    pub(crate) fn maintenance_failed(&self) {
        self.maintenance_failures.fetch_add(1, Ordering::Relaxed);
        self.maintenance_consecutive_failures
            .fetch_add(1, Ordering::Relaxed);
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
