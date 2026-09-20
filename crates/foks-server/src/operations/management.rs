use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;

use crate::{Error, Result, ServerMetrics, WriterHandle};

const MAXIMUM_REQUEST_BYTES: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAXIMUM_CONNECTIONS: usize = 32;

pub(crate) struct ManagementServer {
    address: SocketAddr,
    ready: Arc<AtomicBool>,
    stop: watch::Sender<bool>,
    thread: Option<JoinHandle<Result<()>>>,
}

struct ManagementState {
    ready: Arc<AtomicBool>,
    service_live: Arc<AtomicBool>,
    writer: WriterHandle,
    readiness_in_flight: Arc<AtomicBool>,
    database_path: PathBuf,
    metrics: Arc<ServerMetrics>,
}

impl ManagementServer {
    pub(crate) fn start(
        address: SocketAddr,
        writer: WriterHandle,
        database_path: PathBuf,
        metrics: Arc<ServerMetrics>,
        service_live: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::validate_address(address)?;
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let ready = Arc::new(AtomicBool::new(false));
        let thread_ready = Arc::clone(&ready);
        let (stop, receiver) = watch::channel(false);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .thread_name("foks-server-management")
            .enable_all()
            .build()
            .map_err(Error::Io)?;
        let thread = thread::spawn(move || {
            let result = runtime.block_on(run(
                listener,
                thread_ready,
                service_live,
                writer,
                database_path,
                metrics,
                receiver,
            ));
            runtime.shutdown_timeout(IO_TIMEOUT);
            result
        });
        Ok(Self {
            address,
            ready,
            stop,
            thread: Some(thread),
        })
    }

    pub(crate) fn validate_address(address: SocketAddr) -> Result<()> {
        if !address.ip().is_loopback() {
            return Err(Error::Config(
                "management listener must remain loopback-only",
            ));
        }
        Ok(())
    }

    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub(crate) fn mark_not_ready(&self) {
        self.ready.store(false, Ordering::Release);
    }

    pub(crate) fn shutdown(mut self) -> Result<()> {
        self.ready.store(false, Ordering::Release);
        let _ = self.stop.send(true);
        match self.thread.take() {
            Some(thread) => thread.join().map_err(|_| Error::Thread)?,
            None => Ok(()),
        }
    }
}

impl Drop for ManagementServer {
    fn drop(&mut self) {
        self.ready.store(false, Ordering::Release);
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn run(
    listener: TcpListener,
    ready: Arc<AtomicBool>,
    service_live: Arc<AtomicBool>,
    writer: WriterHandle,
    database_path: PathBuf,
    metrics: Arc<ServerMetrics>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let permits = Arc::new(Semaphore::new(MAXIMUM_CONNECTIONS));
    let state = Arc::new(ManagementState {
        ready,
        service_live,
        writer,
        readiness_in_flight: Arc::new(AtomicBool::new(false)),
        database_path,
        metrics,
    });
    let mut connections = JoinSet::new();
    loop {
        while let Some(joined) = connections.try_join_next() {
            joined.map_err(|_| Error::Thread)?;
        }
        tokio::select! {
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                    continue;
                };
                connections.spawn(handle(stream, Arc::clone(&state), permit));
            }
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn handle(
    mut stream: tokio::net::TcpStream,
    state: Arc<ManagementState>,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let mut request = Vec::with_capacity(512);
    let read = tokio::time::timeout(IO_TIMEOUT, async {
        loop {
            if request.len() == MAXIMUM_REQUEST_BYTES {
                return Err(std::io::Error::other("management request is too large"));
            }
            let mut bytes = [0; 512];
            let read = stream.read(&mut bytes).await?;
            if read == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
            }
            request.extend_from_slice(&bytes[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                return Ok(());
            }
        }
    })
    .await;
    if !matches!(read, Ok(Ok(()))) {
        return;
    }
    let first_line = request
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let response = match first_line.strip_suffix(b"\r") {
        Some(b"GET /healthz HTTP/1.1") => response(200, "text/plain", "ok\n".to_owned()),
        Some(b"GET /readyz HTTP/1.1") => {
            if readiness(
                Arc::clone(&state.ready),
                Arc::clone(&state.service_live),
                state.writer.clone(),
                Arc::clone(&state.readiness_in_flight),
            )
            .await
            {
                response(200, "text/plain", "ready\n".to_owned())
            } else {
                response(503, "text/plain", "not ready\n".to_owned())
            }
        }
        Some(b"GET /metrics HTTP/1.1") => response(
            200,
            "text/plain; version=0.0.4",
            prometheus(&state.metrics, &state.writer, &state.database_path),
        ),
        Some(line) if !line.starts_with(b"GET ") => {
            response(405, "text/plain", "method not allowed\n".to_owned())
        }
        _ => response(404, "text/plain", "not found\n".to_owned()),
    };
    let _ = tokio::time::timeout(IO_TIMEOUT, async {
        stream.write_all(&response).await?;
        stream.shutdown().await
    })
    .await;
}

async fn readiness(
    ready: Arc<AtomicBool>,
    service_live: Arc<AtomicBool>,
    writer: WriterHandle,
    in_flight: Arc<AtomicBool>,
) -> bool {
    if !ready.load(Ordering::Acquire) || !service_live.load(Ordering::Acquire) {
        return false;
    }
    if in_flight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    let guard = ReadinessFlight(in_flight);
    matches!(
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || {
                let _guard = guard;
                writer.call(|database| Ok(database.storage_report()?))
            }),
        )
        .await,
        Ok(Ok(Ok(_)))
    )
}

struct ReadinessFlight(Arc<AtomicBool>);

impl Drop for ReadinessFlight {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn response(status: u16, content_type: &str, body: String) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        503 => "Service Unavailable",
        _ => "Error",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn prometheus(metrics: &ServerMetrics, writer: &WriterHandle, database_path: &Path) -> String {
    let (database_bytes, wal_bytes) = match storage_bytes(database_path) {
        Ok(storage) => storage,
        Err(()) => {
            metrics.storage_sample_failed();
            (0, 0)
        }
    };
    let metrics = metrics.snapshot();
    let writer = writer.metrics();
    let mut output = format!(
        concat!(
            "# TYPE foks_requests_started_total counter\n",
            "foks_requests_started_total {}\n",
            "# TYPE foks_responses_completed_total counter\n",
            "foks_responses_completed_total {}\n",
            "# TYPE foks_connections_accepted_total counter\n",
            "foks_connections_accepted_total {}\n",
            "# TYPE foks_connections_rejected_total counter\n",
            "foks_connections_rejected_total {}\n",
            "# TYPE foks_active_connections gauge\n",
            "foks_active_connections {}\n",
            "# TYPE foks_rate_limited_connections_total counter\n",
            "foks_rate_limited_connections_total {}\n",
            "# TYPE foks_rate_limited_requests_total counter\n",
            "foks_rate_limited_requests_total {}\n",
            "# TYPE foks_request_duration_seconds summary\n",
            "foks_request_duration_seconds_count {}\n",
            "foks_request_duration_seconds_sum {:.6}\n",
            "# TYPE foks_request_duration_seconds_max gauge\n",
            "foks_request_duration_seconds_max {:.6}\n",
            "# TYPE foks_handler_duration_seconds summary\n",
            "foks_handler_duration_seconds_count {}\n",
            "foks_handler_duration_seconds_sum {:.6}\n",
            "# TYPE foks_handler_duration_seconds_max gauge\n",
            "foks_handler_duration_seconds_max {:.6}\n",
            "# TYPE foks_writer_accepted_total counter\n",
            "foks_writer_accepted_total {}\n",
            "# TYPE foks_writer_rejected_total counter\n",
            "foks_writer_rejected_total {}\n",
            "# TYPE foks_writer_pending gauge\n",
            "foks_writer_pending {}\n",
            "# TYPE foks_writer_queue_wait_seconds summary\n",
            "foks_writer_queue_wait_seconds_count {}\n",
            "foks_writer_queue_wait_seconds_sum {:.6}\n",
            "# TYPE foks_writer_queue_wait_seconds_max gauge\n",
            "foks_writer_queue_wait_seconds_max {:.6}\n",
            "# TYPE foks_writer_execution_duration_seconds summary\n",
            "foks_writer_execution_duration_seconds_count {}\n",
            "foks_writer_execution_duration_seconds_sum {:.6}\n",
            "# TYPE foks_writer_execution_duration_seconds_max gauge\n",
            "foks_writer_execution_duration_seconds_max {:.6}\n",
            "# TYPE foks_database_bytes gauge\n",
            "foks_database_bytes {}\n",
            "# TYPE foks_wal_bytes gauge\n",
            "foks_wal_bytes {}\n",
            "# TYPE foks_storage_sample_failures_total counter\n",
            "foks_storage_sample_failures_total {}\n",
            "# TYPE foks_backup_attempts_total counter\n",
            "foks_backup_attempts_total {}\n",
            "# TYPE foks_backup_successes_total counter\n",
            "foks_backup_successes_total {}\n",
            "# TYPE foks_backup_failures_total counter\n",
            "foks_backup_failures_total {}\n",
            "# TYPE foks_last_backup_success_unixtime gauge\n",
            "foks_last_backup_success_unixtime {}\n",
            "# TYPE foks_backup_duration_seconds summary\n",
            "foks_backup_duration_seconds_count {}\n",
            "foks_backup_duration_seconds_sum {:.6}\n",
            "# TYPE foks_backup_duration_seconds_max gauge\n",
            "foks_backup_duration_seconds_max {:.6}\n",
            "# TYPE foks_maintenance_attempts_total counter\n",
            "foks_maintenance_attempts_total {}\n",
            "# TYPE foks_maintenance_successes_total counter\n",
            "foks_maintenance_successes_total {}\n",
            "# TYPE foks_maintenance_failures_total counter\n",
            "foks_maintenance_failures_total {}\n",
            "# TYPE foks_maintenance_consecutive_failures gauge\n",
            "foks_maintenance_consecutive_failures {}\n",
            "# TYPE foks_last_maintenance_success_unixtime gauge\n",
            "foks_last_maintenance_success_unixtime {}\n",
            "# TYPE foks_reclaimed_uploads_total counter\n",
            "foks_reclaimed_uploads_total {}\n",
            "# TYPE foks_reclaimed_upload_chunks_total counter\n",
            "foks_reclaimed_upload_chunks_total {}\n",
            "# TYPE foks_reclaimed_upload_bytes_total counter\n",
            "foks_reclaimed_upload_bytes_total {}\n",
            "# TYPE foks_upload_cleanup_deferred_passes gauge\n",
            "foks_upload_cleanup_deferred_passes {}\n",
            "# TYPE foks_maintenance_warning gauge\n",
            "foks_maintenance_warning {}\n"
        ),
        metrics.requests_started,
        metrics.responses_completed,
        metrics.connections_accepted,
        metrics.connections_rejected,
        metrics.active_connections,
        metrics.rate_limited_connections,
        metrics.rate_limited_requests,
        metrics.request_duration_observations,
        seconds(metrics.request_duration_microseconds_total),
        seconds(metrics.request_duration_microseconds_max),
        metrics.handler_duration_observations,
        seconds(metrics.handler_duration_microseconds_total),
        seconds(metrics.handler_duration_microseconds_max),
        writer.accepted,
        writer.rejected,
        writer.pending,
        writer.queue_wait_observations,
        seconds(writer.queue_wait_microseconds_total),
        seconds(writer.queue_wait_microseconds_max),
        writer.execution_observations,
        seconds(writer.execution_microseconds_total),
        seconds(writer.execution_microseconds_max),
        database_bytes,
        wal_bytes,
        metrics.storage_sample_failures,
        metrics.backup_attempts,
        metrics.backup_successes,
        metrics.backup_failures,
        metrics.last_backup_success_unixtime,
        metrics.backup_duration_observations,
        seconds(metrics.backup_duration_microseconds_total),
        seconds(metrics.backup_duration_microseconds_max),
        metrics.maintenance_attempts,
        metrics.maintenance_successes,
        metrics.maintenance_failures,
        metrics.maintenance_consecutive_failures,
        metrics.last_maintenance_success_unixtime,
        metrics.reclaimed_uploads,
        metrics.reclaimed_upload_chunks,
        metrics.reclaimed_upload_bytes,
        metrics.upload_cleanup_deferred_passes,
        u8::from(
            metrics.maintenance_consecutive_failures >= 3
                || metrics.upload_cleanup_deferred_passes >= 60
        ),
    );
    {
        use std::fmt::Write as _;
        for (name, value) in [
            (
                "foks_admin_cleanup_attempts_total",
                metrics.admin_cleanup_attempts,
            ),
            (
                "foks_admin_cleanup_failures_total",
                metrics.admin_cleanup_failures,
            ),
            (
                "foks_admin_reclaimed_records_total",
                metrics.admin_reclaimed_records,
            ),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
    }
    expiry_prometheus(&mut output, &metrics.expiry);
    checkpoint_prometheus(&mut output, &metrics.checkpoint);
    realtime_prometheus(&mut output, &metrics.realtime);
    // Independent read snapshot; metrics never enqueue a policy mutation.
    let sample = (|| -> crate::Result<Option<foks_server_db::SsoRolloutStatus>> {
        let db = foks_server_db::ReadDatabase::open(database_path, Default::default())?;
        let Some(host) = db.host_bootstrap()? else {
            return Ok(None);
        };
        let host = host
            .host_id
            .as_slice()
            .try_into()
            .map_err(|_| crate::Error::Config("invalid host ID"))?;
        let snapshot = db.snapshot()?;
        Ok(snapshot.sso_rollout_status(host)?)
    })();
    if let Ok(Some(s)) = sample {
        use std::fmt::Write as _;
        for (name, value) in [
            ("foks_oidc_rollout_mode", s.policy.mode as u64 + 1),
            (
                "foks_oidc_provider_fenced",
                u64::from(s.policy.fence.is_some()),
            ),
            (
                "foks_oidc_authorization_epoch",
                s.policy.authorization_epoch,
            ),
            ("foks_oidc_cohort_accounts", s.cohort),
            ("foks_oidc_cohort_linked", s.linked),
            ("foks_oidc_cohort_unlinked", s.unlinked),
            (
                "foks_oidc_cohort_locked_out",
                if s.policy.mode == foks_server_db::SsoRolloutMode::Enforced {
                    s.unlinked
                } else {
                    0
                },
            ),
        ] {
            let _ = writeln!(output, "# TYPE {name} gauge\n{name} {value}");
        }
    }
    output
}

fn expiry_prometheus(output: &mut String, expiry: &crate::ExpiryMetricsSnapshot) {
    use std::fmt::Write;
    for (name, counts) in [
        (
            "foks_expiry_deleted_rows_total",
            &expiry.deleted_parent_rows,
        ),
        ("foks_expiry_empty_passes_total", &expiry.empty_passes),
    ] {
        let _ = writeln!(output, "# TYPE {name} counter");
        for kind in crate::ExpiryKind::ALL {
            let _ = writeln!(
                output,
                "{name}{{kind=\"{}\"}} {}",
                kind.label(),
                counts[kind as usize]
            );
        }
    }
}

fn checkpoint_prometheus(output: &mut String, checkpoint: &crate::CheckpointMetricsSnapshot) {
    use std::fmt::Write as _;
    for (name, value) in [
        ("attempts", checkpoint.attempts),
        ("complete", checkpoint.complete),
        ("deferred", checkpoint.deferred),
        ("unavailable", checkpoint.unavailable),
        ("errors", checkpoint.errors),
        ("busy", checkpoint.busy),
    ] {
        let _ = writeln!(
            output,
            "# TYPE foks_checkpoint_{name}_total counter\nfoks_checkpoint_{name}_total {value}"
        );
    }
    let _ = writeln!(output, "# TYPE foks_checkpoint_duration_seconds histogram");
    for (index, micros) in crate::metrics::CHECKPOINT_BUCKET_MICROS.iter().enumerate() {
        let _ = writeln!(
            output,
            "foks_checkpoint_duration_seconds_bucket{{le=\"{}\"}} {}",
            seconds(*micros),
            checkpoint.duration_buckets[index]
        );
    }
    let _ = writeln!(
        output,
        "foks_checkpoint_duration_seconds_bucket{{le=\"+Inf\"}} {}",
        checkpoint.duration_buckets[8]
    );
    let _ = writeln!(
        output,
        "foks_checkpoint_duration_seconds_count {}",
        checkpoint.duration_buckets[8]
    );
    let _ = writeln!(
        output,
        "foks_checkpoint_duration_seconds_sum {:.6}",
        seconds(checkpoint.duration_microseconds_total)
    );
    for (name, value) in [
        (
            "foks_checkpoint_frame_counts_available",
            u64::from(checkpoint.frame_counts_available),
        ),
        ("foks_checkpoint_wal_frames", checkpoint.wal_frames),
        (
            "foks_checkpoint_backfilled_frames",
            checkpoint.backfilled_frames,
        ),
        (
            "foks_checkpoint_remaining_frames",
            checkpoint.remaining_frames,
        ),
        (
            "foks_last_checkpoint_observation_unixtime",
            checkpoint.last_observation_unixtime,
        ),
        (
            "foks_last_checkpoint_complete_unixtime",
            checkpoint.last_complete_unixtime,
        ),
    ] {
        let _ = writeln!(output, "# TYPE {name} gauge\n{name} {value}");
    }
}

fn seconds(microseconds: u64) -> f64 {
    microseconds as f64 / 1_000_000.0
}

fn storage_bytes(database_path: &Path) -> std::result::Result<(u64, u64), ()> {
    let database_bytes = std::fs::metadata(database_path).map_err(|_| ())?.len();
    let mut wal_path = database_path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let wal_bytes = match std::fs::metadata(PathBuf::from(wal_path)) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(_) => return Err(()),
    };
    Ok((database_bytes, wal_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_export_has_exact_fixed_labels_and_values() {
        let snapshot = crate::ExpiryMetricsSnapshot {
            deleted_parent_rows: [1, 2, 3, 4, 5, 6, 7, 8, 9],
            empty_passes: [9, 8, 7, 6, 5, 4, 3, 2, 1],
        };
        let mut text = String::new();
        expiry_prometheus(&mut text, &snapshot);
        let labels = [
            "names",
            "team_names",
            "recovery_challenges",
            "team_view_tokens",
            "team_view_challenges",
            "team_admin_tokens",
            "sso_sessions",
            "log_sends",
            "request_receipts",
        ];
        assert_eq!(text.lines().count(), 20);
        for (index, label) in labels.iter().enumerate() {
            assert!(text.contains(&format!(
                "foks_expiry_deleted_rows_total{{kind=\"{label}\"}} {}\n",
                index + 1
            )));
            assert!(text.contains(&format!(
                "foks_expiry_empty_passes_total{{kind=\"{label}\"}} {}\n",
                9 - index
            )));
        }
        assert!(text.contains("# TYPE foks_expiry_deleted_rows_total counter\n"));
        assert!(text.contains("# TYPE foks_expiry_empty_passes_total counter\n"));
    }

    #[test]
    fn management_listener_accepts_only_loopback_addresses() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            ManagementServer::validate_address(address.parse().unwrap()).unwrap();
        }
        for address in ["0.0.0.0:0", "[::]:0", "192.0.2.1:9090"] {
            assert!(matches!(
                ManagementServer::validate_address(address.parse().unwrap()),
                Err(Error::Config(
                    "management listener must remain loopback-only"
                ))
            ));
        }
    }

    #[test]
    fn readiness_requires_storage_and_service_liveness() {
        let temporary = tempfile::tempdir().unwrap();
        let database_path = temporary.path().join("readiness.sqlite");
        foks_server_db::Database::open(&database_path, foks_server_db::Config::default()).unwrap();
        let writer =
            crate::Writer::start(database_path, foks_server_db::Config::default(), 1).unwrap();
        let ready = Arc::new(AtomicBool::new(true));
        let service_live = Arc::new(AtomicBool::new(false));
        let in_flight = Arc::new(AtomicBool::new(false));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        assert!(!runtime.block_on(readiness(
            Arc::clone(&ready),
            Arc::clone(&service_live),
            writer.handle(),
            Arc::clone(&in_flight),
        )));
        service_live.store(true, Ordering::Release);
        assert!(runtime.block_on(readiness(
            Arc::clone(&ready),
            Arc::clone(&service_live),
            writer.handle(),
            Arc::clone(&in_flight),
        )));
        service_live.store(false, Ordering::Release);
        assert!(!runtime.block_on(readiness(ready, service_live, writer.handle(), in_flight,)));
        writer.shutdown().unwrap();
    }

    #[test]
    fn timed_out_readiness_probe_does_not_amplify_writer_work() {
        let temporary = tempfile::tempdir().unwrap();
        let writer = Arc::new(
            crate::Writer::start(
                temporary.path().join("readiness-blocked.sqlite"),
                foks_server_db::Config::default(),
                4,
            )
            .unwrap(),
        );
        let (entered_sender, entered_receiver) = std::sync::mpsc::sync_channel(1);
        let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(1);
        let blocked_writer = Arc::clone(&writer);
        let blocked = std::thread::spawn(move || {
            blocked_writer.call(move |_| {
                entered_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                Ok(())
            })
        });
        entered_receiver.recv().unwrap();

        let ready = Arc::new(AtomicBool::new(true));
        let service_live = Arc::new(AtomicBool::new(true));
        let in_flight = Arc::new(AtomicBool::new(false));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(!runtime.block_on(readiness(
            Arc::clone(&ready),
            Arc::clone(&service_live),
            writer.handle(),
            Arc::clone(&in_flight),
        )));
        assert!(in_flight.load(Ordering::Acquire));
        assert_eq!(writer.handle().metrics().pending, 2);

        let before = std::time::Instant::now();
        for _ in 0..8 {
            assert!(!runtime.block_on(readiness(
                Arc::clone(&ready),
                Arc::clone(&service_live),
                writer.handle(),
                Arc::clone(&in_flight),
            )));
        }
        assert!(before.elapsed() < Duration::from_millis(100));
        assert_eq!(writer.handle().metrics().pending, 2);

        release_sender.send(()).unwrap();
        blocked.join().unwrap().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while writer.handle().metrics().pending != 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        Arc::into_inner(writer).unwrap().shutdown().unwrap();
    }
}

fn realtime_prometheus(output: &mut String, realtime: &crate::RealtimeMetricsSnapshot) {
    use std::fmt::Write;
    for (source, s) in [("poll", realtime.poll), ("delta", realtime.delta)] {
        for (name, count) in [
            ("state_reads", s.state_reads),
            ("clean_skips", s.clean_skips),
            ("submissions", s.submissions),
            ("submission_failures", s.submission_failures),
            ("execution_failures", s.execution_failures),
            ("already_clean", s.already_clean),
            ("complete", s.complete),
            ("incomplete", s.incomplete),
            ("restarted", s.restarted),
            ("candidates", s.candidates),
            ("accessibility_changes", s.accessibility_changes),
        ] {
            if source == "poll" {
                let _ = writeln!(
                    output,
                    "# TYPE foks_realtime_reconcile_{name}_total counter"
                );
            }
            let _ = writeln!(
                output,
                "foks_realtime_reconcile_{name}_total{{source=\"{source}\"}} {count}"
            );
        }
        for (name, buckets, total) in [
            (
                "queue_wait",
                s.queue_wait_buckets,
                s.queue_wait_microseconds_total,
            ),
            (
                "execution",
                s.execution_buckets,
                s.execution_microseconds_total,
            ),
        ] {
            let prefix = format!("foks_realtime_reconcile_{name}_seconds");
            if source == "poll" {
                let _ = writeln!(output, "# TYPE {prefix} histogram");
            }
            for (i, bound) in crate::metrics::realtime::BUCKET_MICROS.iter().enumerate() {
                let _ = writeln!(
                    output,
                    "{prefix}_bucket{{source=\"{source}\",le=\"{}\"}} {}",
                    seconds(*bound),
                    buckets[i]
                );
            }
            let _ = writeln!(
                output,
                "{prefix}_bucket{{source=\"{source}\",le=\"+Inf\"}} {}",
                buckets[8]
            );
            let _ = writeln!(
                output,
                "{prefix}_count{{source=\"{source}\"}} {}",
                buckets[8]
            );
            let _ = writeln!(
                output,
                "{prefix}_sum{{source=\"{source}\"}} {:.6}",
                seconds(total)
            );
        }
    }
    for (operation, s) in [
        ("registration", realtime.registration),
        ("recipients", realtime.notification),
        ("membership", realtime.membership),
    ] {
        for (name, count) in [
            ("calls", s.calls),
            ("recipient_keys", s.recipient_keys),
            ("entries_examined", s.entries_examined),
            ("expired_entries_removed", s.expired_entries_removed),
            ("listeners_woken", s.listeners_woken),
        ] {
            if operation == "registration" {
                let _ = writeln!(output, "# TYPE foks_realtime_fanout_{name}_total counter");
            }
            let _ = writeln!(
                output,
                "foks_realtime_fanout_{name}_total{{operation=\"{operation}\"}} {count}"
            );
        }
        for (name, buckets, total) in [
            (
                "mutex_wait",
                s.mutex_wait_buckets,
                s.mutex_wait_nanoseconds_total,
            ),
            (
                "mutex_hold",
                s.mutex_hold_buckets,
                s.mutex_hold_nanoseconds_total,
            ),
            ("wake", s.wake_buckets, s.wake_nanoseconds_total),
        ] {
            let prefix = format!("foks_realtime_fanout_{name}_seconds");
            if operation == "registration" {
                let _ = writeln!(output, "# TYPE {prefix} histogram");
            }
            for (i, bound) in crate::metrics::realtime::BUCKET_MICROS.iter().enumerate() {
                let _ = writeln!(
                    output,
                    "{prefix}_bucket{{operation=\"{operation}\",le=\"{}\"}} {}",
                    seconds(*bound),
                    buckets[i]
                );
            }
            let _ = writeln!(
                output,
                "{prefix}_bucket{{operation=\"{operation}\",le=\"+Inf\"}} {}",
                buckets[8]
            );
            let _ = writeln!(
                output,
                "{prefix}_count{{operation=\"{operation}\"}} {}",
                buckets[8]
            );
            let _ = writeln!(
                output,
                "{prefix}_sum{{operation=\"{operation}\"}} {:.9}",
                total as f64 / 1_000_000_000.0
            );
        }
    }
    for (name, count) in [
        ("hint_wakes", realtime.hint_wakes),
        ("fallback_wakes", realtime.fallback_wakes),
        ("bumped_polls", realtime.bumped_polls),
        ("timed_out_polls", realtime.timed_out_polls),
        ("failed_polls", realtime.failed_polls),
        ("cancelled_polls", realtime.cancelled_polls),
    ] {
        let _ = writeln!(
            output,
            "# TYPE foks_realtime_{name}_total counter\nfoks_realtime_{name}_total {count}"
        );
    }
}

#[cfg(test)]
mod realtime_metric_tests {
    #[test]
    fn fanout_metrics_have_fixed_operations_and_no_recipient_labels() {
        let mut s = crate::RealtimeMetricsSnapshot::default();
        s.notification.calls = 1;
        s.notification.recipient_keys = 2;
        s.notification.entries_examined = 2;
        s.notification.mutex_hold_buckets[8] = 1;
        s.notification.mutex_hold_microseconds_total = 42;
        s.notification.mutex_hold_nanoseconds_total = 42_125;
        s.notification.wake_nanoseconds_total = 300;
        let mut output = String::new();
        super::realtime_prometheus(&mut output, &s);
        assert!(output
            .contains("foks_realtime_fanout_entries_examined_total{operation=\"recipients\"} 2"));
        assert!(output
            .contains("foks_realtime_fanout_mutex_hold_seconds_count{operation=\"recipients\"} 1"));
        assert!(output.contains(
            "foks_realtime_fanout_mutex_hold_seconds_sum{operation=\"recipients\"} 0.000042125"
        ));
        assert!(output.contains(
            "foks_realtime_fanout_wake_seconds_sum{operation=\"recipients\"} 0.000000300"
        ));
        assert!(!output.contains("uid="));
        assert!(!output.contains("host="));
    }
    #[test]
    fn reconciliation_metrics_export_fixed_sources_and_histogram_counts() {
        let mut s = crate::RealtimeMetricsSnapshot::default();
        s.poll.clean_skips = 7;
        s.delta.execution_failures = 1;
        s.delta.execution_buckets[8] = 1;
        s.delta.execution_microseconds_total = 12_345;
        s.cancelled_polls = 2;
        let mut output = String::new();
        super::realtime_prometheus(&mut output, &s);
        assert!(output.contains("foks_realtime_reconcile_clean_skips_total{source=\"poll\"} 7"));
        assert!(
            output.contains("foks_realtime_reconcile_execution_failures_total{source=\"delta\"} 1")
        );
        assert!(output.contains(
            "foks_realtime_reconcile_execution_seconds_bucket{source=\"delta\",le=\"+Inf\"} 1"
        ));
        assert!(
            output.contains("foks_realtime_reconcile_execution_seconds_count{source=\"delta\"} 1")
        );
        assert!(output
            .contains("foks_realtime_reconcile_execution_seconds_sum{source=\"delta\"} 0.012345"));
        assert!(output.contains("foks_realtime_cancelled_polls_total 2"));
    }
}
