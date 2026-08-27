use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::net::session::serve;
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

impl RunningServer {
    pub(crate) fn start(config: Config) -> Result<Self> {
        if config.limits.maximum_frame_bytes == 0
            || config.limits.maximum_requests == 0
            || config.limits.worker_threads == 0
            || config.limits.maximum_pending_connections == 0
        {
            return Err(Error::Config("zero session limit"));
        }
        let probe = bind(config.probe_address)?;
        let public = bind(config.public_address)?;
        let authenticated = bind(config.authenticated_address)?;
        let addresses = ServerAddresses {
            probe: probe.local_addr()?,
            public_services: public.local_addr()?,
            authenticated: authenticated.local_addr()?,
        };
        let stopping = Arc::new(AtomicBool::new(false));
        let threads = vec![
            spawn_listener(
                probe,
                Listener::Probe,
                config.probe_tls,
                Arc::clone(&config.probe_response),
                config.limits,
                Arc::clone(&stopping),
            ),
            spawn_listener(
                public,
                Listener::PublicServices,
                config.public_tls,
                Arc::clone(&config.probe_response),
                config.limits,
                Arc::clone(&stopping),
            ),
            spawn_listener(
                authenticated,
                Listener::Authenticated,
                config.authenticated_tls,
                config.probe_response,
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

fn spawn_listener(
    listener: TcpListener,
    class: Listener,
    tls: Arc<rustls::ServerConfig>,
    probe_response: Arc<[u8]>,
    limits: crate::SessionLimits,
    stopping: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let (sender, receiver) = mpsc::sync_channel(limits.maximum_pending_connections);
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..limits.worker_threads)
            .map(|_| {
                spawn_worker(
                    Arc::clone(&receiver),
                    class,
                    Arc::clone(&tls),
                    Arc::clone(&probe_response),
                    limits,
                )
            })
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
        for worker in workers {
            let _ = worker.join();
        }
    })
}

fn spawn_worker(
    receiver: Arc<Mutex<mpsc::Receiver<std::net::TcpStream>>>,
    class: Listener,
    tls: Arc<rustls::ServerConfig>,
    probe_response: Arc<[u8]>,
    limits: crate::SessionLimits,
) -> JoinHandle<()> {
    thread::spawn(move || loop {
        let stream = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => break,
        };
        let Ok(stream) = stream else {
            break;
        };
        let _ = serve(stream, class, &tls, &probe_response, limits);
    })
}
