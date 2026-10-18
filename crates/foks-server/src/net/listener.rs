use std::future::Future;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinSet;

use crate::fatal::{CoreThread, FatalSignal};
use crate::net::retry_accept;
use crate::net::session::{serve, ServerData};
use crate::rate_limit::{SourceConnection, SourceConnections};
use crate::rpc::Listener;
use crate::{Config, Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerAddresses {
    pub probe: SocketAddr,
    pub public_services: SocketAddr,
    pub authenticated: SocketAddr,
}

pub struct RunningServer {
    addresses: ServerAddresses,
    live: Arc<AtomicBool>,
    stop: watch::Sender<bool>,
    thread: Option<JoinHandle<Result<()>>>,
}

pub(crate) struct BoundListeners {
    probe: TcpListener,
    public: TcpListener,
    authenticated: TcpListener,
    addresses: ServerAddresses,
}

#[derive(Clone)]
struct ListenerContext {
    class: Listener,
    tls: Arc<rustls::ServerConfig>,
    service_data: Arc<ServerData>,
    diagnostics: Option<Arc<dyn crate::SessionDiagnostics>>,
    limits: crate::SessionLimits,
    metrics: Arc<crate::ServerMetrics>,
    rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    failure: ListenerFailure,
}

/// Reports the loss of an accept loop or session loop. Readiness and the
/// fatal signal change as soon as an accept loop ends rather than after its
/// listener's sessions drain, which can take as long as the longest open
/// session.
#[derive(Clone)]
struct ListenerFailure {
    live: Arc<AtomicBool>,
    fatal: FatalSignal,
}

impl ListenerFailure {
    fn report(&self) {
        self.live.store(false, Ordering::Release);
        self.fatal.notify(CoreThread::Listener);
    }

    /// A panicked session task ends the listener. Unless a stop was
    /// requested, it is reported when the task is reaped, not when the next
    /// connection arrives.
    fn session_finished(
        &self,
        joined: std::result::Result<(), tokio::task::JoinError>,
        stop: &watch::Receiver<bool>,
    ) -> Result<()> {
        if joined.is_err() {
            if !*stop.borrow() {
                self.report();
            }
            return Err(Error::Thread);
        }
        Ok(())
    }
}

/// Reports every end of an accept loop except a requested stop: an error
/// return, a panic, or the task being dropped.
struct AcceptorExit {
    failure: ListenerFailure,
    stop: watch::Receiver<bool>,
}

impl Drop for AcceptorExit {
    fn drop(&mut self) {
        if !*self.stop.borrow() {
            self.failure.report();
        }
    }
}

impl BoundListeners {
    pub(crate) fn addresses(&self) -> ServerAddresses {
        self.addresses
    }
}

impl RunningServer {
    pub(crate) fn start(config: Config) -> Result<Self> {
        let listeners = bind_addresses(
            config.probe_address,
            config.public_address,
            config.authenticated_address,
        )?;
        Self::start_bound(config, listeners, FatalSignal::default())
    }

    pub(crate) fn start_bound(
        config: Config,
        listeners: BoundListeners,
        fatal: FatalSignal,
    ) -> Result<Self> {
        Self::start_inner(
            config,
            listeners,
            fatal,
            #[cfg(feature = "celld-proof")]
            None,
        )
    }

    #[cfg(feature = "celld-proof")]
    pub(crate) fn start_proof(
        config: Config,
        proof: Arc<dyn crate::proof::RpcProof>,
    ) -> Result<Self> {
        if config.writer.is_some()
            || config.sso.is_some()
            || config.web_admin.is_some()
            || config.chunk_store.is_some()
        {
            return Err(Error::Config(
                "proof startup requires no ordinary writer or secondary services",
            ));
        }
        let listeners = bind_addresses(
            config.probe_address,
            config.public_address,
            config.authenticated_address,
        )?;
        Self::start_inner(config, listeners, FatalSignal::default(), Some(proof))
    }

    fn start_inner(
        config: Config,
        listeners: BoundListeners,
        fatal: FatalSignal,
        #[cfg(feature = "celld-proof")] proof: Option<Arc<dyn crate::proof::RpcProof>>,
    ) -> Result<Self> {
        config.limits.validate()?;
        let startup_storage = config
            .read_database
            .as_ref()
            .map(|database| {
                crate::storage_lease::StorageLease::with_store(
                    &database.path,
                    config.chunk_store.as_deref(),
                    false,
                )
            })
            .transpose()?;
        if let Some(database) = &config.read_database {
            let source_storage = startup_storage
                .as_ref()
                .ok_or(Error::Chunk("missing source storage lease"))?;
            if let Some(writer) = &config.writer {
                source_storage.require_writer(writer)?;
            }
            let reader =
                foks_server_db::ReadDatabase::open(&database.path, database.database.clone())?;
            if config.chunk_store.is_some()
                && reader.chunk_storage_binding()?.is_none_or(|binding| {
                    binding.state != foks_server_db::ChunkStorageBindingState::Active
                })
            {
                let writer = config.writer.as_ref().ok_or(Error::Chunk(
                    "chunk storage initialization requires writer ownership",
                ))?;
                crate::chunk_binding::initialize_online(
                    writer,
                    config.chunk_store.as_deref(),
                    source_storage,
                )?;
            }
            crate::chunk_binding::verify_reader(
                &reader,
                config.chunk_store.as_deref(),
                source_storage,
            )?;
            if let (Some(writer), Some(store)) = (&config.writer, &config.chunk_store) {
                crate::chunk_ledger::initialize_online(writer, store.as_ref())?;
            }
        } else if config.chunk_store.is_some() {
            return Err(Error::Chunk("chunk storage requires a bound database"));
        }
        let BoundListeners {
            probe,
            public,
            authenticated,
            addresses,
        } = listeners;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config.rate_limits)?);
        let service_data = ServerData::from_config(&config, Arc::clone(&rate_limiter))?;
        #[cfg(feature = "celld-proof")]
        let service_data = service_data.with_proof(proof);
        let service_data = Arc::new(service_data);
        let (stop, stop_receiver) = watch::channel(false);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(config.limits.worker_threads)
            .thread_name("foks-server-io")
            .enable_all()
            .build()
            .map_err(Error::Io)?;
        let limits = config.limits;
        let live = Arc::new(AtomicBool::new(false));
        let thread_live = Arc::clone(&live);
        let (startup, started) = std::sync::mpsc::sync_channel(1);
        let exit_stop = stop_receiver.clone();
        let thread = thread::spawn(move || {
            let liveness = LivenessGuard(Arc::clone(&thread_live));
            let exit = fatal.exit_guard(CoreThread::Listener);
            let result = runtime.block_on(run_server(
                probe,
                public,
                authenticated,
                [
                    config.probe_tls,
                    config.public_tls,
                    config.authenticated_tls,
                ],
                service_data,
                config.diagnostics,
                config.metrics,
                rate_limiter,
                limits,
                stop_receiver,
                ListenerFailure {
                    live: thread_live,
                    fatal: fatal.clone(),
                },
                startup,
            ));
            // Report an unrequested exit before the bounded runtime shutdown.
            exit.end(*exit_stop.borrow());
            drop(liveness);
            runtime.shutdown_timeout(limits.request_timeout);
            result
        });
        match started.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(Error::Io(error));
            }
            Err(_) => {
                let _ = thread.join();
                return Err(Error::Thread);
            }
        }
        Ok(Self {
            addresses,
            live,
            stop,
            thread: Some(thread),
        })
    }

    pub fn addresses(&self) -> ServerAddresses {
        self.addresses
    }

    pub(crate) fn liveness(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.live)
    }

    pub fn shutdown(mut self) -> Result<()> {
        self.request_stop();
        self.join()
    }

    fn request_stop(&self) {
        let _ = self.stop.send(true);
    }

    fn join(&mut self) -> Result<()> {
        match self.thread.take() {
            Some(thread) => thread.join().map_err(|_| Error::Thread)?,
            None => Ok(()),
        }
    }
}

/// A connection that passed admission. It holds one connection of its
/// source's allowance on the listener until the session ends, or until it
/// is dropped from a full admission queue.
struct Admitted<S> {
    stream: S,
    id: u64,
    peer_ip: IpAddr,
    _source: SourceConnection,
}

struct LivenessGuard(Arc<AtomicBool>);

impl Drop for LivenessGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.request_stop();
        let _ = self.join();
    }
}

fn bind(address: SocketAddr) -> Result<TcpListener> {
    let listener = TcpListener::bind(address)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

pub(crate) fn bind_addresses(
    probe_address: SocketAddr,
    public_address: SocketAddr,
    authenticated_address: SocketAddr,
) -> Result<BoundListeners> {
    let probe = bind(probe_address)?;
    let public = bind(public_address)?;
    let authenticated = bind(authenticated_address)?;
    let addresses = ServerAddresses {
        probe: probe.local_addr()?,
        public_services: public.local_addr()?,
        authenticated: authenticated.local_addr()?,
    };
    Ok(BoundListeners {
        probe,
        public,
        authenticated,
        addresses,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_server(
    probe: TcpListener,
    public: TcpListener,
    authenticated: TcpListener,
    tls: [Arc<rustls::ServerConfig>; 3],
    service_data: Arc<ServerData>,
    diagnostics: Option<Arc<dyn crate::SessionDiagnostics>>,
    metrics: Arc<crate::ServerMetrics>,
    rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    limits: crate::SessionLimits,
    stop: watch::Receiver<bool>,
    failure: ListenerFailure,
    startup: std::sync::mpsc::SyncSender<std::io::Result<()>>,
) -> Result<()> {
    let probe = listener_from_std(probe, &startup)?;
    let public = listener_from_std(public, &startup)?;
    let authenticated = listener_from_std(authenticated, &startup)?;
    let contexts = [
        ListenerContext {
            class: Listener::Probe,
            tls: Arc::clone(&tls[0]),
            service_data: Arc::clone(&service_data),
            diagnostics: diagnostics.clone(),
            metrics: Arc::clone(&metrics),
            rate_limiter: Arc::clone(&rate_limiter),
            limits,
            failure: failure.clone(),
        },
        ListenerContext {
            class: Listener::PublicServices,
            tls: Arc::clone(&tls[1]),
            service_data: Arc::clone(&service_data),
            diagnostics: diagnostics.clone(),
            metrics: Arc::clone(&metrics),
            rate_limiter: Arc::clone(&rate_limiter),
            limits,
            failure: failure.clone(),
        },
        ListenerContext {
            class: Listener::Authenticated,
            tls: Arc::clone(&tls[2]),
            service_data,
            diagnostics,
            metrics,
            rate_limiter,
            limits,
            failure: failure.clone(),
        },
    ];
    failure.live.store(true, Ordering::Release);
    let _ = startup.send(Ok(()));
    tokio::try_join!(
        run_listener(probe, contexts[0].clone(), stop.clone()),
        run_listener(public, contexts[1].clone(), stop.clone()),
        run_listener(authenticated, contexts[2].clone(), stop),
    )?;
    Ok(())
}

fn listener_from_std(
    listener: TcpListener,
    startup: &std::sync::mpsc::SyncSender<std::io::Result<()>>,
) -> Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::from_std(listener).map_err(|error| {
        let startup_error = std::io::Error::new(error.kind(), error.to_string());
        let _ = startup.send(Err(startup_error));
        Error::Io(error)
    })
}

async fn run_listener(
    listener: tokio::net::TcpListener,
    context: ListenerContext,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let (sender, mut receiver) = mpsc::channel::<Admitted<tokio::net::TcpStream>>(
        context.limits.maximum_pending_connections,
    );
    let listener = Arc::new(listener);
    let acceptor = tokio::spawn(accept_connections(
        move || {
            let listener = Arc::clone(&listener);
            async move { listener.accept().await }
        },
        sender,
        Arc::clone(&context.metrics),
        Arc::clone(&context.rate_limiter),
        SourceConnections::new(context.rate_limiter.config()),
        context.failure.clone(),
        stop.clone(),
    ));

    let active = Arc::new(Semaphore::new(context.limits.maximum_active_connections));
    let mut sessions = JoinSet::new();
    loop {
        let permit = tokio::select! {
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    break;
                }
                continue;
            }
            Some(joined) = sessions.join_next(), if !sessions.is_empty() => {
                context.failure.session_finished(joined, &stop)?;
                continue;
            }
            permit = Arc::clone(&active).acquire_owned() => permit.map_err(|_| Error::Thread)?,
        };
        let Some(admitted) = (tokio::select! {
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    None
                } else {
                    continue;
                }
            }
            Some(joined) = sessions.join_next(), if !sessions.is_empty() => {
                context.failure.session_finished(joined, &stop)?;
                continue;
            }
            connection = receiver.recv() => connection,
        }) else {
            break;
        };
        let session_context = context.clone();
        let session_stop = stop.clone();
        sessions.spawn(async move {
            let _permit = permit;
            let Admitted {
                stream,
                id,
                peer_ip,
                _source,
            } = admitted;
            let _active = ActiveConnectionMetric::new(Arc::clone(&session_context.metrics));
            let result = serve(
                stream,
                peer_ip,
                session_context.class,
                &session_context.tls,
                &session_context.service_data,
                session_context.limits,
                session_stop,
            )
            .await;
            if let (Err(error), Some(diagnostics)) = (result, &session_context.diagnostics) {
                diagnostics.session_failed(
                    session_context.class,
                    id,
                    crate::diagnostics::classify(&error),
                );
            }
        });
    }

    drop(receiver);
    let accept_result = if acceptor.is_finished() {
        acceptor.await.map_err(|_| Error::Thread)?
    } else {
        acceptor.abort();
        let _ = acceptor.await;
        Ok(())
    };
    while let Some(joined) = sessions.join_next().await {
        joined.map_err(|_| Error::Thread)?;
    }
    accept_result
}

/// Accepts connections until `stop` carries `true` or accept() reports an
/// error that invalidates the listening socket. Transient failures, such as
/// EMFILE while the process is at its descriptor limit, are counted and
/// retried by `retry_accept`. Any other end is reported through `failure`
/// as it happens, independently of the session loop, which can be blocked on
/// an active-connection permit while long-lived sessions hold every permit.
/// A connection is closed at once when its source's connection token bucket
/// is empty or the source already holds `maximum_connections_per_source`
/// queued or active connections on this listener.
async fn accept_connections<S, F, Fut>(
    mut accept: F,
    admission: mpsc::Sender<Admitted<S>>,
    metrics: Arc<crate::ServerMetrics>,
    rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    sources: SourceConnections,
    failure: ListenerFailure,
    mut stop: watch::Receiver<bool>,
) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::io::Result<(S, SocketAddr)>>,
{
    let _exit = AcceptorExit {
        failure,
        stop: stop.clone(),
    };
    let mut next_connection_id = 1_u64;
    loop {
        tokio::select! {
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
            accepted = accept() => {
                let (stream, peer) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        retry_accept(error, &metrics).await?;
                        continue;
                    }
                };
                if !rate_limiter.allow_connection(peer.ip()) {
                    metrics.connection_rate_limited();
                    continue;
                }
                let Some(source) = sources.try_admit(peer.ip()) else {
                    metrics.admission_rejected(crate::AdmissionRejection::SourceConnections);
                    continue;
                };
                let id = next_connection_id;
                next_connection_id = next_connection_id.wrapping_add(1);
                // A full admission queue deliberately rejects new work. This keeps
                // memory bounded and lets TCP/TLS clients retry with backoff.
                match admission.try_send(Admitted {
                    stream,
                    id,
                    peer_ip: peer.ip(),
                    _source: source,
                }) {
                    Ok(()) => metrics.connection_accepted(),
                    Err(_) => metrics.connection_rejected(),
                }
            }
        }
    }
}

struct ActiveConnectionMetric(Arc<crate::ServerMetrics>);

impl ActiveConnectionMetric {
    fn new(metrics: Arc<crate::ServerMetrics>) -> Self {
        metrics.connection_opened();
        Self(metrics)
    }
}

impl Drop for ActiveConnectionMetric {
    fn drop(&mut self) {
        self.0.connection_closed();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;
    use crate::net::AcceptErrorKind;

    type Accepted = std::io::Result<(u32, SocketAddr)>;

    /// An accept() replacement that returns `script` in order and then
    /// waits indefinitely, as a listener without pending connections does.
    fn scripted(
        script: Vec<Accepted>,
    ) -> impl FnMut() -> std::pin::Pin<Box<dyn Future<Output = Accepted> + Send>> {
        let script = Arc::new(Mutex::new(VecDeque::from(script)));
        move || {
            let next = script.lock().unwrap().pop_front();
            Box::pin(async move {
                match next {
                    Some(accepted) => accepted,
                    None => std::future::pending().await,
                }
            })
        }
    }

    fn os_error(code: i32) -> Accepted {
        Err(std::io::Error::from_raw_os_error(code))
    }

    struct Harness {
        metrics: Arc<crate::ServerMetrics>,
        rate_limiter: Arc<crate::rate_limit::RateLimiter>,
        failure: ListenerFailure,
        runtime: tokio::runtime::Runtime,
    }

    impl Harness {
        fn sources(&self) -> SourceConnections {
            SourceConnections::new(self.rate_limiter.config())
        }

        fn new() -> Self {
            Self {
                metrics: Arc::new(crate::ServerMetrics::default()),
                rate_limiter: Arc::new(
                    crate::rate_limit::RateLimiter::new(crate::RateLimitConfig::default()).unwrap(),
                ),
                failure: ListenerFailure {
                    live: Arc::new(AtomicBool::new(true)),
                    fatal: FatalSignal::default(),
                },
                runtime: tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
            }
        }
    }

    #[test]
    fn transient_accept_errors_are_counted_and_retried() {
        let harness = Harness::new();
        let peer: SocketAddr = "192.0.2.7:4000".parse().unwrap();
        let (admission, mut admitted) = mpsc::channel(4);
        let (_stop, stop) = watch::channel(false);
        let result = harness.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(10),
                accept_connections(
                    scripted(vec![
                        os_error(libc::EMFILE),
                        os_error(libc::ECONNABORTED),
                        os_error(libc::EMFILE),
                        Ok((7, peer)),
                        os_error(libc::ENOBUFS),
                        Ok((8, peer)),
                        os_error(libc::EBADF),
                    ]),
                    admission,
                    Arc::clone(&harness.metrics),
                    Arc::clone(&harness.rate_limiter),
                    harness.sources(),
                    harness.failure.clone(),
                    stop,
                ),
            )
            .await
        });
        let Ok(Err(Error::Io(error))) = result else {
            panic!("accept loop did not end with the listener error");
        };
        assert_eq!(error.raw_os_error(), Some(libc::EBADF));
        for (stream, id) in [(7, 1), (8, 2)] {
            let connection = admitted.try_recv().unwrap();
            assert_eq!(
                (connection.stream, connection.id, connection.peer_ip),
                (stream, id, peer.ip())
            );
        }
        let snapshot = harness.metrics.snapshot();
        assert_eq!(snapshot.connections_accepted, 2);
        let mut expected = [0; AcceptErrorKind::ALL.len()];
        expected[AcceptErrorKind::ProcessDescriptors as usize] = 2;
        expected[AcceptErrorKind::ConnectionAborted as usize] = 1;
        expected[AcceptErrorKind::Buffers as usize] = 1;
        assert_eq!(snapshot.accept_errors, expected);
        // The accept loop reports its own end; the session loop and its
        // drain are not involved.
        assert!(!harness.failure.live.load(Ordering::Acquire));
        assert_eq!(harness.failure.fatal.exited(), Some(CoreThread::Listener));
    }

    #[test]
    fn requested_stop_ends_the_accept_loop_without_a_failure() {
        let harness = Harness::new();
        let (admission, _admitted) = mpsc::channel(4);
        let (stop, stop_receiver) = watch::channel(false);
        let acceptor = harness.runtime.spawn(accept_connections(
            scripted(vec![os_error(libc::EMFILE)]),
            admission,
            Arc::clone(&harness.metrics),
            Arc::clone(&harness.rate_limiter),
            harness.sources(),
            harness.failure.clone(),
            stop_receiver,
        ));
        stop.send(true).unwrap();
        harness
            .runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(10), acceptor).await })
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(harness.failure.live.load(Ordering::Acquire));
        assert_eq!(harness.failure.fatal.exited(), None);
    }

    #[test]
    fn dropped_accept_loop_is_reported_unless_stop_was_requested() {
        // Dropping the pending future stands in for an aborted acceptor task
        // and for a panic, after which tokio drops the task's future.
        let drop_pending = |harness: &Harness, stop: watch::Receiver<bool>| {
            let (admission, _admitted) = mpsc::channel(4);
            let acceptor = accept_connections(
                scripted(Vec::new()),
                admission,
                Arc::clone(&harness.metrics),
                Arc::clone(&harness.rate_limiter),
                harness.sources(),
                harness.failure.clone(),
                stop,
            );
            harness.runtime.block_on(async {
                let pending = tokio::time::timeout(Duration::from_millis(20), acceptor).await;
                assert!(pending.is_err());
            });
        };

        let harness = Harness::new();
        let (stop, mut stop_receiver) = watch::channel(false);
        stop.send(true).unwrap();
        // Mark the stop as seen, so that the loop is dropped while pending
        // rather than returning.
        stop_receiver.borrow_and_update();
        drop_pending(&harness, stop_receiver);
        assert_eq!(harness.failure.fatal.exited(), None);

        let (_stop, stop_receiver) = watch::channel(false);
        drop_pending(&harness, stop_receiver);
        assert!(!harness.failure.live.load(Ordering::Acquire));
        assert_eq!(harness.failure.fatal.exited(), Some(CoreThread::Listener));
    }

    #[test]
    fn a_source_holds_at_most_its_connection_allowance() {
        let harness = Harness::new();
        let sources = SourceConnections::new(crate::RateLimitConfig {
            maximum_connections_per_source: 2,
            ..crate::RateLimitConfig::default()
        });
        // Addresses in one IPv6 /64 are one source; loopback is exempt.
        let peers = [
            "[2001:db8::1]:4000",
            "[2001:db8::2]:4000",
            "[2001:db8::3]:4000",
            "[2001:db8:0:1::1]:4000",
            "127.0.0.1:4000",
            "127.0.0.1:4001",
            "127.0.0.1:4002",
        ];
        let script = peers
            .iter()
            .enumerate()
            .map(|(stream, peer)| Ok((stream as u32, peer.parse().unwrap())))
            .collect();
        let (admission, mut admitted) = mpsc::channel(16);
        let (stop, stop_receiver) = watch::channel(false);
        let acceptor = harness.runtime.spawn(accept_connections(
            scripted(script),
            admission,
            Arc::clone(&harness.metrics),
            Arc::clone(&harness.rate_limiter),
            sources,
            harness.failure.clone(),
            stop_receiver,
        ));
        let held = harness.runtime.block_on(async {
            let mut held = Vec::new();
            for _ in 0..6 {
                held.push(
                    tokio::time::timeout(Duration::from_secs(10), admitted.recv())
                        .await
                        .unwrap()
                        .unwrap(),
                );
            }
            held
        });
        assert_eq!(
            held.iter().map(|c| c.stream).collect::<Vec<_>>(),
            [0, 1, 3, 4, 5, 6]
        );
        let snapshot = harness.metrics.snapshot();
        assert_eq!(snapshot.connections_accepted, 6);
        let mut expected = [0; crate::AdmissionRejection::ALL.len()];
        expected[crate::AdmissionRejection::SourceConnections as usize] = 1;
        assert_eq!(snapshot.admission_rejections, expected);
        stop.send(true).unwrap();
        harness
            .runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(10), acceptor).await })
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[test]
    fn a_connection_refused_by_a_full_admission_queue_releases_its_source_slot() {
        let harness = Harness::new();
        let sources = SourceConnections::new(crate::RateLimitConfig {
            maximum_connections_per_source: 1,
            ..crate::RateLimitConfig::default()
        });
        let first: SocketAddr = "192.0.2.7:4000".parse().unwrap();
        let second: SocketAddr = "192.0.2.8:4000".parse().unwrap();
        // The first connection fills the queue. Both connections from the
        // second source are refused by the full queue; the second refusal
        // shows that the first released the source's only slot.
        let (admission, mut admitted) = mpsc::channel(1);
        let (stop, stop_receiver) = watch::channel(false);
        let acceptor = harness.runtime.spawn(accept_connections(
            scripted(vec![Ok((1, first)), Ok((2, second)), Ok((3, second))]),
            admission,
            Arc::clone(&harness.metrics),
            Arc::clone(&harness.rate_limiter),
            sources,
            harness.failure.clone(),
            stop_receiver,
        ));
        harness.runtime.block_on(async {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while harness.metrics.snapshot().connections_rejected < 2 {
                assert!(tokio::time::Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            stop.send(true).unwrap();
            tokio::time::timeout(Duration::from_secs(10), acceptor)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        });
        let snapshot = harness.metrics.snapshot();
        assert_eq!(snapshot.connections_accepted, 1);
        let mut expected = [0; crate::AdmissionRejection::ALL.len()];
        expected[crate::AdmissionRejection::AdmissionQueue as usize] = 2;
        assert_eq!(snapshot.admission_rejections, expected);
        assert_eq!(admitted.try_recv().unwrap().stream, 1);
    }
}
