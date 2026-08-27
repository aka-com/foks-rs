use std::net::{SocketAddr, TcpListener};
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

impl ManagementServer {
    pub(crate) fn start(
        address: SocketAddr,
        writer: WriterHandle,
        metrics: Arc<ServerMetrics>,
        service_live: Arc<AtomicBool>,
    ) -> Result<Self> {
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
            runtime.block_on(run(
                listener,
                thread_ready,
                service_live,
                writer,
                metrics,
                receiver,
            ))
        });
        Ok(Self {
            address,
            ready,
            stop,
            thread: Some(thread),
        })
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
    metrics: Arc<ServerMetrics>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let permits = Arc::new(Semaphore::new(MAXIMUM_CONNECTIONS));
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
                connections.spawn(handle(
                    stream,
                    Arc::clone(&ready),
                    Arc::clone(&service_live),
                    writer.clone(),
                    Arc::clone(&metrics),
                    permit,
                ));
            }
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn handle(
    mut stream: tokio::net::TcpStream,
    ready: Arc<AtomicBool>,
    service_live: Arc<AtomicBool>,
    writer: WriterHandle,
    metrics: Arc<ServerMetrics>,
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
                Arc::clone(&ready),
                Arc::clone(&service_live),
                writer.clone(),
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
            prometheus(&metrics, &writer),
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
) -> bool {
    if !ready.load(Ordering::Acquire) || !service_live.load(Ordering::Acquire) {
        return false;
    }
    matches!(
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || {
                writer.call(|database| Ok(database.storage_report()?))
            }),
        )
        .await,
        Ok(Ok(Ok(_)))
    )
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

fn prometheus(metrics: &ServerMetrics, writer: &WriterHandle) -> String {
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
            "# TYPE foks_writer_accepted_total counter\n",
            "foks_writer_accepted_total {}\n",
            "# TYPE foks_writer_rejected_total counter\n",
            "foks_writer_rejected_total {}\n",
            "# TYPE foks_writer_pending gauge\n",
            "foks_writer_pending {}\n",
            "# TYPE foks_backup_attempts_total counter\n",
            "foks_backup_attempts_total {}\n",
            "# TYPE foks_backup_successes_total counter\n",
            "foks_backup_successes_total {}\n",
            "# TYPE foks_backup_failures_total counter\n",
            "foks_backup_failures_total {}\n",
            "# TYPE foks_last_backup_success_unixtime gauge\n",
            "foks_last_backup_success_unixtime {}\n"
        ),
        metrics.requests_started,
        metrics.responses_completed,
        metrics.connections_accepted,
        metrics.connections_rejected,
        metrics.active_connections,
        metrics.rate_limited_connections,
        metrics.rate_limited_requests,
        writer.accepted,
        writer.rejected,
        writer.pending,
        metrics.backup_attempts,
        metrics.backup_successes,
        metrics.backup_failures,
        metrics.last_backup_success_unixtime,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_storage_and_service_liveness() {
        let temporary = tempfile::tempdir().unwrap();
        let database_path = temporary.path().join("readiness.sqlite");
        foks_server_db::Database::open(&database_path, foks_server_db::Config::default()).unwrap();
        let writer =
            crate::Writer::start(database_path, foks_server_db::Config::default(), 1).unwrap();
        let ready = Arc::new(AtomicBool::new(true));
        let service_live = Arc::new(AtomicBool::new(false));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        assert!(!runtime.block_on(readiness(
            Arc::clone(&ready),
            Arc::clone(&service_live),
            writer.handle(),
        )));
        service_live.store(true, Ordering::Release);
        assert!(runtime.block_on(readiness(
            Arc::clone(&ready),
            Arc::clone(&service_live),
            writer.handle(),
        )));
        service_live.store(false, Ordering::Release);
        assert!(!runtime.block_on(readiness(ready, service_live, writer.handle())));
        writer.shutdown().unwrap();
    }
}
