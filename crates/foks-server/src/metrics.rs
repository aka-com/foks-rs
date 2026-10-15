mod checkpoint;
mod expiry;
pub use expiry::{ExpiryKind, ExpiryMetricsSnapshot};
pub(crate) mod realtime;
pub use realtime::{FanoutMetricsSnapshot, RealtimeMetricsSnapshot, ReconcileMetricsSnapshot};

pub use checkpoint::CheckpointMetricsSnapshot;
pub(crate) use checkpoint::BUCKET_MICROS as CHECKPOINT_BUCKET_MICROS;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The `family` label of a response written before the request was routed:
/// a per-address rate-limit refusal, an unknown method, a method on the wrong
/// listener or an argument over the method limit.
pub(crate) const UNROUTED: &str = "unrouted";

/// A refused connection or request, labeled `reason` in
/// `foks_admission_rejected_total`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionRejection {
    /// The source's connection token bucket was empty.
    ConnectionRate,
    /// The source already held its maximum connections on the listener.
    SourceConnections,
    /// The listener's queue of accepted connections was full.
    AdmissionQueue,
    /// The source's request token bucket was empty.
    RequestRate,
    /// Every blocking request handler permit was in use.
    RequestHandlers,
    /// The account already held its maximum realtime inbox polls.
    AccountRealtimePolls,
}

impl AdmissionRejection {
    pub const ALL: [Self; 6] = [
        Self::ConnectionRate,
        Self::SourceConnections,
        Self::AdmissionQueue,
        Self::RequestRate,
        Self::RequestHandlers,
        Self::AccountRealtimePolls,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ConnectionRate => "connection_rate",
            Self::SourceConnections => "source_connections",
            Self::AdmissionQueue => "admission_queue",
            Self::RequestRate => "request_rate",
            Self::RequestHandlers => "request_handlers",
            Self::AccountRealtimePolls => "account_realtime_polls",
        }
    }
}

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
    pub reader_pool_exhausted: u64,
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
    pub oidc_sample_failures: u64,
    pub maintenance_attempts: u64,
    pub maintenance_successes: u64,
    pub maintenance_failures: u64,
    pub maintenance_consecutive_failures: u64,
    pub last_maintenance_success_unixtime: u64,
    pub reclaimed_uploads: u64,
    pub reclaimed_upload_chunks: u64,
    pub reclaimed_upload_bytes: u64,
    pub upload_cleanup_deferred_passes: u64,
    /// Transient accept() failures across all listeners, indexed by
    /// `AcceptErrorKind` discriminant.
    pub accept_errors: [u64; crate::AcceptErrorKind::ALL.len()],
    /// Refused connections and requests, indexed by `AdmissionRejection`
    /// discriminant.
    pub admission_rejections: [u64; AdmissionRejection::ALL.len()],
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
    reader_pool_exhausted: AtomicU64,
    /// Written responses keyed by route family and wire status code.
    request_outcomes: Mutex<BTreeMap<(&'static str, u64), u64>>,
    request_duration: DurationMetric,
    handler_duration: DurationMetric,
    backup_attempts: AtomicU64,
    backup_successes: AtomicU64,
    backup_failures: AtomicU64,
    last_backup_success_unixtime: AtomicU64,
    backup_duration: DurationMetric,
    storage_sample_failures: AtomicU64,
    oidc_sample_failures: AtomicU64,
    maintenance_attempts: AtomicU64,
    maintenance_successes: AtomicU64,
    maintenance_failures: AtomicU64,
    maintenance_consecutive_failures: AtomicU64,
    last_maintenance_success_unixtime: AtomicU64,
    reclaimed_uploads: AtomicU64,
    reclaimed_upload_chunks: AtomicU64,
    reclaimed_upload_bytes: AtomicU64,
    upload_cleanup_deferred_passes: AtomicU64,
    accept_errors: [AtomicU64; crate::AcceptErrorKind::ALL.len()],
    admission_rejections: [AtomicU64; AdmissionRejection::ALL.len()],
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
            reader_pool_exhausted: self.reader_pool_exhausted.load(Ordering::Relaxed),
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
            oidc_sample_failures: self.oidc_sample_failures.load(Ordering::Relaxed),
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
            accept_errors: self
                .accept_errors
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
            admission_rejections: self
                .admission_rejections
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
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

    /// Counts a written response. `family` is the route's protocol name, or
    /// [`UNROUTED`] for a request answered without a route, and `status` is
    /// the wire status code, `STATUS_OK` for a success.
    pub(crate) fn response_completed(&self, family: &'static str, status: u64) {
        self.responses_completed.fetch_add(1, Ordering::Relaxed);
        let mut outcomes = self
            .request_outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *outcomes.entry((family, status)).or_default() += 1;
    }

    /// Returns the written responses as `(family, status, count)`, ordered by
    /// family and status code.
    pub(crate) fn request_outcomes(&self) -> Vec<(&'static str, u64, u64)> {
        self.request_outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(&(family, status), &count)| (family, status, count))
            .collect()
    }

    pub(crate) fn connection_accepted(&self) {
        self.connections_accepted.fetch_add(1, Ordering::Relaxed);
    }

    /// Counts a connection refused because the listener's admission queue
    /// was full.
    pub(crate) fn connection_rejected(&self) {
        self.connections_rejected.fetch_add(1, Ordering::Relaxed);
        self.admission_rejected(AdmissionRejection::AdmissionQueue);
    }

    pub(crate) fn admission_rejected(&self, reason: AdmissionRejection) {
        self.admission_rejections[reason as usize].fetch_add(1, Ordering::Relaxed);
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

    pub(crate) fn accept_failed(&self, kind: crate::net::AcceptErrorKind) {
        self.accept_errors[kind as usize].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_rate_limited(&self) {
        self.rate_limited_connections
            .fetch_add(1, Ordering::Relaxed);
        self.admission_rejected(AdmissionRejection::ConnectionRate);
    }

    /// Counts a request refused by the source's request token bucket
    /// (`RequestRate`) or by saturated handler permits (`RequestHandlers`).
    pub(crate) fn request_rate_limited(&self, reason: AdmissionRejection) {
        self.rate_limited_requests.fetch_add(1, Ordering::Relaxed);
        self.admission_rejected(reason);
    }

    /// Counts a request refused because every pooled read connection was in
    /// use. This is separate from the per-address token bucket.
    pub(crate) fn reader_pool_exhausted(&self) {
        self.reader_pool_exhausted.fetch_add(1, Ordering::Relaxed);
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

    pub(crate) fn oidc_sample_failed(&self) {
        self.oidc_sample_failures.fetch_add(1, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_are_counted_by_family_and_status() {
        let metrics = ServerMetrics::default();
        metrics.response_completed("KvStore", foks_rpc::STATUS_OK);
        metrics.response_completed("KvStore", foks_rpc::STATUS_OK);
        metrics.response_completed("KvStore", foks_rpc::STATUS_TX_RETRY_ERROR);
        metrics.response_completed(UNROUTED, foks_rpc::STATUS_RATE_LIMIT_ERROR);
        metrics.reader_pool_exhausted();
        assert_eq!(
            metrics.request_outcomes(),
            [
                ("KvStore", foks_rpc::STATUS_OK, 2),
                ("KvStore", foks_rpc::STATUS_TX_RETRY_ERROR, 1),
                (UNROUTED, foks_rpc::STATUS_RATE_LIMIT_ERROR, 1),
            ]
        );
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.responses_completed, 4);
        assert_eq!(snapshot.reader_pool_exhausted, 1);
        assert_eq!(snapshot.rate_limited_requests, 0);
    }
}
