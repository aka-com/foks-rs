use std::sync::Arc;

mod handlers;
mod kex;

use foks_proto::{EntityId, MerkleRoot};
use foks_rpc::{encode_status_response_at, RpcStatus, STATUS_OK, STATUS_RATE_LIMIT_ERROR};
use foks_snowpack::{decode, Value};
use rustls::pki_types::CertificateDer;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::auth::Principal;
use crate::keys::HostKeyProvider;
use crate::rpc::{route_call, Listener, RouteError, RouteId, RouteSpec};
use crate::services::merkle_query::{validate_host_argument_against, MerkleQueries};
use crate::services::IdentityPorts;
use crate::{Entropy, Result, SessionLimits, WriterHandle};

#[derive(Clone)]
pub(crate) struct ServerData {
    vhost_management_host: String,
    web_admin: Option<Arc<crate::web_admin::WebAdminService>>,
    sso: Option<Arc<crate::sso::SsoService>>,
    peer_ip: Option<std::net::IpAddr>,
    probe_response: Arc<[u8]>,
    host_id: Vec<u8>,
    canonical_name: String,
    /// The advertised `host:port` a peer should use to reach this host, taken
    /// from the probe service in the signed public zone.
    probe_endpoint: String,
    current_root: Vec<u8>,
    read_database: Option<crate::read_pool::ReadPool>,
    writer: Option<WriterHandle>,
    clock: Arc<dyn foks_server_db::Clock>,
    entropy: Arc<dyn Entropy>,
    key_provider: Option<Arc<dyn HostKeyProvider>>,
    hostchain_tail: foks_proto::HostchainTail,
    session_faults: Option<Arc<crate::SessionFaults>>,
    metrics: Arc<crate::ServerMetrics>,
    rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    execution: Arc<Semaphore>,
    /// Realtime inbox poll database reads in progress. The number of polls is
    /// bounded by the authenticated listener's connection limit, since the
    /// route is served only there and each poll holds its connection; a hint
    /// can wake all of them at once. Poll reads wait for one of these permits
    /// before leasing a reader, so they hold at most half of the reader pool
    /// (or its only reader) and leave the rest to ordinary requests.
    realtime_poll_reads: Arc<Semaphore>,
    /// Realtime inbox polls in progress per account UID.
    account_realtime_polls: Arc<crate::rate_limit::KeyedLimit<Vec<u8>>>,
    request_memory: Arc<Semaphore>,
    kex_relay: Arc<kex::Relay>,
    realtime: crate::services::realtime::RealtimeService,
    waitlist_enabled: bool,
}

impl ServerData {
    pub(crate) fn from_config(
        config: &crate::Config,
        rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    ) -> Result<Self> {
        crate::config::validate_vhost_management_host(&config.vhost_management_host)?;
        let probe_response = Arc::clone(&config.probe_response);
        let probe = foks_proto::ProbeResponse::decode(&probe_response)?;
        let first = probe
            .hostchain
            .first()
            .ok_or(crate::Error::Config("empty bootstrap hostchain"))?;
        let host_id = first.decode_change()?.host.into_bytes();
        if config
            .sso
            .as_ref()
            .is_some_and(|s| s.configured_host().as_slice() != host_id)
        {
            return Err(crate::Error::Config(
                "OIDC service belongs to a different host",
            ));
        }
        let zone = foks_proto::PublicZone::decode(&probe.public_zone.inner)?;
        let root = MerkleRoot::decode(&probe.merkle_root.inner)?;
        let probe_endpoint = zone.services.probe.clone();
        let canonical_name = endpoint_host(&probe_endpoint)
            .ok_or(crate::Error::Config("invalid bootstrap probe endpoint"))?
            .to_owned();
        Ok(Self {
            sso: config.sso.clone(),
            web_admin: config.web_admin.clone(),
            vhost_management_host: config.vhost_management_host.clone(),
            peer_ip: None,
            probe_response,
            host_id,
            canonical_name,
            probe_endpoint,
            current_root: probe.merkle_root.encoded()?,
            read_database: config
                .read_database
                .clone()
                .map(|database| {
                    crate::read_pool::ReadPool::new(
                        database,
                        config.limits.maximum_read_connections,
                    )
                })
                .transpose()?,
            writer: config.writer.clone(),
            clock: Arc::clone(&config.clock),
            entropy: Arc::clone(&config.entropy),
            key_provider: config.key_provider.clone(),
            hostchain_tail: root.hostchain,
            session_faults: config.session_faults.clone(),
            metrics: Arc::clone(&config.metrics),
            rate_limiter,
            execution: Arc::new(Semaphore::new(config.limits.maximum_in_flight_requests)),
            realtime_poll_reads: Arc::new(Semaphore::new(realtime_poll_read_permits(
                config.limits.maximum_read_connections,
            ))),
            account_realtime_polls: Arc::new(crate::rate_limit::KeyedLimit::new(
                config.limits.maximum_realtime_polls_per_account,
            )),
            request_memory: Arc::new(Semaphore::new(config.limits.maximum_request_memory_bytes)),
            kex_relay: Arc::new(kex::Relay::default()),
            realtime: crate::services::realtime::RealtimeService::new(Arc::clone(&config.metrics)),
            waitlist_enabled: config.waitlist_enabled,
        })
    }

    fn read_database(&self) -> std::result::Result<crate::read_pool::ReadLease, RpcStatus> {
        let pool = self.read_database.as_ref().ok_or(RpcStatus::Unsupported)?;
        pool.lease(&self.metrics)
    }

    fn host(&self) -> std::result::Result<EntityId, RpcStatus> {
        EntityId::from_bytes(self.host_id.clone()).map_err(bad_arguments)
    }

    /// The state that signup, username reservation, certificate issuance and
    /// user-chain mutations read. It carries this request's writer, which an
    /// authenticated request has already scoped to its principal.
    fn identity_ports(&self) -> IdentityPorts<'_> {
        IdentityPorts {
            host_id: &self.host_id,
            canonical_name: &self.canonical_name,
            read_database: self.read_database.as_ref(),
            metrics: &self.metrics,
            writer: self.writer.as_ref(),
            key_provider: self.key_provider.as_ref(),
            sso: self.sso.as_ref(),
            clock: &self.clock,
            entropy: &self.entropy,
            hostchain_tail: &self.hostchain_tail,
        }
    }

    fn merkle_queries(&self) -> MerkleQueries<'_> {
        MerkleQueries {
            host_id: &self.host_id,
            read_database: self.read_database.as_ref(),
            metrics: &self.metrics,
            current_root: &self.current_root,
            probe_response: &self.probe_response,
        }
    }

    fn validate_host_argument(&self, argument: &[u8]) -> std::result::Result<(), RpcStatus> {
        validate_host_argument_against(argument, &self.host_id, false)
    }

    fn validate_optional_host_argument(
        &self,
        argument: &[u8],
    ) -> std::result::Result<(), RpcStatus> {
        validate_host_argument_against(argument, &self.host_id, true)
    }

    /// Answers a public Beacon lookup for this host's advertised endpoint. The
    /// request names an exact HostID; any other host is a typed not-found. The
    /// hint is untrusted routing data: a peer must still probe the returned
    /// endpoint and verify its hostchain against the requested HostID.
    fn beacon_lookup(&self, argument: &[u8]) -> std::result::Result<Vec<u8>, RpcStatus> {
        validate_host_argument_against(argument, &self.host_id, false)?;
        foks_snowpack::encode(&Value::Text(self.probe_endpoint.as_bytes().to_vec()))
            .map_err(|_| RpcStatus::TransactionRetry)
    }

    fn validate_probe(&self, argument: &[u8]) -> std::result::Result<(), RpcStatus> {
        let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
            return Err(bad_arguments("probe argument is not a struct"));
        };
        let [Value::Text(hostname), Value::Unsigned(_), host_id] = fields.as_slice() else {
            return Err(bad_arguments("probe argument has the wrong shape"));
        };
        if hostname.as_slice() != self.canonical_name.as_bytes() {
            return Err(RpcStatus::NotFound("host not found".to_owned()));
        }
        match host_id {
            Value::Null => Ok(()),
            Value::Binary(host_id)
                if host_id.len() == 33
                    && host_id.first() == Some(&foks_proto::ENTITY_HOST)
                    && host_id == &self.host_id =>
            {
                Ok(())
            }
            Value::Binary(host_id)
                if host_id.len() == 33 && host_id.first() == Some(&foks_proto::ENTITY_HOST) =>
            {
                Err(RpcStatus::NotFound("host not found".to_owned()))
            }
            Value::Binary(_) => Err(bad_arguments("probe HostID is malformed")),
            _ => Err(bad_arguments("probe HostID has the wrong shape")),
        }
    }
}

pub(crate) async fn serve(
    stream: tokio::net::TcpStream,
    peer_ip: std::net::IpAddr,
    listener: Listener,
    tls: &Arc<rustls::ServerConfig>,
    service_data: &Arc<ServerData>,
    limits: SessionLimits,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    // Small request/response TLS records must not wait for delayed TCP ACKs.
    stream.set_nodelay(true)?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::clone(tls));
    let handshake = tokio::select! {
        result = stop.changed() => {
            let _ = result;
            return Ok(());
        }
        result = tokio::time::timeout(limits.io_timeout, acceptor.accept(stream)) => result,
    };
    let mut stream =
        handshake.map_err(|_| crate::Error::Io(timeout_error("TLS handshake timed out")))??;
    let peer_certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .map(|certificate| certificate.as_ref().to_vec());

    for _ in 0..limits.maximum_requests {
        let read = tokio::select! {
            result = stop.changed() => {
                let _ = result;
                break;
            }
            result = tokio::time::timeout(
                limits.io_timeout,
                read_frame_length_async(&mut stream, limits.maximum_frame_bytes),
            ) => result,
        };
        let length = match read {
            Err(_) => break,
            Ok(Ok(length)) => length,
            Ok(Err(foks_rpc::Error::Io(error)))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Ok(Err(error)) => return Err(error.into()),
        };
        if !service_data.rate_limiter.allow_request(peer_ip) {
            service_data.metrics.request_started();
            let _request_timer =
                crate::ServerMetrics::request_timer(Arc::clone(&service_data.metrics));
            service_data
                .metrics
                .request_rate_limited(crate::AdmissionRejection::RequestRate);
            let response = encode_status_response_at(&RpcStatus::RateLimited, 0)?;
            write_response(&mut stream, &response, limits.io_timeout).await?;
            service_data
                .metrics
                .response_completed(crate::metrics::UNROUTED, STATUS_RATE_LIMIT_ERROR);
            return Ok(());
        }
        let read = tokio::select! {
            result = stop.changed() => {
                let _ = result;
                break;
            }
            result = tokio::time::timeout(
                limits.io_timeout,
                read_call_body_async(
                    &mut stream,
                    length,
                    limits.maximum_frame_bytes,
                    Arc::clone(&service_data.request_memory),
                ),
            ) => result,
        };
        let call = match read {
            Err(_) => break,
            Ok(Ok(call)) => call,
            Ok(Err(foks_rpc::Error::Io(error)))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Ok(Err(error)) => return Err(error.into()),
        };
        let (message, request_memory) = call;
        let call = match message {
            foks_rpc::InboundMessage::Call(call) => call,
            // NOTIFY and CANCEL control messages are never answered by
            // go-snowpack-rpc. Discard the frame and keep the connection open
            // instead of treating the unexpected method type as fatal.
            foks_rpc::InboundMessage::Control => {
                drop(request_memory);
                continue;
            }
        };
        service_data.metrics.request_started();
        let _request_timer = crate::ServerMetrics::request_timer(Arc::clone(&service_data.metrics));
        let sequence = call.sequence();
        let data = Arc::clone(service_data);
        let certificate = peer_certificate.clone();
        let routed = route_call(call, listener);
        let family = routed
            .as_ref()
            .map_or(crate::metrics::UNROUTED, |call| call.route.protocol);
        // The end of a bounded handler's execution budget. It is taken just
        // before the execution timeout below starts, so it does not follow the
        // instant at which the session stops waiting and closes the
        // connection. The request's writer skips a queued write that it has
        // not started by then.
        let deadline = std::time::Instant::now().checked_add(limits.request_timeout);
        let outcome = match routed {
            Ok(call) if call.route.id == RouteId::RegPollOAuth2SessionCompletion => {
                let poll = service_data.long_poll(call.route, sequence, request_memory);
                let handler = handlers::sso_poll_response(service_data.as_ref(), call);
                poll.run(stream.get_ref().0.readable(), &mut stop, handler)
                    .await?
            }
            Ok(call) if call.route.id == RouteId::KexReceive => {
                let poll = service_data.long_poll(call.route, sequence, request_memory);
                let handler = handlers::kex_receive_response(service_data.as_ref(), call);
                poll.run(stream.get_ref().0.readable(), &mut stop, handler)
                    .await?
            }
            Ok(call) if call.route.id == RouteId::RealTimeRtPollInbox => {
                let poll = service_data.long_poll(call.route, sequence, request_memory);
                let handler =
                    handlers::realtime_poll_response(service_data.as_ref(), call, certificate);
                poll.run(stream.get_ref().0.readable(), &mut stop, handler)
                    .await?
            }
            routed => match execute_bounded(
                Arc::clone(&service_data.execution),
                limits.request_timeout,
                &mut stop,
                move || -> Result<RequestOutcome> {
                    // A timed-out blocking handler can still own decoded request
                    // memory, so keep its reservation in the same closure.
                    let _request_memory = request_memory;
                    let _handler_timer =
                        crate::ServerMetrics::handler_timer(Arc::clone(&data.metrics));
                    let principal = if listener == Listener::Authenticated {
                        let certificate =
                            CertificateDer::from(certificate.ok_or(crate::Error::Config(
                                "authenticated TLS session has no client certificate",
                            ))?);
                        let now = data.clock.now_micros()?;
                        let database = match data.read_database() {
                            Ok(database) => database,
                            Err(status) => {
                                let (response, status) = encode_result(Err(status), sequence)?;
                                return Ok(RequestOutcome {
                                    response,
                                    status,
                                    route: None,
                                    disconnect_before_response: false,
                                });
                            }
                        };
                        Some(Principal::authenticate(&certificate, &database, now)?)
                    } else {
                        None
                    };
                    let mut route = None;
                    let result = match routed {
                        Ok(call) => {
                            let protocol = call.route.protocol;
                            let method = call.route.method;
                            route = Some(call.route);
                            if data.should_disconnect(
                                crate::SessionFaultPoint::BeforeDurableMutation,
                                protocol,
                                method,
                            ) {
                                return Ok(RequestOutcome {
                                    response: Vec::new(),
                                    status: STATUS_OK,
                                    route,
                                    disconnect_before_response: true,
                                });
                            }
                            let mut request_data = data.as_ref().clone();
                            request_data.peer_ip = Some(peer_ip);
                            if let Some(deadline) = deadline {
                                request_data.writer = request_data
                                    .writer
                                    .as_ref()
                                    .map(|w| w.with_deadline(deadline));
                            }
                            let authorization = if !call.route.supported {
                                Err(RpcStatus::Unsupported)
                            } else if let Some(principal) = principal.as_ref() {
                                request_data.authorize_principal(principal).map(|()| {
                                    request_data.writer = request_data.writer.as_ref().map(|w| {
                                        w.for_authenticated_request(
                                            principal.uid(),
                                            principal.device_id(),
                                            request_data.clock.clone(),
                                        )
                                    });
                                })
                            } else {
                                Ok(())
                            };
                            authorization.and_then(|()| {
                                handlers::response(&request_data, call, principal.as_ref())
                            })
                        }
                        Err(RouteError::RequestTooLarge { .. }) => Err(RpcStatus::BadArguments(
                            "request exceeds the method limit".to_owned(),
                        )),
                        Err(RouteError::Unknown { .. } | RouteError::WrongListener) => {
                            Err(RpcStatus::Unsupported)
                        }
                    };
                    let (response, status) = encode_result(result, sequence)?;
                    Ok(RequestOutcome {
                        response,
                        status,
                        route,
                        disconnect_before_response: false,
                    })
                },
            )
            .await?
            {
                BoundedExecution::Completed(outcome) => Some(outcome?),
                BoundedExecution::Saturated => {
                    service_data
                        .metrics
                        .request_rate_limited(crate::AdmissionRejection::RequestHandlers);
                    let response = encode_status_response_at(&RpcStatus::RateLimited, sequence)?;
                    write_response(&mut stream, &response, limits.io_timeout).await?;
                    service_data
                        .metrics
                        .response_completed(family, STATUS_RATE_LIMIT_ERROR);
                    return Ok(());
                }
                BoundedExecution::TimedOut => {
                    // A durable mutation may still be reconciling in the bounded
                    // blocking pool. Close without a protocol response so clients
                    // preserve the operation's ambiguous outcome semantics.
                    return Err(crate::Error::Io(timeout_error(
                        "request execution timed out",
                    )));
                }
                BoundedExecution::Stopped => break,
            },
        };
        // A long poll ends without an outcome when the peer becomes readable
        // or the server stops; either way the connection closes unanswered.
        let Some(outcome) = outcome else {
            break;
        };
        if outcome.disconnect_before_response {
            return Ok(());
        }
        let response = outcome.response;
        let status = outcome.status;
        let route = outcome.route;
        if let Some(route) = route {
            let protocol = route.protocol;
            let method = route.method;
            let point = if route.id == crate::rpc::RouteId::KvStoreFileUploadChunk {
                crate::SessionFaultPoint::BetweenLargeFileChunks
            } else {
                crate::SessionFaultPoint::AfterDurableCommitBeforeResponse
            };
            if service_data.should_disconnect(point, protocol, method) {
                return Ok(());
            }
            if service_data.should_disconnect(
                crate::SessionFaultPoint::DuringResponseWrite,
                protocol,
                method,
            ) {
                let split = response.len().div_ceil(2);
                write_response(&mut stream, &response[..split], limits.io_timeout).await?;
                return Ok(());
            }
        }
        let write = tokio::select! {
            result = stop.changed() => {
                let _ = result;
                break;
            }
            result = write_response(&mut stream, &response, limits.io_timeout) => result,
        };
        write?;
        service_data.metrics.response_completed(family, status);
    }
    Ok(())
}

struct RequestOutcome {
    response: Vec<u8>,
    /// The wire status code that `response` carries, `STATUS_OK` for a success.
    status: u64,
    route: Option<&'static crate::rpc::RouteSpec>,
    disconnect_before_response: bool,
}

/// Encodes a handler result and returns the response frame with the wire
/// status code it carries.
fn encode_result(
    result: std::result::Result<Vec<u8>, RpcStatus>,
    sequence: u64,
) -> Result<(Vec<u8>, u64)> {
    match result {
        Ok(response) => Ok((response, STATUS_OK)),
        Err(status) => Ok((encode_status_response_at(&status, sequence)?, status.code())),
    }
}

enum BoundedExecution<T> {
    Completed(T),
    Saturated,
    TimedOut,
    Stopped,
}

async fn execute_bounded<T>(
    execution: Arc<Semaphore>,
    timeout: std::time::Duration,
    stop: &mut watch::Receiver<bool>,
    operation: impl FnOnce() -> T + Send + 'static,
) -> Result<BoundedExecution<T>>
where
    T: Send + 'static,
{
    let Ok(permit) = execution.try_acquire_owned() else {
        return Ok(BoundedExecution::Saturated);
    };
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation()
    });
    tokio::select! {
        result = stop.changed() => {
            let _ = result;
            Ok(BoundedExecution::Stopped)
        }
        result = tokio::time::timeout(timeout, &mut task) => {
            match result {
                Ok(Ok(value)) => Ok(BoundedExecution::Completed(value)),
                Ok(Err(_)) => Err(crate::Error::Thread),
                Err(_) => Ok(BoundedExecution::TimedOut),
            }
        }
    }
}

/// One long-poll request: the SSO completion poll, a KEX receive or a
/// realtime inbox poll. A Go-compatible long poll can remain open for a long
/// time, so it is awaited on the connection task rather than a blocking worker;
/// the listener's active-connection semaphore bounds how many run at once.
struct LongPoll<'a> {
    metrics: &'a Arc<crate::ServerMetrics>,
    faults: Option<&'a crate::SessionFaults>,
    route: &'static RouteSpec,
    sequence: u64,
    /// The decoded request stays reserved until the poll answers.
    request_memory: OwnedSemaphorePermit,
}

impl LongPoll<'_> {
    /// Applies the request fault check and handler timer that bounded
    /// requests get, then awaits `handler`. Returns `None` when the
    /// connection should close without a response: `peer_readable` completed
    /// or the server is stopping.
    async fn run<P, H>(
        self,
        peer_readable: P,
        stop: &mut watch::Receiver<bool>,
        handler: H,
    ) -> Result<Option<RequestOutcome>>
    where
        P: std::future::Future,
        H: std::future::Future<Output = std::result::Result<Vec<u8>, RpcStatus>>,
    {
        let _request_memory = self.request_memory;
        let _handler_timer = crate::ServerMetrics::handler_timer(Arc::clone(self.metrics));
        let route = self.route;
        if self.faults.is_some_and(|faults| {
            faults.disconnect(
                crate::SessionFaultPoint::BeforeDurableMutation,
                route.protocol,
                route.method,
            )
        }) {
            return Ok(Some(RequestOutcome {
                response: Vec::new(),
                status: STATUS_OK,
                route: Some(route),
                disconnect_before_response: true,
            }));
        }
        // A finished handler wins over peer readability that became ready at
        // the same time. A concurrent stop is not consumed here, so the write
        // in `serve` can still close the connection without the response.
        let result = tokio::select! {
            biased;
            result = handler => result,
            // Snowpack is request/response serial. Readability while this
            // response is pending is therefore either a client
            // cancellation/control frame or socket shutdown. Closing promptly
            // drops the handler future, which releases relay waiters and poll
            // permits instead of retaining an abandoned Go long poll.
            _ = peer_readable => return Ok(None),
            result = stop.changed() => {
                let _ = result;
                return Ok(None);
            }
        };
        let (response, status) = encode_result(result, self.sequence)?;
        Ok(Some(RequestOutcome {
            response,
            status,
            route: Some(route),
            disconnect_before_response: false,
        }))
    }
}

async fn read_frame_length_async<R: AsyncRead + Unpin>(
    reader: &mut R,
    maximum: usize,
) -> foks_rpc::Result<usize> {
    let marker = read_byte_async(reader).await?;
    let length = match marker {
        0x00..=0x7f => usize::from(marker),
        0xcc => {
            let value = usize::from(read_byte_async(reader).await?);
            if value <= 0x7f {
                return Err(foks_rpc::Error::FrameLengthMarker(marker));
            }
            value
        }
        0xcd => {
            let value = usize::from(read_u16_async(reader).await?);
            if value <= usize::from(u8::MAX) {
                return Err(foks_rpc::Error::FrameLengthMarker(marker));
            }
            value
        }
        0xce => {
            let value = usize::try_from(read_u32_async(reader).await?).map_err(|_| {
                foks_rpc::Error::FrameTooLarge {
                    received: usize::MAX,
                    maximum,
                }
            })?;
            if value <= usize::from(u16::MAX) {
                return Err(foks_rpc::Error::FrameLengthMarker(marker));
            }
            value
        }
        _ => return Err(foks_rpc::Error::FrameLengthMarker(marker)),
    };
    if length > maximum {
        return Err(foks_rpc::Error::FrameTooLarge {
            received: length,
            maximum,
        });
    }
    Ok(length)
}

async fn read_call_body_async<R: AsyncRead + Unpin>(
    reader: &mut R,
    length: usize,
    maximum: usize,
    request_memory: Arc<Semaphore>,
) -> foks_rpc::Result<(foks_rpc::InboundMessage, OwnedSemaphorePermit)> {
    let reserved = length
        .checked_mul(2)
        .ok_or(foks_rpc::Error::FrameTooLarge {
            received: length,
            maximum,
        })?;
    let reserved = u32::try_from(reserved).map_err(|_| foks_rpc::Error::FrameTooLarge {
        received: length,
        maximum,
    })?;
    let permit = request_memory
        .acquire_many_owned(reserved)
        .await
        .map_err(|_| {
            foks_rpc::Error::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "request memory budget closed",
            ))
        })?;
    let mut content = vec![0; length];
    reader.read_exact(&mut content).await?;
    Ok((foks_rpc::decode_message(&content)?, permit))
}

async fn read_byte_async<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<u8> {
    let mut byte = [0];
    reader.read_exact(&mut byte).await?;
    Ok(byte[0])
}

async fn read_u16_async<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<u16> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes).await?;
    Ok(u16::from_be_bytes(bytes))
}

async fn read_u32_async<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(u32::from_be_bytes(bytes))
}

async fn write_response<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    response: &[u8],
    timeout: std::time::Duration,
) -> Result<()> {
    tokio::time::timeout(timeout, async {
        writer.write_all(response).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| crate::Error::Io(timeout_error("response write timed out")))??;
    Ok(())
}

fn timeout_error(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::TimedOut, message)
}

impl ServerData {
    fn long_poll(
        &self,
        route: &'static RouteSpec,
        sequence: u64,
        request_memory: OwnedSemaphorePermit,
    ) -> LongPoll<'_> {
        LongPoll {
            metrics: &self.metrics,
            faults: self.session_faults.as_deref(),
            route,
            sequence,
            request_memory,
        }
    }

    fn should_disconnect(
        &self,
        point: crate::SessionFaultPoint,
        protocol: &str,
        method: &str,
    ) -> bool {
        self.session_faults
            .as_ref()
            .is_some_and(|faults| faults.disconnect(point, protocol, method))
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    let mut message = error.to_string();
    if message.len() > 160 {
        let mut end = 160;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    RpcStatus::BadArguments(message)
}

/// Half of the reader pool, and at least one reader.
fn realtime_poll_read_permits(maximum_read_connections: usize) -> usize {
    (maximum_read_connections / 2).max(1)
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("active device does not authorize this request".to_owned())
}

fn endpoint_host(endpoint: &str) -> Option<&str> {
    if let Some(endpoint) = endpoint.strip_prefix('[') {
        let (host, port) = endpoint.split_once("]:")?;
        (!host.is_empty() && port.parse::<u16>().is_ok()).then_some(host)
    } else {
        let (host, port) = endpoint.rsplit_once(':')?;
        (!host.is_empty() && port.parse::<u16>().is_ok()).then_some(host)
    }
}

impl ServerData {
    fn authorize_principal(&self, principal: &Principal) -> std::result::Result<(), RpcStatus> {
        let reader = self.read_database()?;
        let now = self
            .clock
            .now_micros()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        if reader
            .sso_require_access(principal.uid(), now / 1000)
            .is_ok()
        {
            return Ok(());
        }
        drop(reader);
        let sso = self.sso.as_ref().ok_or_else(|| {
            RpcStatus::OAuth2Auth(Box::new(RpcStatus::OAuth2(
                "provider disabled; service access remains locked".into(),
            )))
        })?;
        sso.ensure_access(principal.uid(), principal.device_id())
            .map_err(crate::sso::access_status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG_POLL_ROUTES: [RouteId; 3] = [
        RouteId::RegPollOAuth2SessionCompletion,
        RouteId::KexReceive,
        RouteId::RealTimeRtPollInbox,
    ];

    fn route_spec(id: RouteId) -> &'static RouteSpec {
        crate::rpc::ROUTES
            .iter()
            .find(|route| route.id == id)
            .unwrap()
    }

    fn long_poll<'a>(
        metrics: &'a Arc<crate::ServerMetrics>,
        faults: &'a crate::SessionFaults,
        memory: &Arc<Semaphore>,
        id: RouteId,
    ) -> LongPoll<'a> {
        LongPoll {
            metrics,
            faults: Some(faults),
            route: route_spec(id),
            sequence: 7,
            request_memory: Arc::clone(memory).try_acquire_owned().unwrap(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn long_poll_run_honors_an_armed_fault_for_each_long_poll_route() {
        for id in LONG_POLL_ROUTES {
            let metrics = Arc::new(crate::ServerMetrics::default());
            let faults = crate::SessionFaults::default();
            let memory = Arc::new(Semaphore::new(1));
            let route = route_spec(id);
            faults.arm(
                crate::SessionFaultPoint::BeforeDurableMutation,
                route.protocol,
                route.method,
            );
            let (_stop, mut stop) = watch::channel(false);
            let polled = std::cell::Cell::new(false);
            let outcome = long_poll(&metrics, &faults, &memory, id)
                .run(std::future::pending::<()>(), &mut stop, async {
                    polled.set(true);
                    Ok(Vec::new())
                })
                .await
                .unwrap()
                .unwrap();
            assert!(outcome.disconnect_before_response, "{id:?}");
            assert_eq!(outcome.route.map(|route| route.id), Some(id));
            assert!(!polled.get(), "{id:?}");
            assert_eq!(faults.hits(), 1, "{id:?}");
            assert_eq!(metrics.snapshot().handler_duration_observations, 1);
            assert_eq!(memory.available_permits(), 1);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn long_poll_results_are_encoded_and_timed() {
        let metrics = Arc::new(crate::ServerMetrics::default());
        let faults = crate::SessionFaults::default();
        let memory = Arc::new(Semaphore::new(1));
        let (_stop, mut stop) = watch::channel(false);

        let response = foks_rpc::encode_success_response_at(&[0x91, 0x01], 7).unwrap();
        let outcome = long_poll(
            &metrics,
            &faults,
            &memory,
            RouteId::RegPollOAuth2SessionCompletion,
        )
        .run(std::future::pending::<()>(), &mut stop, async {
            Ok(response.clone())
        })
        .await
        .unwrap()
        .unwrap();
        assert!(!outcome.disconnect_before_response);
        assert_eq!(outcome.response, response);
        assert_eq!(outcome.status, STATUS_OK);

        let outcome = long_poll(&metrics, &faults, &memory, RouteId::RealTimeRtPollInbox)
            .run(std::future::pending::<()>(), &mut stop, async {
                Err(RpcStatus::RateLimited)
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.response,
            encode_status_response_at(&RpcStatus::RateLimited, 7).unwrap()
        );
        assert_eq!(outcome.status, RpcStatus::RateLimited.code());
        assert_eq!(metrics.snapshot().handler_duration_observations, 2);
        assert_eq!(faults.hits(), 0);
        assert_eq!(memory.available_permits(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn long_polls_close_unanswered_when_the_peer_is_readable_or_the_server_stops() {
        let metrics = Arc::new(crate::ServerMetrics::default());
        let faults = crate::SessionFaults::default();
        let memory = Arc::new(Semaphore::new(1));
        let (stop_sender, mut stop) = watch::channel(false);
        let pending = std::future::pending::<std::result::Result<Vec<u8>, RpcStatus>>;

        let outcome = long_poll(&metrics, &faults, &memory, RouteId::KexReceive)
            .run(std::future::ready(()), &mut stop, pending())
            .await
            .unwrap();
        assert!(outcome.is_none());
        assert_eq!(memory.available_permits(), 1);

        stop_sender.send(true).unwrap();
        let outcome = long_poll(&metrics, &faults, &memory, RouteId::RealTimeRtPollInbox)
            .run(std::future::pending::<()>(), &mut stop, pending())
            .await
            .unwrap();
        assert!(outcome.is_none());
        assert_eq!(memory.available_permits(), 1);
        assert_eq!(metrics.snapshot().handler_duration_observations, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timed_out_execution_remains_bounded_until_work_finishes() {
        let execution = Arc::new(Semaphore::new(1));
        let (_stop, mut receiver) = watch::channel(false);
        let (entered_sender, entered_receiver) = std::sync::mpsc::sync_channel(1);
        let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(1);

        let first = execute_bounded(
            Arc::clone(&execution),
            std::time::Duration::from_millis(20),
            &mut receiver,
            move || {
                entered_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                1_u8
            },
        );
        let result = first.await.unwrap();
        entered_receiver.recv().unwrap();
        assert!(matches!(result, BoundedExecution::TimedOut));
        assert_eq!(execution.available_permits(), 0);

        assert!(matches!(
            execute_bounded(
                Arc::clone(&execution),
                std::time::Duration::from_secs(1),
                &mut receiver,
                || 2_u8,
            )
            .await
            .unwrap(),
            BoundedExecution::Saturated
        ));

        release_sender.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while execution.available_permits() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn async_frame_memory_is_globally_bounded_before_allocation() {
        let argument = foks_snowpack::encode(&Value::Binary(vec![0x41; 300])).unwrap();
        let request = foks_rpc::encode_call(17, 23, &argument, 0).unwrap();
        assert_eq!(request[0], 0xcd);
        let content_length = foks_rpc::read_frame(
            &mut std::io::Cursor::new(&request),
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
        )
        .unwrap()
        .len();
        let budget = Arc::new(Semaphore::new(content_length * 2));
        let (mut sender_one, mut reader_one) = tokio::io::duplex(request.len());
        let (mut sender_two, mut reader_two) = tokio::io::duplex(request.len());
        sender_one.write_all(&request).await.unwrap();
        sender_two.write_all(&request).await.unwrap();

        let first_length =
            read_frame_length_async(&mut reader_one, foks_rpc::DEFAULT_MAX_FRAME_LENGTH)
                .await
                .unwrap();
        let first = read_call_body_async(
            &mut reader_one,
            first_length,
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            Arc::clone(&budget),
        )
        .await
        .unwrap();
        match &first.0 {
            foks_rpc::InboundMessage::Call(call) => assert_eq!(call.argument(), argument),
            foks_rpc::InboundMessage::Control => panic!("expected a CALL_V2 request"),
        }
        assert_eq!(budget.available_permits(), 0);

        let second_length =
            read_frame_length_async(&mut reader_two, foks_rpc::DEFAULT_MAX_FRAME_LENGTH)
                .await
                .unwrap();
        let mut second = Box::pin(read_call_body_async(
            &mut reader_two,
            second_length,
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            Arc::clone(&budget),
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut second)
                .await
                .is_err()
        );
        drop(first);
        let second = tokio::time::timeout(std::time::Duration::from_secs(1), second)
            .await
            .unwrap()
            .unwrap();
        match &second.0 {
            foks_rpc::InboundMessage::Call(call) => assert_eq!(call.argument(), argument),
            foks_rpc::InboundMessage::Control => panic!("expected a CALL_V2 request"),
        }
    }
}
