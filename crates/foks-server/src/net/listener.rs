use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
    stopping: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

pub(crate) struct BoundListeners {
    probe: TcpListener,
    public: TcpListener,
    authenticated: TcpListener,
    addresses: ServerAddresses,
}

struct ActiveConnection {
    id: u64,
    stream: TcpStream,
}

struct WorkerContext {
    class: Listener,
    tls: Arc<rustls::ServerConfig>,
    service_data: Arc<ServerData>,
    limits: crate::SessionLimits,
    stopping: Arc<AtomicBool>,
    active: Arc<Mutex<Vec<ActiveConnection>>>,
    next_connection_id: AtomicU64,
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
        let service_data = Arc::new(ServerData::from_probe(
            Arc::clone(&config.probe_response),
            config.read_database,
            config.writer,
            config.clock,
            config.entropy,
            config.key_provider,
        )?);
        let stopping = Arc::new(AtomicBool::new(false));
        let threads = vec![
            spawn_listener(
                probe,
                Listener::Probe,
                config.probe_tls,
                Arc::clone(&service_data),
                config.limits,
                Arc::clone(&stopping),
            ),
            spawn_listener(
                public,
                Listener::PublicServices,
                config.public_tls,
                Arc::clone(&service_data),
                config.limits,
                Arc::clone(&stopping),
            ),
            spawn_listener(
                authenticated,
                Listener::Authenticated,
                config.authenticated_tls,
                service_data,
                config.limits,
                Arc::clone(&stopping),
            ),
        ];
        Ok(Self {
            addresses,
            stopping,
            threads,
        })
    }

    pub fn addresses(&self) -> ServerAddresses {
        self.addresses
    }

    pub fn shutdown(mut self) -> Result<()> {
        self.stopping.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            thread.join().map_err(|_| Error::Thread)?;
        }
        Ok(())
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
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

fn spawn_listener(
    listener: TcpListener,
    class: Listener,
    tls: Arc<rustls::ServerConfig>,
    service_data: Arc<ServerData>,
    limits: crate::SessionLimits,
    stopping: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let (sender, receiver) = mpsc::sync_channel(limits.maximum_pending_connections);
        let receiver = Arc::new(Mutex::new(receiver));
        let active = Arc::new(Mutex::new(Vec::<ActiveConnection>::new()));
        let context = Arc::new(WorkerContext {
            class,
            tls,
            service_data,
            limits,
            stopping: Arc::clone(&stopping),
            active: Arc::clone(&active),
            next_connection_id: AtomicU64::new(1),
        });
        let workers = (0..limits.worker_threads)
            .map(|_| spawn_worker(Arc::clone(&receiver), Arc::clone(&context)))
            .collect::<Vec<_>>();
        while !stopping.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    if stream.set_nonblocking(false).is_ok() {
                        let _ = sender.try_send(stream);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
        drop(sender);
        if let Ok(active) = active.lock() {
            for connection in active.iter() {
                let _ = connection.stream.shutdown(Shutdown::Both);
            }
        }
        for worker in workers {
            let _ = worker.join();
        }
    })
}

fn spawn_worker(
    receiver: Arc<Mutex<mpsc::Receiver<std::net::TcpStream>>>,
    context: Arc<WorkerContext>,
) -> JoinHandle<()> {
    thread::spawn(move || loop {
        let stream = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => break,
        };
        let Ok(stream) = stream else {
            break;
        };
        if context.stopping.load(Ordering::Acquire) {
            break;
        }
        let id = context.next_connection_id.fetch_add(1, Ordering::Relaxed);
        let control = match stream.try_clone() {
            Ok(stream) => stream,
            Err(_) => continue,
        };
        let registered = context.active.lock().map(|mut active| {
            active.push(ActiveConnection {
                id,
                stream: control,
            });
        });
        if registered.is_err() {
            break;
        }
        if context.stopping.load(Ordering::Acquire) {
            if let Ok(active) = context.active.lock() {
                if let Some(connection) = active.iter().find(|connection| connection.id == id) {
                    let _ = connection.stream.shutdown(Shutdown::Both);
                }
            }
        }
        let _ = serve(
            stream,
            context.class,
            &context.tls,
            &context.service_data,
            context.limits,
        );
        if let Ok(mut active) = context.active.lock() {
            active.retain(|connection| connection.id != id);
        }
    })
}
