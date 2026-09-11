//! Concrete TCP, TLS, and Snowpack RPC transport.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use foks_crypto::{device_signing_key_pkcs8, prefixed_hash};
use foks_proto::SecretSeed;
use foks_rpc::{
    call_protocol_id, is_headerless_result_protocol, read_bare_response, read_bare_void_response,
    read_probe_response, read_response, read_void_response, resequence_call, write_probe_request,
    DEFAULT_MAX_FRAME_LENGTH,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use zeroize::Zeroizing;

use crate::{authenticated_tls_roots, DeviceCredential, Error, PinnedHost, ProbeTarget, Result};

struct TlsClientCredential<'a> {
    signing_key_pkcs8: Zeroizing<Vec<u8>>,
    certificate_chain: &'a [Vec<u8>],
}

const AUTH_POOL_KEY_TYPE_ID: u64 = 0x36b2_da48_2f67_b8a4;
const PRIVATE_KEY_POOL_KEY_TYPE_ID: u64 = 0xa322_efbc_1d63_8e62;
const TRUST_POOL_KEY_TYPE_ID: u64 = 0x45cc_9506_2626_ded6;
const DEFAULT_IO_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_MAX_IDLE_PER_ENDPOINT: usize = 2;
const DEFAULT_MAX_IDLE_CONNECTIONS: usize = 16;
pub const MAX_CONFIGURABLE_FRAME_LENGTH: usize = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
struct OperationControl {
    deadline: Instant,
    cancellation: CancellationToken,
    poll_interval: Duration,
}

#[derive(Debug)]
struct CancelledIo;

impl std::fmt::Display for CancelledIo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FOKS operation cancelled")
    }
}

impl std::error::Error for CancelledIo {}

impl OperationControl {
    fn remaining(&self) -> std::io::Result<Duration> {
        if self.cancellation.is_cancelled() {
            return Err(std::io::Error::other(CancelledIo));
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "FOKS operation deadline exceeded",
                )
            })
    }

    fn next_timeout(&self) -> std::io::Result<Duration> {
        Ok(self.remaining()?.min(self.poll_interval))
    }
}

pub(crate) struct ControlledTcpStream {
    inner: TcpStream,
    control: OperationControl,
}

impl ControlledTcpStream {
    fn set_control(&mut self, control: OperationControl) {
        self.control = control;
    }

    /// True when the peer has not closed this idle socket and nothing is
    /// already pending on it.
    ///
    /// A server closes a kept-alive session once its own idle timeout
    /// elapses. Without this check the close is discovered only when the next
    /// request fails mid-flight, which cannot be retried safely because the
    /// request may already have been processed. Probing before reuse keeps
    /// every RPC exactly-once: a dead connection is replaced before anything
    /// is written. Operations that idle for a long time between calls -- a
    /// hardware-backed credential waiting on a person, most of all -- would
    /// otherwise fail on their first call after the pause.
    fn is_reusable(&self) -> bool {
        if self.inner.set_nonblocking(true).is_err() {
            return false;
        }
        let mut probe = [0_u8; 1];
        let live = matches!(
            self.inner.peek(&mut probe),
            // Nothing readable: an ordinary idle keep-alive connection.
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        );
        // Zero bytes is a clean peer close and any already-readable byte on an
        // idle connection is a close_notify or a desynchronized stream. Both
        // are discarded, as is a socket that cannot be restored to blocking
        // mode, because the per-operation read and write timeouts require it.
        self.inner.set_nonblocking(false).is_ok() && live
    }
}

impl Read for ControlledTcpStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            self.inner
                .set_read_timeout(Some(self.control.next_timeout()?))?;
            match self.inner.read(buffer) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    self.control.remaining()?;
                }
                result => return result,
            }
        }
    }
}

impl Write for ControlledTcpStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        loop {
            self.inner
                .set_write_timeout(Some(self.control.next_timeout()?))?;
            match self.inner.write(buffer) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    self.control.remaining()?;
                }
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.control.remaining()?;
        self.inner.flush()
    }
}

pub(crate) type TlsStream = rustls::StreamOwned<rustls::ClientConnection, ControlledTcpStream>;

#[derive(Clone, Eq)]
struct PoolKey {
    hostname: String,
    port: u16,
    host_id: Vec<u8>,
    authentication: [u8; 32],
    trust: [u8; 32],
    selection_protocol: Option<u64>,
}

impl PartialEq for PoolKey {
    fn eq(&self, other: &Self) -> bool {
        self.hostname == other.hostname
            && self.port == other.port
            && self.host_id == other.host_id
            && self.authentication == other.authentication
            && self.trust == other.trust
            && self.selection_protocol == other.selection_protocol
    }
}

impl Hash for PoolKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hostname.hash(state);
        self.port.hash(state);
        self.host_id.hash(state);
        self.authentication.hash(state);
        self.trust.hash(state);
        self.selection_protocol.hash(state);
    }
}

pub(crate) struct RpcConnection {
    stream: TlsStream,
    next_sequence: u64,
}

struct ConnectionPool {
    idle: HashMap<PoolKey, Vec<RpcConnection>>,
    idle_count: usize,
    maximum_per_key: usize,
    maximum_total: usize,
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self {
            idle: HashMap::new(),
            idle_count: 0,
            maximum_per_key: DEFAULT_MAX_IDLE_PER_ENDPOINT,
            maximum_total: DEFAULT_MAX_IDLE_CONNECTIONS,
        }
    }
}

pub(crate) struct PooledConnection {
    connection: Option<RpcConnection>,
    key: PoolKey,
    pool: Arc<Mutex<ConnectionPool>>,
    reusable: bool,
    maximum_frame_length: usize,
    timeout: Duration,
    poll_interval: Duration,
    cancellation: CancellationToken,
}

impl PooledConnection {
    pub(crate) fn call(&mut self, request: &[u8], is_void: bool) -> Result<Vec<u8>> {
        let control = OperationControl {
            deadline: Instant::now()
                .checked_add(self.timeout)
                .ok_or(Error::Transport("operation deadline overflow"))?,
            cancellation: self.cancellation.clone(),
            poll_interval: self.poll_interval,
        };
        self.connection
            .as_mut()
            .ok_or(Error::Transport("pooled connection is absent"))?
            .stream
            .sock
            .set_control(control);
        let result = self.call_current(request, is_void);
        if result.is_err()
            && !matches!(
                &result,
                Err(Error::Rpc(foks_rpc::Error::RemoteStatus { .. }))
            )
        {
            // An unread or partial response must never satisfy a later call.
            self.invalidate();
        }
        result
    }

    pub(crate) fn invalidate(&mut self) {
        self.reusable = false;
        self.connection.take();
    }

    fn call_current(&mut self, request: &[u8], is_void: bool) -> Result<Vec<u8>> {
        self.reusable = false;
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Transport("pooled connection is absent"))?;
        let sequence = connection.next_sequence;
        let request = resequence_call(request, sequence, self.maximum_frame_length)?;
        // Team and Kex protocols exchange bare arguments and results with no
        // DataWrap envelope, so select the matching response decoder from the
        // protocol the request targets.
        let headerless = is_headerless_result_protocol(
            call_protocol_id(&request, self.maximum_frame_length).map_err(map_rpc_error)?,
        );
        connection
            .stream
            .write_all(&request)
            .map_err(map_io_error)?;
        connection.stream.flush().map_err(map_io_error)?;
        let response = match (is_void, headerless) {
            (true, false) => {
                read_void_response(&mut connection.stream, self.maximum_frame_length, sequence)
                    .map(|()| Vec::new())
            }
            (true, true) => {
                read_bare_void_response(&mut connection.stream, self.maximum_frame_length, sequence)
                    .map(|()| Vec::new())
            }
            (false, false) => {
                read_response(&mut connection.stream, self.maximum_frame_length, sequence)
            }
            (false, true) => {
                read_bare_response(&mut connection.stream, self.maximum_frame_length, sequence)
            }
        }
        .map_err(map_rpc_error);
        // A status error is a complete frame with the expected sequence, so
        // consume its sequence just like success. It does not desynchronize I/O.
        if let Err(error) = &response {
            if !matches!(error, Error::Rpc(foks_rpc::Error::RemoteStatus { .. })) {
                return response;
            }
        }
        connection.next_sequence = sequence
            .checked_add(1)
            .ok_or(Error::Transport("RPC sequence overflow"))?;
        self.reusable = true;
        response
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        if !self.reusable {
            return;
        }
        let Some(connection) = self.connection.take() else {
            return;
        };
        let Ok(mut pool) = self.pool.lock() else {
            return;
        };
        let key_count = pool.idle.get(&self.key).map_or(0, Vec::len);
        if key_count >= pool.maximum_per_key || pool.idle_count >= pool.maximum_total {
            return;
        }
        pool.idle
            .entry(self.key.clone())
            .or_default()
            .push(connection);
        pool.idle_count += 1;
    }
}

pub struct FoksClient {
    roots: rustls::RootCertStore,
    timeout: Duration,
    io_poll_interval: Duration,
    cancellation: CancellationToken,
    connection_pool: Arc<Mutex<ConnectionPool>>,
    pub(crate) maximum_frame_length: usize,
}

impl Clone for FoksClient {
    fn clone(&self) -> Self {
        Self {
            roots: self.roots.clone(),
            timeout: self.timeout,
            io_poll_interval: self.io_poll_interval,
            cancellation: self.cancellation.clone(),
            connection_pool: self.connection_pool.clone(),
            maximum_frame_length: self.maximum_frame_length,
        }
    }
}

impl Default for FoksClient {
    fn default() -> Self {
        Self::webpki()
    }
}

impl FoksClient {
    pub fn webpki() -> Self {
        Self {
            roots: rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
            timeout: Duration::from_secs(15),
            io_poll_interval: DEFAULT_IO_POLL_INTERVAL,
            cancellation: CancellationToken::new(),
            connection_pool: Arc::new(Mutex::new(ConnectionPool::default())),
            maximum_frame_length: DEFAULT_MAX_FRAME_LENGTH,
        }
    }

    pub fn with_roots(roots: rustls::RootCertStore) -> Self {
        Self {
            roots,
            timeout: Duration::from_secs(15),
            io_poll_interval: DEFAULT_IO_POLL_INTERVAL,
            cancellation: CancellationToken::new(),
            connection_pool: Arc::new(Mutex::new(ConnectionPool::default())),
            maximum_frame_length: DEFAULT_MAX_FRAME_LENGTH,
        }
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub(crate) fn isolated_with_timeout(&self, timeout: Duration) -> Self {
        Self {
            roots: self.roots.clone(),
            timeout,
            io_poll_interval: self.io_poll_interval,
            cancellation: self.cancellation.clone(),
            connection_pool: Arc::new(Mutex::new(ConnectionPool::default())),
            maximum_frame_length: self.maximum_frame_length,
        }
    }

    pub fn set_maximum_frame_length(&mut self, maximum: usize) -> Result<()> {
        if !(1..=MAX_CONFIGURABLE_FRAME_LENGTH).contains(&maximum) {
            return Err(Error::Transport("frame limit is outside supported bounds"));
        }
        self.maximum_frame_length = maximum;
        Ok(())
    }

    pub fn with_cancellation_token(&self, cancellation: CancellationToken) -> Self {
        let mut client = self.clone();
        client.cancellation = cancellation;
        client
    }

    pub fn set_connection_pool_limits(
        &self,
        maximum_per_endpoint: usize,
        maximum_total: usize,
    ) -> Result<()> {
        if maximum_per_endpoint > maximum_total || maximum_total > 1024 {
            return Err(Error::Transport("connection pool limits are inconsistent"));
        }
        let mut pool = self
            .connection_pool
            .lock()
            .map_err(|_| Error::Transport("connection pool lock is poisoned"))?;
        pool.maximum_per_key = maximum_per_endpoint;
        pool.maximum_total = maximum_total;
        for connections in pool.idle.values_mut() {
            connections.truncate(maximum_per_endpoint);
        }
        pool.idle.retain(|_, connections| !connections.is_empty());
        pool.idle_count = pool.idle.values().map(Vec::len).sum();
        while pool.idle_count > maximum_total {
            let Some(key) = pool.idle.keys().next().cloned() else {
                break;
            };
            if pool.idle.get_mut(&key).and_then(Vec::pop).is_some() {
                pool.idle_count -= 1;
            }
            if pool.idle.get(&key).is_some_and(Vec::is_empty) {
                pool.idle.remove(&key);
            }
        }
        Ok(())
    }

    /// Closes every currently idle pooled connection. Checked-out connections
    /// are subject to the configured limits when they are returned.
    pub fn clear_connection_pool(&self) -> Result<()> {
        let mut pool = self
            .connection_pool
            .lock()
            .map_err(|_| Error::Transport("connection pool lock is poisoned"))?;
        pool.idle.clear();
        pool.idle_count = 0;
        Ok(())
    }

    fn operation_control(&self) -> Result<OperationControl> {
        let deadline = Instant::now()
            .checked_add(self.timeout)
            .ok_or(Error::Transport("operation deadline overflow"))?;
        Ok(OperationControl {
            deadline,
            cancellation: self.cancellation.clone(),
            poll_interval: self.io_poll_interval,
        })
    }

    pub fn probe(&self, target: &ProbeTarget) -> Result<Vec<u8>> {
        self.probe_with_stream(target)
    }

    fn probe_with_stream(&self, target: &ProbeTarget) -> Result<Vec<u8>> {
        let control = self.operation_control()?;
        let tcp = self.connect_tcp(target, &control)?;
        let config = self.tls_config(None)?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        write_probe_request(&mut tls, &target.hostname, 0, None).map_err(map_rpc_error)?;
        read_probe_response(&mut tls, self.maximum_frame_length).map_err(map_rpc_error)
    }

    fn connect_tcp(
        &self,
        target: &ProbeTarget,
        control: &OperationControl,
    ) -> Result<ControlledTcpStream> {
        let remaining = control.remaining().map_err(map_io_error)?;
        let socket_addresses = if let Ok(ip) = target.hostname.parse::<std::net::IpAddr>() {
            vec![std::net::SocketAddr::new(ip, target.port)]
        } else {
            let (sender, receiver) = std::sync::mpsc::channel();
            let host = target.hostname.clone();
            let port = target.port;
            std::thread::Builder::new()
                .name("foks-dns-resolve".into())
                .spawn(move || {
                    let addrs = (host.as_str(), port)
                        .to_socket_addrs()
                        .map(|iter| iter.collect::<Vec<_>>());
                    let _ = sender.send(addrs);
                })
                .map_err(|_| Error::Transport("failed to spawn DNS resolver thread"))?;
            match receiver.recv_timeout(remaining) {
                Ok(result) => result.map_err(map_connect_error)?,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return Err(Error::DeadlineExceeded);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(Error::Transport("DNS resolver thread exited unexpectedly"));
                }
            }
        };
        control.remaining().map_err(map_io_error)?;
        if socket_addresses.is_empty() {
            return Err(Error::NoAddress(target.address()));
        }
        let mut last_error = None;
        let mut tcp = None;
        let address_count = socket_addresses.len();
        for (index, address) in socket_addresses.into_iter().enumerate() {
            let remaining_addresses = u32::try_from(address_count - index)
                .map_err(|_| Error::Transport("too many resolved addresses"))?;
            let address_budget = (control.remaining().map_err(map_io_error)? / remaining_addresses)
                .max(Duration::from_nanos(1));
            let address_deadline = Instant::now()
                .checked_add(address_budget)
                .ok_or(Error::Transport("address deadline overflow"))?;
            while let Some(address_remaining) =
                address_deadline.checked_duration_since(Instant::now())
            {
                if address_remaining.is_zero() {
                    break;
                }
                let timeout = control
                    .next_timeout()
                    .map_err(map_io_error)?
                    .min(address_remaining);
                match TcpStream::connect_timeout(&address, timeout) {
                    Ok(stream) => {
                        tcp = Some(stream);
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                        last_error = Some(error);
                        control.remaining().map_err(map_io_error)?;
                    }
                    Err(error) => {
                        last_error = Some(error);
                        break;
                    }
                }
            }
            if tcp.is_some() {
                break;
            }
        }
        if tcp.is_none() {
            control.remaining().map_err(map_io_error)?;
        }
        let tcp = tcp.ok_or_else(|| {
            Error::Connect(last_error.unwrap_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "connection attempt timed out before socket opened",
                )
            }))
        })?;
        Ok(ControlledTcpStream {
            inner: tcp,
            control: control.clone(),
        })
    }

    fn tls_config(&self, credential: Option<&DeviceCredential>) -> Result<rustls::ClientConfig> {
        self.tls_config_material(
            &self.roots,
            credential
                .map(|credential| (&credential.seed, credential.certificate_chain.as_slice())),
        )
    }

    pub(crate) fn tls_config_material(
        &self,
        roots: &rustls::RootCertStore,
        credential: Option<(&SecretSeed, &[Vec<u8>])>,
    ) -> Result<rustls::ClientConfig> {
        let credential = credential
            .map(|(seed, certificates)| -> Result<_> {
                Ok(TlsClientCredential {
                    signing_key_pkcs8: device_signing_key_pkcs8(seed)?,
                    certificate_chain: certificates,
                })
            })
            .transpose()?;
        self.tls_config_pkcs8_material(roots, credential)
    }

    fn tls_config_pkcs8_material(
        &self,
        roots: &rustls::RootCertStore,
        credential: Option<TlsClientCredential<'_>>,
    ) -> Result<rustls::ClientConfig> {
        // Both ring and aws-lc can enter the workspace graph. Name the
        // provider so rustls never has to guess which process default to use.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots.clone());
        match credential {
            None => Ok(builder.with_no_client_auth()),
            Some(TlsClientCredential {
                signing_key_pkcs8,
                certificate_chain,
            }) => {
                if certificate_chain.is_empty() {
                    return Err(Error::CertificateChain);
                }
                let certificates = certificate_chain
                    .iter()
                    .cloned()
                    .map(CertificateDer::from)
                    .collect();
                let key =
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key_pkcs8.as_slice()));
                let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key)?;
                let certified_key = rustls::sign::CertifiedKey::new(certificates, signing_key);
                match certified_key.keys_match() {
                    Ok(())
                    | Err(rustls::Error::InconsistentKeys(rustls::InconsistentKeys::Unknown)) => {}
                    Err(error) => return Err(error.into()),
                }
                Ok(builder.with_client_cert_resolver(Arc::new(
                    rustls::sign::SingleCertAndKey::from(certified_key),
                )))
            }
        }
    }

    pub(crate) fn connect_tls(
        &self,
        target: &ProbeTarget,
        tcp: ControlledTcpStream,
        config: rustls::ClientConfig,
    ) -> Result<TlsStream> {
        let server_name =
            ServerName::try_from(target.hostname.clone()).map_err(|_| Error::ServerName)?;
        let connection = rustls::ClientConnection::new(Arc::new(config), server_name)?;
        Ok(rustls::StreamOwned::new(connection, tcp))
    }

    pub(crate) fn call(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        credential: Option<&DeviceCredential>,
    ) -> Result<Vec<u8>> {
        let credential = credential
            .map(|credential| self.credential(&credential.seed, &credential.certificate_chain))
            .transpose()?;
        self.call_prepared(host, target, request, credential, None, false)
    }

    /// Executes a public RPC against a WebPKI-authenticated endpoint that has
    /// not yet been pinned. This is intentionally not pooled and is suitable
    /// only for discovery data that will be independently authenticated.
    pub(crate) fn call_unpinned(&self, target: &ProbeTarget, request: &[u8]) -> Result<Vec<u8>> {
        let control = self.operation_control()?;
        let tcp = self.connect_tcp(target, &control)?;
        let config = self.tls_config(None)?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        tls.write_all(request).map_err(map_io_error)?;
        tls.flush().map_err(map_io_error)?;
        read_response(&mut tls, self.maximum_frame_length, 0).map_err(map_rpc_error)
    }

    pub(crate) fn call_with_material(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<Vec<u8>> {
        let credential = Some(self.credential(seed, certificate_chain)?);
        self.call_prepared(host, target, request, credential, None, false)
    }

    pub(crate) fn call_with_pkcs8_material(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        key: Zeroizing<Vec<u8>>,
        certificate_chain: &[Vec<u8>],
    ) -> Result<Vec<u8>> {
        self.call_prepared(
            host,
            target,
            request,
            Some(TlsClientCredential {
                signing_key_pkcs8: key,
                certificate_chain,
            }),
            None,
            false,
        )
    }

    pub(crate) fn call_void(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        credential: &DeviceCredential,
    ) -> Result<()> {
        self.call_void_with_material(
            host,
            target,
            request,
            &credential.seed,
            &credential.certificate_chain,
        )
    }

    pub(crate) fn call_void_with_material(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<()> {
        let credential = Some(self.credential(seed, certificate_chain)?);
        self.call_prepared(host, target, request, credential, None, true)
            .map(|_| ())
    }

    pub(crate) fn call_void_with_pkcs8_material(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        key: Zeroizing<Vec<u8>>,
        certificate_chain: &[Vec<u8>],
    ) -> Result<()> {
        self.call_prepared(
            host,
            target,
            request,
            Some(TlsClientCredential {
                signing_key_pkcs8: key,
                certificate_chain,
            }),
            None,
            true,
        )
        .map(|_| ())
    }

    pub(crate) fn call_after_vhost_selection(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        select_request: &[u8],
        request: &[u8],
    ) -> Result<Vec<u8>> {
        self.call_prepared(host, target, request, None, Some(select_request), false)
    }

    pub(crate) fn call_void_after_vhost_selection(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        select_request: &[u8],
        request: &[u8],
    ) -> Result<()> {
        self.call_prepared(host, target, request, None, Some(select_request), true)
            .map(|_| ())
    }

    pub(crate) fn pooled_connection_with_material(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        select_request: &[u8],
    ) -> Result<PooledConnection> {
        let credential = Some(self.credential(seed, certificate_chain)?);
        self.checkout_connection(host, target, credential, Some(select_request))
    }

    fn credential<'a>(
        &self,
        seed: &SecretSeed,
        certificate_chain: &'a [Vec<u8>],
    ) -> Result<TlsClientCredential<'a>> {
        Ok(TlsClientCredential {
            signing_key_pkcs8: device_signing_key_pkcs8(seed)?,
            certificate_chain,
        })
    }

    fn call_prepared(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        credential: Option<TlsClientCredential<'_>>,
        select_request: Option<&[u8]>,
        is_void: bool,
    ) -> Result<Vec<u8>> {
        let mut connection = self.checkout_connection(host, target, credential, select_request)?;
        connection.call_current(request, is_void)
    }

    fn checkout_connection(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        credential: Option<TlsClientCredential<'_>>,
        select_request: Option<&[u8]>,
    ) -> Result<PooledConnection> {
        let control = self.operation_control()?;
        control.remaining().map_err(map_io_error)?;
        let authentication = authentication_fingerprint(credential.as_ref());
        let trust = trust_fingerprint(host);
        let key = PoolKey {
            hostname: target.hostname.clone(),
            port: target.port,
            host_id: host.host_id.as_bytes().to_vec(),
            authentication,
            trust,
            selection_protocol: select_request
                .map(|request| {
                    call_protocol_id(request, self.maximum_frame_length).map_err(map_rpc_error)
                })
                .transpose()?,
        };
        let roots = authenticated_tls_roots(host)?;
        // Build and validate the credential even on a pool hit. This prevents
        // a caller from borrowing an authenticated connection with a key that
        // does not match the supplied certificate chain.
        let config = self.tls_config_pkcs8_material(&roots, credential)?;
        let pooled = {
            let mut pool = self
                .connection_pool
                .lock()
                .map_err(|_| Error::Transport("connection pool lock is poisoned"))?;
            // Discard every idle connection the peer has already closed
            // rather than handing one out to fail on its first write.
            let live = loop {
                let Some(connection) = pool.idle.get_mut(&key).and_then(Vec::pop) else {
                    break None;
                };
                pool.idle_count -= 1;
                if connection.stream.sock.is_reusable() {
                    break Some(connection);
                }
            };
            if pool.idle.get(&key).is_some_and(Vec::is_empty) {
                pool.idle.remove(&key);
            }
            live
        };
        let connection = match pooled {
            Some(mut connection) => {
                connection.stream.sock.set_control(control);
                connection
            }
            None => {
                let tcp = self.connect_tcp(target, &control)?;
                RpcConnection {
                    stream: self.connect_tls(target, tcp, config)?,
                    next_sequence: 0,
                }
            }
        };
        let mut pooled = PooledConnection {
            connection: Some(connection),
            key,
            pool: self.connection_pool.clone(),
            reusable: true,
            maximum_frame_length: self.maximum_frame_length,
            timeout: self.timeout,
            poll_interval: self.io_poll_interval,
            cancellation: self.cancellation.clone(),
        };
        if let Some(select_request) = select_request {
            if pooled
                .connection
                .as_ref()
                .is_some_and(|connection| connection.next_sequence == 0)
            {
                pooled.call_current(select_request, true)?;
            }
        }
        Ok(pooled)
    }
}

fn authentication_fingerprint(credential: Option<&TlsClientCredential<'_>>) -> [u8; 32] {
    let mut input = Vec::new();
    if let Some(credential) = credential {
        // Partition on a one-way digest because some supported signing
        // providers cannot prove key/certificate equality before the TLS
        // handshake. Raw private key bytes never enter an ordinary buffer or
        // the long-lived pool key.
        input.extend_from_slice(&prefixed_hash(
            PRIVATE_KEY_POOL_KEY_TYPE_ID,
            &credential.signing_key_pkcs8,
        ));
        for certificate in credential.certificate_chain {
            input.extend_from_slice(&(certificate.len() as u64).to_be_bytes());
            input.extend_from_slice(certificate);
        }
    }
    prefixed_hash(AUTH_POOL_KEY_TYPE_ID, &input)
}

fn trust_fingerprint(host: &PinnedHost) -> [u8; 32] {
    let mut input = Vec::new();
    input.extend_from_slice(host.host_id.as_bytes());
    for certificate in &host.tls_ca_certificates {
        input.extend_from_slice(&(certificate.len() as u64).to_be_bytes());
        input.extend_from_slice(certificate);
    }
    prefixed_hash(TRUST_POOL_KEY_TYPE_ID, &input)
}

fn map_connect_error(error: std::io::Error) -> Error {
    if is_cancelled_io(&error) {
        return Error::Cancelled;
    }
    match error.kind() {
        std::io::ErrorKind::TimedOut => Error::DeadlineExceeded,
        _ => Error::Connect(error),
    }
}

fn map_io_error(error: std::io::Error) -> Error {
    if is_cancelled_io(&error) {
        return Error::Cancelled;
    }
    match error.kind() {
        std::io::ErrorKind::TimedOut => Error::DeadlineExceeded,
        _ => Error::Rpc(foks_rpc::Error::Io(error)),
    }
}

fn is_cancelled_io(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|source| source.downcast_ref::<CancelledIo>().is_some())
}

fn map_rpc_error(error: foks_rpc::Error) -> Error {
    match error {
        foks_rpc::Error::Io(error) => map_io_error(error),
        error => Error::Rpc(error),
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn cancellation_and_deadline_have_distinct_errors() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let cancelled = OperationControl {
            deadline: Instant::now() + Duration::from_secs(1),
            cancellation,
            poll_interval: DEFAULT_IO_POLL_INTERVAL,
        };
        assert_eq!(
            cancelled.remaining().unwrap_err().kind(),
            std::io::ErrorKind::Other
        );

        let expired = OperationControl {
            deadline: Instant::now() - Duration::from_millis(1),
            cancellation: CancellationToken::new(),
            poll_interval: DEFAULT_IO_POLL_INTERVAL,
        };
        assert_eq!(
            expired.remaining().unwrap_err().kind(),
            std::io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn frame_and_pool_configuration_is_bounded() {
        let mut client = FoksClient::with_roots(rustls::RootCertStore::empty());
        assert!(client.set_maximum_frame_length(0).is_err());
        assert!(client
            .set_maximum_frame_length(MAX_CONFIGURABLE_FRAME_LENGTH + 1)
            .is_err());
        client.set_maximum_frame_length(1024).unwrap();
        assert!(client.set_connection_pool_limits(2, 1).is_err());
        client.set_connection_pool_limits(0, 0).unwrap();
        client.clear_connection_pool().unwrap();
    }

    #[test]
    fn isolated_timeout_uses_a_dedicated_connection_pool() {
        let client = FoksClient::with_roots(rustls::RootCertStore::empty());
        let isolated = client.isolated_with_timeout(Duration::from_secs(60));
        assert!(!Arc::ptr_eq(
            &client.connection_pool,
            &isolated.connection_pool
        ));
        assert_eq!(isolated.timeout, Duration::from_secs(60));
    }

    #[test]
    fn pool_credentials_are_partitioned_by_private_key_and_chain() {
        let first_key = Zeroizing::new(vec![1; 32]);
        let second_key = Zeroizing::new(vec![2; 32]);
        let first_chain = vec![vec![3; 64]];
        let second_chain = vec![vec![4; 64]];
        let first = authentication_fingerprint(Some(&TlsClientCredential {
            signing_key_pkcs8: first_key,
            certificate_chain: &first_chain,
        }));
        let changed_key = authentication_fingerprint(Some(&TlsClientCredential {
            signing_key_pkcs8: second_key,
            certificate_chain: &first_chain,
        }));
        let changed_chain = authentication_fingerprint(Some(&TlsClientCredential {
            signing_key_pkcs8: Zeroizing::new(vec![1; 32]),
            certificate_chain: &second_chain,
        }));
        assert_ne!(first, changed_key);
        assert_ne!(first, changed_chain);
        assert_ne!(first, authentication_fingerprint(None));
    }
    #[test]
    fn pool_scope_includes_selection_protocol_and_security_material() {
        let base = PoolKey {
            hostname: "localhost".into(),
            port: 443,
            host_id: vec![2; 33],
            authentication: [3; 32],
            trust: [4; 32],
            selection_protocol: Some(foks_rpc::REAL_TIME_PROTOCOL_ID),
        };
        let mut keys = std::collections::HashSet::from([base.clone()]);
        let mut key = base.clone();
        key.selection_protocol = None;
        assert!(keys.insert(key));
        let mut key = base.clone();
        key.selection_protocol = Some(1);
        assert!(keys.insert(key));
        let mut key = base.clone();
        key.host_id[1] ^= 1;
        assert!(keys.insert(key));
        let mut key = base.clone();
        key.authentication[0] ^= 1;
        assert!(keys.insert(key));
        let mut key = base;
        key.trust[0] ^= 1;
        assert!(keys.insert(key));
    }
}
