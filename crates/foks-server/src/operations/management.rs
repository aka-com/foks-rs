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
    format!(
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
            "foks_backup_duration_seconds_max {:.6}\n"
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
    )
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
