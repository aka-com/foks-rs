use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinSet;

use crate::net::session::{serve, ServerData};
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
        Self::start_bound(config, listeners)
    }

    pub(crate) fn start_bound(config: Config, listeners: BoundListeners) -> Result<Self> {
        config.limits.validate()?;
        let BoundListeners {
            probe,
            public,
            authenticated,
            addresses,
        } = listeners;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config.rate_limits)?);
        let service_data = Arc::new(ServerData::from_config(&config, Arc::clone(&rate_limiter))?);
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
        let thread = thread::spawn(move || {
            let _liveness = LivenessGuard(Arc::clone(&thread_live));
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
                thread_live,
                startup,
            ));
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
    live: Arc<AtomicBool>,
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
        },
        ListenerContext {
            class: Listener::PublicServices,
            tls: Arc::clone(&tls[1]),
            service_data: Arc::clone(&service_data),
            diagnostics: diagnostics.clone(),
            metrics: Arc::clone(&metrics),
            rate_limiter: Arc::clone(&rate_limiter),
            limits,
        },
        ListenerContext {
            class: Listener::Authenticated,
            tls: Arc::clone(&tls[2]),
            service_data,
            diagnostics,
            metrics,
            rate_limiter,
            limits,
        },
    ];
    live.store(true, Ordering::Release);
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
    let (sender, mut receiver) = mpsc::channel::<(tokio::net::TcpStream, u64, std::net::IpAddr)>(
        context.limits.maximum_pending_connections,
    );
    let next_connection_id = Arc::new(AtomicU64::new(1));
    let accept_ids = Arc::clone(&next_connection_id);
    let mut accept_stop = stop.clone();
    let accept_metrics = Arc::clone(&context.metrics);
    let accept_limiter = Arc::clone(&context.rate_limiter);
    let acceptor = tokio::spawn(async move {
        loop {
            tokio::select! {
                result = accept_stop.changed() => {
                    if result.is_err() || *accept_stop.borrow() {
                        return Ok::<(), Error>(());
                    }
                }
                accepted = listener.accept() => {
                    let (stream, peer) = accepted?;
                    if !accept_limiter.allow_connection(peer.ip()) {
                        accept_metrics.connection_rate_limited();
                        continue;
                    }
                    let id = accept_ids.fetch_add(1, Ordering::Relaxed);
                    // A full admission queue deliberately rejects new work. This keeps
                    // memory bounded and lets TCP/TLS clients retry with backoff.
                    match sender.try_send((stream, id, peer.ip())) {
                        Ok(()) => accept_metrics.connection_accepted(),
                        Err(_) => accept_metrics.connection_rejected(),
                    }
                }
            }
        }
    });

    let active = Arc::new(Semaphore::new(context.limits.maximum_active_connections));
    let mut sessions = JoinSet::new();
    loop {
        while let Some(joined) = sessions.try_join_next() {
            if joined.is_err() {
                return Err(Error::Thread);
            }
        }
        let permit = tokio::select! {
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    break;
                }
                continue;
            }
            permit = Arc::clone(&active).acquire_owned() => permit.map_err(|_| Error::Thread)?,
        };
        let Some((stream, id, peer_ip)) = (tokio::select! {
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    None
                } else {
                    continue;
                }
            }
            connection = receiver.recv() => connection,
        }) else {
            break;
        };
        let session_context = context.clone();
        let session_stop = stop.clone();
        sessions.spawn(async move {
            let _permit = permit;
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
