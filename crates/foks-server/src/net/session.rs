use std::sync::Arc;

mod handlers;
mod kex;
#[cfg(feature = "celld-proof")]
mod proof;

use foks_proto::{EntityId, MerkleRoot};
use foks_rpc::{encode_status_response_at, RpcStatus, STATUS_OK, STATUS_RATE_LIMIT_ERROR};
use foks_snowpack::{decode, Value};
use rustls::pki_types::CertificateDer;
#[cfg(test)]
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{watch, Semaphore};

use crate::auth::Principal;
use crate::keys::HostKeyProvider;
use crate::rpc::{route_call, Listener, RouteError, RouteId, RouteSpec};
use crate::services::merkle_query::{validate_host_argument_against, MerkleQueries};
use crate::services::IdentityPorts;
use crate::{Entropy, Result, SessionLimits, WriterHandle};

#[derive(Clone)]
pub(crate) struct ServerData {
    #[cfg(feature = "celld-proof")]
    proof: Option<Arc<dyn crate::proof::RpcProof>>,
    chunk_store: Option<Arc<dyn crate::chunks::ChunkStore>>,
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
    /// Outstanding anonymous KEX receives, across all connections.
    kex_polling: Arc<Semaphore>,
    /// Outstanding KEX receives per source, so that one source cannot hold
    /// the whole `kex_polling` budget. Loopback peers and trusted sources
    /// are exempt.
    kex_polling_sources: Arc<crate::rate_limit::KeyedLimit<crate::rate_limit::SourceKey>>,
    request_memory: Arc<Semaphore>,
    small_request_memory: Arc<Semaphore>,
    memory: Arc<super::memory::Pools>,
    kex_relay: Arc<kex::Relay>,
    realtime: crate::services::realtime::RealtimeService,
    waitlist_enabled: bool,
    // Exclusion outlives all contained resources, including abandoned work.
    _storage_lease: Option<Arc<crate::storage_lease::StorageLease>>,
}

impl ServerData {
    #[cfg(feature = "celld-proof")]
    pub(crate) fn with_proof(mut self, proof: Option<Arc<dyn crate::proof::RpcProof>>) -> Self {
        self.proof = proof;
        self
    }

    fn proof_enabled(&self) -> bool {
        #[cfg(feature = "celld-proof")]
        {
            self.proof.is_some()
        }
        #[cfg(not(feature = "celld-proof"))]
        {
            false
        }
    }

    /// `read_opener` opens the request reader pool; it is `Some` exactly when
    /// `config.read_database` is.
    pub(crate) fn from_config(
        config: &crate::Config,
        read_opener: Option<foks_server_db::ReadDatabaseOpener>,
        rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    ) -> Result<Self> {
        crate::config::validate_vhost_management_host(&config.vhost_management_host)?;
        let storage_lease = config
            .read_database
            .as_ref()
            .map(|db| {
                crate::storage_lease::StorageLease::with_store(
                    &db.path,
                    config.chunk_store.as_deref(),
                    false,
                )
                .map(Arc::new)
            })
            .transpose()?;
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
            _storage_lease: storage_lease,
            #[cfg(feature = "celld-proof")]
            proof: None,
            sso: config.sso.clone(),
            web_admin: config.web_admin.clone(),
            vhost_management_host: config.vhost_management_host.clone(),
            peer_ip: None,
            probe_response,
            host_id,
            canonical_name,
            probe_endpoint,
            current_root: probe.merkle_root.encoded()?,
            read_database: read_opener
                .map(|opener| {
                    let pool = crate::read_pool::ReadPool::with_opener(
                        opener,
                        config.limits.maximum_read_connections,
                    )?;
                    config.metrics.reader_pool_opened(pool.open_connections()?);
                    if config.chunk_store.is_none() && pool.checkout()?.has_external_chunks()? {
                        return Err(crate::Error::Chunk(
                            "external references require a configured chunk store",
                        ));
                    }
                    Ok::<_, crate::Error>(pool)
                })
                .transpose()?,
            writer: config.writer.clone(),
            chunk_store: config.chunk_store.clone(),
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
            kex_polling: Arc::new(Semaphore::new(config.limits.kex_receive_capacity())),
            kex_polling_sources: Arc::new(crate::rate_limit::KeyedLimit::new(
                kex_receive_source_share(config.limits.kex_receive_capacity()),
            )),
            request_memory: Arc::new(Semaphore::new(config.limits.maximum_request_memory_bytes)),
            small_request_memory: Arc::new(Semaphore::new(
                config.limits.maximum_control_memory_bytes,
            )),
            memory: super::memory::pools(config.limits)?,
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
    enable_keepalive(&stream)?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::clone(tls));
    let handshake = tokio::select! {
        result = stop.changed() => {
            let _ = result;
            return Ok(());
        }
        result = tokio::time::timeout(limits.io_timeout, acceptor.accept(stream)) => result,
    };
    let stream =
        handshake.map_err(|_| crate::Error::Io(timeout_error("TLS handshake timed out")))??;
    let peer_certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .map(|certificate| certificate.as_ref().to_vec());
    // Reading continues while long polls are pending, so that a call the peer
    // pipelines behind a poll is served instead of being taken for a
    // disconnect. Reads go through TLS, so a frame rustls already decrypted
    // and buffered is seen even when the socket itself is not readable.
    let (mut reader, writer) = tokio::io::split(stream);
    let mut responder = Responder {
        writer,
        polls: PendingPolls::default(),
        data: service_data.as_ref(),
        limits,
    };

    let mut frames = 0_usize;
    loop {
        // Once the connection's frame budget is spent, the session reads only
        // to see cancels and end of stream while it answers the long polls
        // still pending, and ends when none is left.
        let budget_spent = frames >= limits.maximum_requests;
        if budget_spent && responder.polls.is_empty() {
            break;
        }
        // The request deadline starts at the first byte of the frame. Body
        // reading, handler execution and queued writes all consume it; idle
        // time between requests does not, and the response write gets its
        // own allowance. A connection with a pending long poll is not idle.
        let read = match responder
            .wait(
                read_frame_ingress(
                    &mut reader,
                    limits.maximum_frame_bytes,
                    limits.request_timeout,
                ),
                Some(limits.io_timeout),
                budget_spent,
                &mut stop,
            )
            .await?
        {
            Waited::Done(read) => read,
            Waited::Idle | Waited::Drained | Waited::Ended => break,
        };
        frames = frames.saturating_add(1);
        let (length, deadline) = match read {
            Ok(length) => length,
            Err(foks_rpc::Error::Io(error))
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
            Err(error) => return Err(error.into()),
        };
        // A request over its source's bucket is still read, within the frame
        // limit and the request memory budget, so that its refusal carries
        // the call's own sequence number. Go and Rust clients number calls
        // per connection, so a refusal at any other sequence would not match
        // the pending call.
        let request_allowed = service_data.rate_limiter.allow_request(peer_ip);
        if !request_allowed && service_data.proof_enabled() {
            return Ok(());
        }
        let read = match responder
            .wait(
                tokio::time::timeout_at(
                    phase_deadline(deadline, limits.io_timeout)?,
                    read_call_body_async(
                        &mut reader,
                        length,
                        limits.maximum_frame_bytes,
                        Arc::clone(if length <= 4096 {
                            &service_data.small_request_memory
                        } else {
                            &service_data.request_memory
                        }),
                        Arc::clone(&service_data.memory),
                    ),
                ),
                None,
                false,
                &mut stop,
            )
            .await?
        {
            Waited::Done(read) => read,
            Waited::Idle | Waited::Drained | Waited::Ended => break,
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
            Inbound::Call(call) => call,
            // A control frame over the request bucket has no sequence to
            // answer at, so the connection closes without a response.
            Inbound::Cancel(_) | Inbound::Control if !request_allowed => return Ok(()),
            // The call's argument did not fit the working memory budget
            // while it was decoded. No handler ran, so the refusal is a
            // definite rejection and is answered at the call's sequence.
            Inbound::OverBudget(sequence) => {
                drop(request_memory);
                if service_data.proof_enabled() {
                    return Ok(());
                }
                service_data.metrics.request_started();
                let _request_timer =
                    crate::ServerMetrics::request_timer(Arc::clone(&service_data.metrics));
                service_data
                    .metrics
                    .request_rate_limited(crate::AdmissionRejection::RequestMemory);
                let memory = service_data.memory.request(None);
                responder
                    .refuse(&memory, sequence, crate::metrics::UNROUTED, deadline)
                    .await?;
                return Ok(());
            }
            // go-snowpack-rpc cancels a call whose context ended, as go-foks
            // KEX does for the receive it no longer needs. A pending long
            // poll for that sequence ends without a response, releasing its
            // permits; a cancel for anything else is ignored.
            Inbound::Cancel(sequence) => {
                drop(request_memory);
                responder.polls.cancel(sequence);
                continue;
            }
            // NOTIFY control messages are never answered by go-snowpack-rpc.
            // Discard the frame and keep the connection open instead of
            // treating the unexpected method type as fatal.
            Inbound::Control => {
                drop(request_memory);
                continue;
            }
        };
        if !request_allowed {
            service_data.metrics.request_started();
            let _request_timer =
                crate::ServerMetrics::request_timer(Arc::clone(&service_data.metrics));
            service_data
                .metrics
                .request_rate_limited(crate::AdmissionRejection::RequestRate);
            let sequence = call.sequence();
            drop(call);
            responder
                .refuse(
                    &request_memory,
                    sequence,
                    crate::metrics::UNROUTED,
                    deadline,
                )
                .await?;
            return Ok(());
        }
        if budget_spent {
            // The frame budget is spent; only the pending polls are served.
            service_data.metrics.request_started();
            let _request_timer =
                crate::ServerMetrics::request_timer(Arc::clone(&service_data.metrics));
            let sequence = call.sequence();
            drop(call);
            responder
                .refuse(
                    &request_memory,
                    sequence,
                    crate::metrics::UNROUTED,
                    deadline,
                )
                .await?;
            continue;
        }
        service_data.metrics.request_started();
        let request_timer = crate::ServerMetrics::request_timer(Arc::clone(&service_data.metrics));
        let sequence = call.sequence();
        let argument_bytes = call.argument().len();
        let data = Arc::clone(service_data);
        let certificate = peer_certificate.clone();
        let routed = route_call(call, listener);
        let family = routed
            .as_ref()
            .map_or(crate::metrics::UNROUTED, |call| call.route.protocol);
        let proof = service_data.proof_enabled();
        let route_id = routed.as_ref().ok().map(|call| call.route.id);
        let workload = if !proof && route_id == Some(RouteId::KvStoreGetEncryptedChunk) {
            super::memory::Workload::ChunkDownload {
                maximum_encoded_bytes: crate::chunks::MAXIMUM_ENCODED_CHUNK_BYTES,
            }
        } else {
            super::memory::Workload::Ordinary
        };
        if !request_memory.admit_working(argument_bytes, workload) {
            if proof {
                return Ok(());
            }
            service_data
                .metrics
                .request_rate_limited(crate::AdmissionRejection::RequestMemory);
            responder
                .refuse(&request_memory, sequence, family, deadline)
                .await?;
            return Ok(());
        }
        deadline.remaining()?;
        let long_poll = !proof
            && matches!(
                route_id,
                Some(
                    RouteId::RegPollOAuth2SessionCompletion
                        | RouteId::KexReceive
                        | RouteId::RealTimeRtPollInbox
                )
            );
        let routed = match routed {
            Ok(call) if long_poll => {
                // A long poll waits on the connection task, outside the
                // request deadline, while the session reads and serves the
                // peer's next frames. Its response is written when it
                // completes, in completion order.
                // A refused poll is answered at its sequence and the session
                // goes on: the frame was read in full, and the peer's other
                // calls on this connection stay pending.
                if responder.polls.len() >= MAXIMUM_PENDING_POLLS {
                    service_data
                        .metrics
                        .request_rate_limited(crate::AdmissionRejection::ConnectionLongPolls);
                    responder
                        .refuse(&request_memory, sequence, family, deadline)
                        .await?;
                    continue;
                }
                let pending = match call.route.id {
                    RouteId::KexReceive => {
                        let (poll, kex, kex_source) = match service_data.kex_long_poll(
                            call.route,
                            sequence,
                            request_memory,
                            peer_ip,
                        ) {
                            Ok(admitted) => admitted,
                            Err(request_memory) => {
                                responder
                                    .refuse(&request_memory, sequence, family, deadline)
                                    .await?;
                                continue;
                            }
                        };
                        let handler = handlers::kex_receive_response(service_data.as_ref(), call);
                        PendingPoll {
                            sequence,
                            family,
                            kex: Some(kex),
                            kex_source,
                            _request_timer: request_timer,
                            future: Box::pin(async move {
                                let mut completion = None;
                                let handler = async {
                                    match handler.await {
                                        Ok(response) => {
                                            completion =
                                                Some(crate::metrics::kex::Completion::Delivered);
                                            Ok(response)
                                        }
                                        Err(error) => {
                                            completion = Some(error.completion);
                                            Err(error.status)
                                        }
                                    }
                                };
                                let outcome = poll.run(handler).await?;
                                Ok(PolledOutcome {
                                    outcome,
                                    kex_completion: completion,
                                })
                            }),
                        }
                    }
                    RouteId::RealTimeRtPollInbox => {
                        let poll = service_data.long_poll(call.route, sequence, request_memory);
                        let handler = handlers::realtime_poll_response(
                            service_data.as_ref(),
                            call,
                            certificate,
                        );
                        PendingPoll::new(sequence, family, request_timer, poll, handler)
                    }
                    _ => {
                        let poll = service_data.long_poll(call.route, sequence, request_memory);
                        let handler = handlers::sso_poll_response(service_data.as_ref(), call);
                        PendingPoll::new(sequence, family, request_timer, poll, handler)
                    }
                };
                responder.polls.push(pending);
                continue;
            }
            routed => routed,
        };
        let _request_timer = request_timer;
        let mut bounded_stop = stop.clone();
        let execution = execute_bounded(
            Arc::clone(&service_data.execution),
            deadline,
            &mut bounded_stop,
            move || -> Result<RequestOutcome> {
                // The scope's deadline applies to reader checkout, queued
                // writes and chunk transfers inside the handler.
                crate::deadline::check()?;
                let scope = Arc::clone(&request_memory);
                scope.sync(|| {
                    // A timed-out blocking handler can still own decoded
                    // request memory, so keep its reservation in the same
                    // closure.
                    let _request_memory = Arc::clone(&request_memory);
                    let _handler_timer =
                        crate::ServerMetrics::handler_timer(Arc::clone(&data.metrics));
                    #[cfg(feature = "celld-proof")]
                    if data.proof_enabled() {
                        let route = routed.as_ref().ok().map(|call| call.route);
                        let response = proof::response(
                            &data,
                            listener,
                            routed,
                            certificate,
                            peer_ip,
                            sequence,
                        )?;
                        return Ok(RequestOutcome {
                            response,
                            status: STATUS_OK,
                            route,
                            disconnect_before_response: false,
                            kex: None,
                            _memory: Arc::clone(&request_memory),
                        });
                    }
                    let principal = if listener == Listener::Authenticated {
                        // A failed authentication is answered on the open
                        // connection, as go-foks answers every call of an
                        // unauthenticated connection with AUTH_ERROR.
                        match authenticate_request(&data, certificate) {
                            Ok(principal) => Some(principal),
                            Err(status) => {
                                let (response, status) = encode_result(Err(status), sequence)?;
                                return Ok(RequestOutcome {
                                    response,
                                    status,
                                    route: None,
                                    disconnect_before_response: false,
                                    kex: None,
                                    _memory: Arc::clone(&request_memory),
                                });
                            }
                        }
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
                                    kex: None,
                                    _memory: Arc::clone(&request_memory),
                                });
                            }
                            let mut request_data = data.as_ref().clone();
                            request_data.peer_ip = Some(peer_ip);
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
                        kex: None,
                        _memory: Arc::clone(&request_memory),
                    })
                })
            },
        );
        let execution = match responder.wait(execution, None, false, &mut stop).await? {
            Waited::Done(execution) => execution?,
            Waited::Idle | Waited::Drained | Waited::Ended => break,
        };
        let outcome = match execution {
            BoundedExecution::Completed(outcome) => outcome?,
            BoundedExecution::Saturated => {
                if service_data.proof_enabled() {
                    return Ok(());
                }
                service_data
                    .metrics
                    .request_rate_limited(crate::AdmissionRejection::RequestHandlers);
                let memory = service_data.memory.request(None);
                responder
                    .refuse(&memory, sequence, family, deadline)
                    .await?;
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
        };
        match responder
            .write_outcome(outcome, family, sequence, &mut stop)
            .await?
        {
            Flow::Continue => {}
            Flow::End => return Ok(()),
        }
    }
    Ok(())
}

/// The write side of one session, with the long polls whose responses are
/// still pending on it.
struct Responder<'a, W> {
    writer: W,
    polls: PendingPolls<'a>,
    data: &'a ServerData,
    limits: SessionLimits,
}

/// How [`Responder::wait`] ended.
enum Waited<T> {
    Done(T),
    /// No frame arrived within the idle allowance and no poll was pending.
    Idle,
    /// The last pending poll was answered while waiting `until_drained`.
    Drained,
    /// The server is stopping, or a poll's response could not be written.
    Ended,
}

/// Whether the session continues after a response.
enum Flow {
    Continue,
    End,
}

impl<'a, W: tokio::io::AsyncWrite + Unpin> Responder<'a, W> {
    /// Awaits `operation` while writing the response of each long poll that
    /// completes meanwhile. With `idle`, gives up once no poll has been
    /// pending for that long; with `until_drained`, gives up as soon as the
    /// last pending poll has been answered.
    async fn wait<F: std::future::Future>(
        &mut self,
        operation: F,
        idle: Option<std::time::Duration>,
        until_drained: bool,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Waited<F::Output>> {
        tokio::pin!(operation);
        let idle_after = |idle: std::time::Duration| tokio::time::Instant::now() + idle;
        let mut idle_deadline = idle.map(idle_after);
        loop {
            let polls_pending = !self.polls.is_empty();
            tokio::select! {
                biased;
                result = stop.changed() => {
                    let _ = result;
                    self.polls.closing = crate::metrics::kex::Completion::Shutdown;
                    return Ok(Waited::Ended);
                }
                completed = self.polls.next_completed(), if polls_pending => {
                    let completed = completed?;
                    // The request timer observes the duration through the
                    // response write.
                    let _request_timer = completed.request_timer;
                    match self
                        .write_outcome(completed.outcome, completed.family, completed.sequence, stop)
                        .await?
                    {
                        Flow::Continue => {}
                        Flow::End => return Ok(Waited::Ended),
                    }
                    if until_drained && self.polls.is_empty() {
                        return Ok(Waited::Drained);
                    }
                    if self.polls.is_empty() {
                        idle_deadline = idle.map(idle_after);
                    }
                }
                output = &mut operation => return Ok(Waited::Done(output)),
                () = sleep_until_option(idle_deadline), if !polls_pending && idle_deadline.is_some() => {
                    return Ok(Waited::Idle);
                }
            }
        }
    }

    /// Answers `sequence` with `RateLimited` for a refused request.
    async fn refuse(
        &mut self,
        memory: &Arc<super::memory::Request>,
        sequence: u64,
        family: &'static str,
        deadline: crate::deadline::Deadline,
    ) -> Result<()> {
        let response =
            memory.sync(|| encode_status_response_at(&RpcStatus::RateLimited, sequence))?;
        write_response_until(
            &mut self.writer,
            &response,
            deadline,
            self.limits.io_timeout,
        )
        .await?;
        self.data
            .metrics
            .response_completed(family, STATUS_RATE_LIMIT_ERROR);
        Ok(())
    }

    /// Writes a request's response, applying the armed session faults.
    async fn write_outcome(
        &mut self,
        outcome: RequestOutcome,
        family: &'static str,
        sequence: u64,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Flow> {
        if outcome.disconnect_before_response {
            return Ok(Flow::End);
        }
        // An operation may already have committed before a working or output
        // allocation failed. Preserve that ambiguity: never translate the
        // failure into a status that implies a known non-write. A route that
        // only reads has nothing to leave ambiguous and is refused instead.
        if outcome._memory.failed() {
            let refusal = memory_failure_refusal(outcome.route, sequence, &self.data.memory)?;
            drop(outcome);
            if let Some(response) = refusal {
                self.data
                    .metrics
                    .request_rate_limited(crate::AdmissionRejection::RequestMemory);
                write_response_until(
                    &mut self.writer,
                    &response,
                    response_write_deadline(self.limits.io_timeout)?,
                    self.limits.io_timeout,
                )
                .await?;
                self.data
                    .metrics
                    .response_completed(family, STATUS_RATE_LIMIT_ERROR);
            }
            return Ok(Flow::End);
        }
        let _memory = outcome._memory;
        let response_deadline = response_write_deadline(self.limits.io_timeout)?;
        // A KEX receive keeps its budget permit until its response is written.
        let mut kex = outcome.kex;
        let response = outcome.response;
        let status = outcome.status;
        if let Some(route) = outcome.route {
            let protocol = route.protocol;
            let method = route.method;
            let point = if route.id == crate::rpc::RouteId::KvStoreFileUploadChunk {
                crate::SessionFaultPoint::BetweenLargeFileChunks
            } else {
                crate::SessionFaultPoint::AfterDurableCommitBeforeResponse
            };
            if self.data.should_disconnect(point, protocol, method) {
                return Ok(Flow::End);
            }
            if self.data.should_disconnect(
                crate::SessionFaultPoint::DuringResponseWrite,
                protocol,
                method,
            ) {
                let split = response.len().div_ceil(2);
                write_response_until(
                    &mut self.writer,
                    &response[..split],
                    response_deadline,
                    self.limits.io_timeout,
                )
                .await?;
                return Ok(Flow::End);
            }
        }
        let write = tokio::select! {
            result = stop.changed() => {
                let _ = result;
                if let Some(kex) = kex.take() {
                    kex.stopped();
                }
                self.polls.closing = crate::metrics::kex::Completion::Shutdown;
                return Ok(Flow::End);
            }
            result = write_response_until(
                &mut self.writer,
                &response,
                response_deadline,
                self.limits.io_timeout,
            ) => result,
        };
        write?;
        if let Some(kex) = kex {
            kex.written();
        }
        self.data.metrics.response_completed(family, status);
        Ok(Flow::Continue)
    }
}

/// TCP keepalive on accepted RPC connections: probes after 15 s idle, every
/// 15 s, nine times, as a Go `net.Listen` listener sets them. A peer that
/// vanished without closing its connection is then detected within about
/// two and a half minutes, so that a long poll it left pending, such as an
/// hour-long KEX receive, does not keep its permits until it times out.
fn enable_keepalive(stream: &tokio::net::TcpStream) -> std::io::Result<()> {
    let period = std::time::Duration::from_secs(15);
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(period)
        .with_interval(period);
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    let keepalive = keepalive.with_retries(9);
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)
}

/// The deadline for writing a completed request's response: a fresh
/// `io_timeout` allowance. The request deadline bounds the body read, the
/// handler and its queued writes; a long poll waits on its own timeout. In
/// either case the work is done once the response exists, and a write that
/// timed out on what remained would close the connection, discard the
/// response and make the client repeat the work.
fn response_write_deadline(io_timeout: std::time::Duration) -> Result<crate::deadline::Deadline> {
    Ok(crate::deadline::Deadline::after(io_timeout)?)
}

async fn sleep_until_option(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

struct RequestOutcome {
    response: Vec<u8>,
    /// The wire status code that `response` carries, `STATUS_OK` for a success.
    status: u64,
    route: Option<&'static crate::rpc::RouteSpec>,
    disconnect_before_response: bool,
    /// Set for a KEX receive; see [`KexReceive`].
    kex: Option<KexReceive>,
    // Fields drop in declaration order: buffers must go before reservations.
    _memory: Arc<super::memory::Request>,
}

/// An answered KEX receive's permit of the global KEX receive budget and its
/// metrics. The completion the handler reported is recorded only once the
/// response has been written; a receive dropped before that, by a failed or
/// skipped write, is recorded as failed.
struct KexReceive {
    poll: crate::metrics::kex::Poll,
    /// The receive's slot of its source's share, released with the permit.
    _source: KexSourcePermit,
    /// `None` when the handler did not run, after an armed fault.
    completion: Option<crate::metrics::kex::Completion>,
}

impl KexReceive {
    fn written(mut self) {
        if let Some(completion) = self.completion {
            self.poll.complete(completion);
        }
    }

    fn stopped(mut self) {
        self.poll
            .complete(crate::metrics::kex::Completion::Shutdown);
    }
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

/// Runs `operation` on the blocking pool within the request deadline scope.
/// The session stops waiting at the deadline; the operation itself keeps its
/// handler permit until it returns.
async fn execute_bounded<T>(
    execution: Arc<Semaphore>,
    deadline: crate::deadline::Deadline,
    stop: &mut watch::Receiver<bool>,
    operation: impl FnOnce() -> T + Send + 'static,
) -> Result<BoundedExecution<T>>
where
    T: Send + 'static,
{
    if deadline.remaining().is_err() {
        return Ok(BoundedExecution::TimedOut);
    }
    let Ok(permit) = execution.try_acquire_owned() else {
        return Ok(BoundedExecution::Saturated);
    };
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::deadline::scope(Some(deadline), operation)
    });
    tokio::select! {
        result = stop.changed() => {
            let _ = result;
            Ok(BoundedExecution::Stopped)
        }
        result = tokio::time::timeout_at(deadline.instant().into(), &mut task) => {
            match result {
                Ok(Ok(value)) => Ok(BoundedExecution::Completed(value)),
                Ok(Err(_)) => Err(crate::Error::Thread),
                Err(_) => Ok(BoundedExecution::TimedOut),
            }
        }
    }
}

/// Most long polls one connection can hold pending at once. Go clients
/// multiplex calls on one connection: while provisioning a device, go-foks
/// holds up to three KEX receives at once (client/libclient/kex.go). A long
/// poll over this limit is refused with `RateLimited` at its sequence, and
/// the connection goes on serving.
const MAXIMUM_PENDING_POLLS: usize = 4;

/// One long-poll request: the SSO completion poll, a KEX receive or a
/// realtime inbox poll. A Go-compatible long poll can remain open for a long
/// time, so it is awaited on the connection task rather than a blocking
/// worker, while the session goes on serving the connection's other
/// requests. The listener's active-connection semaphore and
/// [`MAXIMUM_PENDING_POLLS`] bound how many run at once, and KEX receives
/// also hold a permit of the global KEX receive budget. Long polls run
/// outside the request deadline scope.
struct LongPoll<'a> {
    metrics: &'a Arc<crate::ServerMetrics>,
    faults: Option<&'a crate::SessionFaults>,
    route: &'static RouteSpec,
    sequence: u64,
    /// The request's memory reservations; they stay held until the poll's
    /// response has been written.
    request_memory: Arc<super::memory::Request>,
}

impl LongPoll<'_> {
    /// Applies the request fault check and handler timer that bounded
    /// requests get, then awaits `handler` with its allocations charged to the
    /// request.
    async fn run<H>(self, handler: H) -> Result<RequestOutcome>
    where
        H: std::future::Future<Output = std::result::Result<Vec<u8>, RpcStatus>>,
    {
        let _handler_timer = crate::ServerMetrics::handler_timer(Arc::clone(self.metrics));
        let route = self.route;
        if self.faults.is_some_and(|faults| {
            faults.disconnect(
                crate::SessionFaultPoint::BeforeDurableMutation,
                route.protocol,
                route.method,
            )
        }) {
            return Ok(RequestOutcome {
                response: Vec::new(),
                status: STATUS_OK,
                route: Some(route),
                disconnect_before_response: true,
                kex: None,
                _memory: self.request_memory,
            });
        }
        let result = self.request_memory.run(handler).await;
        let (response, status) = self
            .request_memory
            .sync(|| encode_result(result, self.sequence))?;
        Ok(RequestOutcome {
            response,
            status,
            route: Some(route),
            disconnect_before_response: false,
            kex: None,
            _memory: self.request_memory,
        })
    }
}

type PollFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<PolledOutcome>> + Send + 'a>>;

/// A completed long poll's outcome, and for a KEX receive the completion its
/// handler reported.
struct PolledOutcome {
    outcome: RequestOutcome,
    kex_completion: Option<crate::metrics::kex::Completion>,
}

/// A long poll admitted on a connection whose response is pending.
struct PendingPoll<'a> {
    sequence: u64,
    family: &'static str,
    /// A KEX receive's permit of the global KEX receive budget and its
    /// metrics. It is held outside the handler, so that a receive dropped
    /// unanswered is recorded with the reason it was dropped.
    kex: Option<crate::metrics::kex::Poll>,
    /// A KEX receive's slot of its source's share of the budget.
    kex_source: KexSourcePermit,
    /// Observes the request's duration, through its response write.
    _request_timer: crate::metrics::RequestTimer,
    future: PollFuture<'a>,
}

impl<'a> PendingPoll<'a> {
    fn new<H>(
        sequence: u64,
        family: &'static str,
        request_timer: crate::metrics::RequestTimer,
        poll: LongPoll<'a>,
        handler: H,
    ) -> Self
    where
        H: std::future::Future<Output = std::result::Result<Vec<u8>, RpcStatus>> + Send + 'a,
    {
        Self {
            sequence,
            family,
            kex: None,
            kex_source: None,
            _request_timer: request_timer,
            future: Box::pin(async move {
                Ok(PolledOutcome {
                    outcome: poll.run(handler).await?,
                    kex_completion: None,
                })
            }),
        }
    }

    /// Drops the poll unanswered, recording `completion` for a KEX receive.
    fn abandon(mut self, completion: crate::metrics::kex::Completion) {
        if let Some(kex) = self.kex.as_mut() {
            kex.complete(completion);
        }
    }
}

/// The long polls pending on one connection.
struct PendingPolls<'a> {
    polls: Vec<PendingPoll<'a>>,
    /// Recorded for each KEX receive still pending when the set is dropped:
    /// `Disconnected` unless the server is stopping.
    closing: crate::metrics::kex::Completion,
}

impl Default for PendingPolls<'_> {
    fn default() -> Self {
        Self {
            polls: Vec::new(),
            closing: crate::metrics::kex::Completion::Disconnected,
        }
    }
}

impl<'a> PendingPolls<'a> {
    fn len(&self) -> usize {
        self.polls.len()
    }

    fn is_empty(&self) -> bool {
        self.polls.is_empty()
    }

    fn push(&mut self, poll: PendingPoll<'a>) {
        self.polls.push(poll);
    }

    /// Drops the pending poll with `sequence`, if there is one, after its
    /// client cancelled the call.
    fn cancel(&mut self, sequence: u64) -> bool {
        let Some(index) = self.polls.iter().position(|poll| poll.sequence == sequence) else {
            return false;
        };
        self.polls
            .swap_remove(index)
            .abandon(crate::metrics::kex::Completion::Cancelled);
        true
    }

    /// Waits for any pending poll to complete and returns its outcome and
    /// route family. Cancellation safe: the polls stay pending when the
    /// returned future is dropped.
    async fn next_completed(&mut self) -> Result<CompletedPoll> {
        let (index, result) = std::future::poll_fn(|context| {
            for (index, poll) in self.polls.iter_mut().enumerate() {
                if let std::task::Poll::Ready(result) = poll.future.as_mut().poll(context) {
                    return std::task::Poll::Ready((index, result));
                }
            }
            std::task::Poll::Pending
        })
        .await;
        let poll = self.polls.swap_remove(index);
        let polled = result?;
        let mut outcome = polled.outcome;
        if let Some(kex) = poll.kex {
            outcome.kex = Some(KexReceive {
                poll: kex,
                _source: poll.kex_source,
                completion: polled.kex_completion,
            });
        }
        Ok(CompletedPoll {
            outcome,
            family: poll.family,
            sequence: poll.sequence,
            request_timer: poll._request_timer,
        })
    }
}

/// A long poll whose handler has completed, ready for its response write.
struct CompletedPoll {
    outcome: RequestOutcome,
    family: &'static str,
    sequence: u64,
    request_timer: crate::metrics::RequestTimer,
}

impl Drop for PendingPolls<'_> {
    fn drop(&mut self) {
        for poll in self.polls.drain(..) {
            poll.abandon(self.closing);
        }
    }
}

/// For a request whose working or output memory reservation failed, the
/// `RateLimited` response at `sequence` when its route only reads, built
/// from a fresh control reservation; `None` for any other route, whose
/// connection closes unanswered because a write may have committed.
fn memory_failure_refusal(
    route: Option<&'static RouteSpec>,
    sequence: u64,
    pools: &Arc<super::memory::Pools>,
) -> Result<Option<Vec<u8>>> {
    if !route.is_some_and(|route| read_only_route(route.id)) {
        return Ok(None);
    }
    let memory = pools.request(None);
    Ok(Some(memory.sync(|| {
        encode_status_response_at(&RpcStatus::RateLimited, sequence)
    })?))
}

/// Routes that read without changing server state, so that a request
/// refused after its handler ran has no ambiguous outcome.
fn read_only_route(id: RouteId) -> bool {
    matches!(
        id,
        RouteId::KvStoreGetRoot
            | RouteId::KvStoreGet
            | RouteId::KvStoreGetNode
            | RouteId::KvStoreGetEncryptedChunk
            | RouteId::KvStoreGetDir
            | RouteId::KvStoreCacheCheck
            | RouteId::KvStoreList
            | RouteId::KvStoreUsage
            | RouteId::RegLoadUserChain
            | RouteId::UserLoadUserChain
            | RouteId::UserLoadGenericChain
            | RouteId::UserGetPukForRole
            | RouteId::TeamLoaderLoadTeamChain
            | RouteId::TeamLoaderLoadTeamMembershipChain
            | RouteId::TeamAdminLoadTeamRawInbox
            | RouteId::MerkleQueryLookup
            | RouteId::MerkleQueryGetHistoricalRoots
            | RouteId::MerkleQueryGetCurrentRoot
            | RouteId::MerkleQueryGetCurrentRootHash
            | RouteId::MerkleQueryCheckKeyExists
            | RouteId::MerkleQueryGetCurrentRootSigned
            | RouteId::MerkleQueryMLookup
            | RouteId::RealTimeRtGetChannel
            | RouteId::RealTimeRtListAllChannelsForTeam
            | RouteId::RealTimeRtGetThread
            | RouteId::RealTimeRtGetInboxVersion
            | RouteId::RealTimeRtGetChangedThreads
            | RouteId::RealTimeRtGetThreadRecents
    )
}

/// The KEX receives one source can hold at once: a quarter of the global
/// budget (8 of the default 32), and at least one. Pairing one go-foks
/// device holds up to three receives across both devices, so two pairings
/// behind one NAT address fit in the share.
fn kex_receive_source_share(capacity: usize) -> usize {
    (capacity / 4).max(1)
}

/// A source's slot of the per-source KEX receive share; `None` for an exempt
/// source.
type KexSourcePermit = Option<crate::rate_limit::KeyedPermit<crate::rate_limit::SourceKey>>;

/// Takes a slot of `source`'s share of the KEX receive budget, then a permit
/// of the global budget `polls`, or counts the rejection under
/// `KexReceivePolls`. The budget never queues, so receives cannot hold every
/// ordinary connection, and one source cannot hold all of it.
fn admit_kex_receive(
    metrics: &Arc<crate::ServerMetrics>,
    polls: &Arc<Semaphore>,
    sources: &Arc<crate::rate_limit::KeyedLimit<crate::rate_limit::SourceKey>>,
    source: Option<crate::rate_limit::SourceKey>,
) -> Option<(crate::metrics::kex::Poll, KexSourcePermit)> {
    let source = match source {
        Some(source) => {
            let Some(permit) = sources.try_acquire(source) else {
                metrics.kex.rejected();
                metrics.request_rate_limited(crate::AdmissionRejection::KexReceivePolls);
                return None;
            };
            Some(permit)
        }
        None => None,
    };
    let Ok(permit) = Arc::clone(polls).try_acquire_owned() else {
        metrics.kex.rejected();
        metrics.request_rate_limited(crate::AdmissionRejection::KexReceivePolls);
        return None;
    };
    Some((
        crate::metrics::kex::Poll::new(Arc::clone(metrics), permit),
        source,
    ))
}

pub(crate) use foks_hosted_transport::framing::read_frame_ingress;

/// One classified inbound frame.
#[derive(Debug)]
enum Inbound {
    Call(foks_rpc::DecodedCall),
    /// A `CANCEL` control message for the call with this sequence number.
    Cancel(u64),
    /// Any other control message.
    Control,
    /// A call whose argument could not be decoded within the working memory
    /// budget, with its sequence number.
    OverBudget(u64),
}

async fn read_call_body_async<R: AsyncRead + Unpin>(
    reader: &mut R,
    length: usize,
    maximum: usize,
    request_memory: Arc<Semaphore>,
    memory: Arc<super::memory::Pools>,
) -> foks_rpc::Result<(Inbound, Arc<super::memory::Request>)> {
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
    let memory = memory.request(Some(permit));
    memory.classify_input(length);
    let message = match memory.sync(|| foks_rpc::decode_message(&content)) {
        Ok(message) => message,
        // A shortfall of the shared budget is not a malformed frame. Its
        // sequence comes from the call's first two elements, which need no
        // allocation.
        Err(error) => {
            return match foks_rpc::call_sequence(&content) {
                Some(sequence) if memory.failed() => Ok((Inbound::OverBudget(sequence), memory)),
                _ => Err(error),
            };
        }
    };
    let message = match message {
        foks_rpc::InboundMessage::Call(call) => Inbound::Call(call),
        foks_rpc::InboundMessage::Control => match foks_rpc::cancelled_sequence(&content) {
            Some(sequence) => Inbound::Cancel(sequence),
            None => Inbound::Control,
        },
    };
    Ok((message, memory))
}

fn phase_deadline(
    deadline: crate::deadline::Deadline,
    maximum: std::time::Duration,
) -> Result<tokio::time::Instant> {
    foks_hosted_transport::framing::phase_deadline(deadline, maximum).map_err(Into::into)
}

pub(crate) async fn write_response_until<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    response: &[u8],
    deadline: crate::deadline::Deadline,
    maximum: std::time::Duration,
) -> Result<()> {
    foks_hosted_transport::framing::write_response_until(writer, response, deadline, maximum)
        .await
        .map_err(Into::into)
}

fn timeout_error(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::TimedOut, message)
}

impl ServerData {
    fn long_poll(
        &self,
        route: &'static RouteSpec,
        sequence: u64,
        request_memory: Arc<super::memory::Request>,
    ) -> LongPoll<'_> {
        LongPoll {
            metrics: &self.metrics,
            faults: self.session_faults.as_deref(),
            route,
            sequence,
            request_memory,
        }
    }

    /// A KEX receive long poll, its permit of the global KEX receive budget
    /// and its slot of the peer source's share. A Go-compatible receive can
    /// stay open for an hour, so it needs both. Without either the rejection
    /// is counted and the request memory is returned for the `RateLimited`
    /// response.
    #[allow(clippy::type_complexity)]
    fn kex_long_poll(
        &self,
        route: &'static RouteSpec,
        sequence: u64,
        request_memory: Arc<super::memory::Request>,
        peer_ip: std::net::IpAddr,
    ) -> std::result::Result<
        (LongPoll<'_>, crate::metrics::kex::Poll, KexSourcePermit),
        Arc<super::memory::Request>,
    > {
        let source = self.rate_limiter.config().limited_source(peer_ip);
        let Some((kex, source)) = admit_kex_receive(
            &self.metrics,
            &self.kex_polling,
            &self.kex_polling_sources,
            source,
        ) else {
            return Err(request_memory);
        };
        Ok((self.long_poll(route, sequence, request_memory), kex, source))
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

/// Binds the client certificate of an authenticated-listener request to its
/// active credential, or returns the status that answers the request.
fn authenticate_request(
    data: &ServerData,
    certificate: Option<Vec<u8>>,
) -> std::result::Result<Principal, RpcStatus> {
    let certificate = CertificateDer::from(certificate.ok_or(RpcStatus::AuthError)?);
    let now = data
        .clock
        .now_micros()
        .map_err(|error| crate::status::unmapped(error.into(), "authenticate"))?;
    let database = data.read_database()?;
    Principal::authenticate(&certificate, &database, now).map_err(authentication_status)
}

/// The status for a failed [`Principal::authenticate`]. A certificate that is
/// not bound to an active credential, or whose key or credential kind does
/// not match, is go-foks `AUTH_ERROR` (1006). A storage fault while looking up
/// the binding is not a verdict on the certificate: it takes the status that
/// every route gives that fault, and a server fault is counted.
fn authentication_status(error: crate::Error) -> RpcStatus {
    match error {
        error @ crate::Error::Database(_) => crate::status::unmapped(error, "authenticate"),
        _ => RpcStatus::AuthError,
    }
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
        let pools = super::super::memory::pools(crate::SessionLimits::default()).unwrap();
        LongPoll {
            metrics,
            faults: Some(faults),
            route: route_spec(id),
            sequence: 7,
            request_memory: pools.request(Some(Arc::clone(memory).try_acquire_owned().unwrap())),
        }
    }

    #[test]
    fn authentication_failures_are_auth_errors_and_storage_faults_are_retryable() {
        for error in [
            crate::Error::Config(
                "authenticated peer certificate is not bound to an active credential",
            ),
            crate::Error::Config("authenticated peer certificate public key does not match"),
        ] {
            assert_eq!(authentication_status(error), RpcStatus::AuthError);
        }
        let before = crate::status::contract::failures_counted();
        assert_eq!(
            authentication_status(crate::status::contract::sqlite(rusqlite::ffi::SQLITE_BUSY)),
            RpcStatus::TransactionRetry
        );
        assert_eq!(
            authentication_status(crate::status::contract::sqlite(rusqlite::ffi::SQLITE_IOERR)),
            RpcStatus::TransactionRetry
        );
        assert_eq!(crate::status::contract::failures_counted(), before + 2);
        assert_eq!(
            authentication_status(crate::status::contract::database_full()),
            RpcStatus::QuotaExceeded
        );
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
            let polled = std::cell::Cell::new(false);
            let outcome = long_poll(&metrics, &faults, &memory, id)
                .run(async {
                    polled.set(true);
                    Ok(Vec::new())
                })
                .await
                .unwrap();
            assert!(outcome.disconnect_before_response, "{id:?}");
            assert_eq!(outcome.route.map(|route| route.id), Some(id));
            assert!(!polled.get(), "{id:?}");
            assert_eq!(faults.hits(), 1, "{id:?}");
            assert_eq!(metrics.snapshot().handler_duration_observations, 1);
            // The outcome holds the request's memory until it is dropped.
            assert_eq!(memory.available_permits(), 0);
            drop(outcome);
            assert_eq!(memory.available_permits(), 1);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn long_poll_results_are_encoded_and_timed() {
        let metrics = Arc::new(crate::ServerMetrics::default());
        let faults = crate::SessionFaults::default();
        let memory = Arc::new(Semaphore::new(1));

        let response = foks_rpc::encode_success_response_at(&[0x91, 0x01], 7).unwrap();
        let outcome = long_poll(
            &metrics,
            &faults,
            &memory,
            RouteId::RegPollOAuth2SessionCompletion,
        )
        .run(async { Ok(response.clone()) })
        .await
        .unwrap();
        assert!(!outcome.disconnect_before_response);
        assert_eq!(outcome.response, response);
        assert_eq!(outcome.status, STATUS_OK);
        drop(outcome);

        let outcome = long_poll(&metrics, &faults, &memory, RouteId::RealTimeRtPollInbox)
            .run(async { Err(RpcStatus::RateLimited) })
            .await
            .unwrap();
        assert_eq!(
            outcome.response,
            encode_status_response_at(&RpcStatus::RateLimited, 7).unwrap()
        );
        assert_eq!(outcome.status, RpcStatus::RateLimited.code());
        drop(outcome);
        assert_eq!(metrics.snapshot().handler_duration_observations, 2);
        assert_eq!(faults.hits(), 0);
        assert_eq!(memory.available_permits(), 1);
    }

    /// A pending KEX receive whose handler completes when `release` fires.
    fn pending_kex_receive<'a>(
        metrics: &'a Arc<crate::ServerMetrics>,
        faults: &'a crate::SessionFaults,
        memory: &Arc<Semaphore>,
        budget: &Arc<Semaphore>,
        sequence: u64,
        release: tokio::sync::oneshot::Receiver<std::result::Result<Vec<u8>, kex::ReceiveError>>,
    ) -> PendingPoll<'a> {
        let mut poll = long_poll(metrics, faults, memory, RouteId::KexReceive);
        poll.sequence = sequence;
        PendingPoll {
            sequence,
            family: "Kex",
            kex: Some(
                admit_kex_receive(
                    metrics,
                    budget,
                    &Arc::new(crate::rate_limit::KeyedLimit::new(1)),
                    None,
                )
                .unwrap()
                .0,
            ),
            kex_source: None,
            _request_timer: crate::ServerMetrics::request_timer(Arc::clone(metrics)),
            future: Box::pin(async move {
                let mut completion = None;
                let handler = async {
                    match release.await.unwrap() {
                        Ok(response) => {
                            completion = Some(crate::metrics::kex::Completion::Delivered);
                            Ok(response)
                        }
                        Err(error) => {
                            completion = Some(error.completion);
                            Err(error.status)
                        }
                    }
                };
                let outcome = poll.run(handler).await?;
                Ok(PolledOutcome {
                    outcome,
                    kex_completion: completion,
                })
            }),
        }
    }

    #[test]
    fn a_memory_failure_is_refused_on_read_routes_and_closes_mutations() {
        let pools = super::super::memory::pools(crate::SessionLimits::default()).unwrap();
        let refusal = memory_failure_refusal(Some(route_spec(RouteId::KvStoreList)), 9, &pools)
            .unwrap()
            .unwrap();
        assert!(matches!(
            foks_rpc::read_response(&mut refusal.as_slice(), 4096, 9),
            Err(foks_rpc::Error::RemoteStatus { code: 1012, .. })
        ));
        for route in [Some(route_spec(RouteId::KvStorePut)), None] {
            assert!(memory_failure_refusal(route, 9, &pools).unwrap().is_none());
        }
        for id in [
            RouteId::KvStorePut,
            RouteId::KvStoreFileUploadChunk,
            RouteId::KexReceive,
            RouteId::RealTimeRtSend,
            RouteId::RealTimeRtPollInbox,
            RouteId::UserRevokeDevice,
        ] {
            assert!(!read_only_route(id), "{id:?}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_call_that_cannot_be_decoded_within_the_working_budget_is_over_budget() {
        let pools = super::super::memory::pools(crate::SessionLimits::default()).unwrap();
        // Another request holds every byte of the working pool.
        let holder = pools.request(None);
        let working = foks_hosted_transport::memory::test_support::snapshot(&pools).working;
        assert!(foks_snowpack::AllocationBudget::reserve(&*holder, working));
        let argument = foks_snowpack::encode(&Value::Binary(vec![0x41; 8192])).unwrap();
        let request = foks_rpc::encode_call(17, 23, &argument, 41).unwrap();
        let (mut sender, mut reader) = tokio::io::duplex(request.len());
        sender.write_all(&request).await.unwrap();
        let length = read_frame_ingress(
            &mut reader,
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            std::time::Duration::from_secs(15),
        )
        .await
        .unwrap()
        .0;
        let (message, _memory) = read_call_body_async(
            &mut reader,
            length,
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            Arc::new(Semaphore::new(length * 2)),
            Arc::clone(&pools),
        )
        .await
        .unwrap();
        assert!(matches!(message, Inbound::OverBudget(41)), "{message:?}");
        assert_eq!(foks_rpc::call_sequence(&[0x95, 0x05, 0x2a]), Some(42));
        assert_eq!(foks_rpc::call_sequence(&[0x94, 0x01, 0x2a]), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn accepted_connections_send_keepalive_probes_as_go_listeners_do() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(address).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        enable_keepalive(&accepted).unwrap();
        let socket = socket2::SockRef::from(&accepted);
        assert!(socket.keepalive().unwrap());
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
        {
            assert_eq!(
                socket.tcp_keepalive_time().unwrap(),
                std::time::Duration::from_secs(15)
            );
            assert_eq!(
                socket.tcp_keepalive_interval().unwrap(),
                std::time::Duration::from_secs(15)
            );
            assert_eq!(socket.tcp_keepalive_retries().unwrap(), 9);
        }
    }

    #[test]
    fn one_source_holds_at_most_its_share_of_kex_receives() {
        let metrics = Arc::new(crate::ServerMetrics::default());
        let capacity = 8;
        let budget = Arc::new(Semaphore::new(capacity));
        let share = kex_receive_source_share(capacity);
        assert_eq!(share, 2);
        assert_eq!(kex_receive_source_share(32), 8);
        assert_eq!(kex_receive_source_share(2), 1);
        let sources = Arc::new(crate::rate_limit::KeyedLimit::new(share));
        let config = crate::RateLimitConfig::default();
        let first = config.limited_source("192.0.2.1".parse().unwrap());
        let second = config.limited_source("2001:db8::1".parse().unwrap());
        let admit = |source| admit_kex_receive(&metrics, &budget, &sources, source);
        let held = [admit(first).unwrap(), admit(first).unwrap()];
        // The first source is at its share; another source and exempt peers
        // are still admitted.
        assert!(admit(first).is_none());
        let other = admit(second).unwrap();
        assert!(config
            .limited_source("127.0.0.1".parse().unwrap())
            .is_none());
        let exempt = [
            admit(None).unwrap(),
            admit(None).unwrap(),
            admit(None).unwrap(),
        ];
        assert_eq!(budget.available_permits(), capacity - 6);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.kex.rejected, 1);
        assert_eq!(
            snapshot.admission_rejections[crate::AdmissionRejection::KexReceivePolls as usize],
            1
        );
        // A refusal by the source share takes no global permit, and a
        // released receive returns the source's slot.
        drop(held);
        assert!(admit(first).is_some());
        drop((other, exempt));
        assert_eq!(budget.available_permits(), capacity);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_polls_complete_in_completion_order_and_record_why_they_end() {
        use crate::metrics::kex::Completion;
        let metrics = Arc::new(crate::ServerMetrics::default());
        let faults = crate::SessionFaults::default();
        let memory = Arc::new(Semaphore::new(8));
        let budget = Arc::new(Semaphore::new(4));
        let mut polls = PendingPolls::default();
        let (first_release, first) = tokio::sync::oneshot::channel();
        let (second_release, second) = tokio::sync::oneshot::channel();
        let (_third_release, third) = tokio::sync::oneshot::channel();
        polls.push(pending_kex_receive(
            &metrics, &faults, &memory, &budget, 0, first,
        ));
        polls.push(pending_kex_receive(
            &metrics, &faults, &memory, &budget, 1, second,
        ));
        polls.push(pending_kex_receive(
            &metrics, &faults, &memory, &budget, 2, third,
        ));
        assert_eq!(budget.available_permits(), 1);

        // The later call completes first and is answered first, at its own
        // sequence.
        second_release.send(Ok(b"second".to_vec())).unwrap();
        let completed = polls.next_completed().await.unwrap();
        let mut outcome = completed.outcome;
        assert_eq!(completed.sequence, 1);
        assert_eq!(
            (outcome.response.as_slice(), completed.family),
            (&b"second"[..], "Kex")
        );
        // The request is still being timed until its response is written.
        assert_eq!(metrics.snapshot().request_duration_observations, 0);
        drop(completed.request_timer);
        assert_eq!(metrics.snapshot().request_duration_observations, 1);
        assert_eq!(polls.len(), 2);
        // A KEX receive keeps its budget permit until its response is written.
        assert_eq!(budget.available_permits(), 1);
        outcome.kex.take().unwrap().written();
        assert_eq!(budget.available_permits(), 2);
        assert_eq!(metrics.snapshot().kex.delivered, 1);
        drop(outcome);

        // A handler error is recorded only for a written response.
        first_release
            .send(Err(kex::ReceiveError {
                status: RpcStatus::RateLimited,
                completion: Completion::TimedOut,
            }))
            .unwrap();
        let mut outcome = polls.next_completed().await.unwrap().outcome;
        assert_eq!(
            outcome.response,
            encode_status_response_at(&RpcStatus::RateLimited, 0).unwrap()
        );
        drop(outcome.kex.take());
        drop(outcome);
        assert_eq!(metrics.snapshot().kex.failed, 1);

        // A cancelled receive is dropped unanswered and recorded as
        // cancelled; a cancel for an unknown sequence changes nothing.
        assert!(!polls.cancel(7));
        assert!(polls.cancel(2));
        assert!(polls.is_empty());
        let snapshot = metrics.snapshot();
        assert_eq!((snapshot.kex.cancelled, snapshot.kex.active), (1, 0));
        assert_eq!(budget.available_permits(), 4);

        // Receives still pending when the connection ends are disconnected,
        // or shut down when the server stops.
        let (_release, pending) = tokio::sync::oneshot::channel();
        polls.push(pending_kex_receive(
            &metrics, &faults, &memory, &budget, 3, pending,
        ));
        drop(polls);
        let mut polls = PendingPolls::default();
        let (_release, pending) = tokio::sync::oneshot::channel();
        polls.push(pending_kex_receive(
            &metrics, &faults, &memory, &budget, 4, pending,
        ));
        polls.closing = Completion::Shutdown;
        drop(polls);
        let snapshot = metrics.snapshot();
        assert_eq!((snapshot.kex.disconnected, snapshot.kex.shutdown), (1, 1));
        assert_eq!((snapshot.kex.active, snapshot.kex.admitted), (0, 5));
        assert_eq!(budget.available_permits(), 4);
        assert_eq!(memory.available_permits(), 8);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_response_write_gets_the_io_allowance_after_the_request_deadline() {
        let io_timeout = std::time::Duration::from_secs(2);
        let request =
            crate::deadline::Deadline::after(std::time::Duration::from_millis(1)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let response = vec![0x5a; 64 * 1024];
        let (mut writer, _reader) = tokio::io::duplex(1024);
        // The request deadline has passed; a write bounded by it fails.
        assert!(
            write_response_until(&mut writer, &response, request, io_timeout)
                .await
                .is_err()
        );
        // A slow reader still receives the whole response within the fresh
        // allowance that completed requests get.
        let (mut writer, mut slow) = tokio::io::duplex(1024);
        let forward = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let mut received = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut slow, &mut received)
                .await
                .unwrap();
            received.len()
        });
        write_response_until(
            &mut writer,
            &response,
            response_write_deadline(io_timeout).unwrap(),
            io_timeout,
        )
        .await
        .unwrap();
        drop(writer);
        assert_eq!(forward.await.unwrap(), response.len());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn frame_budget_is_not_renewed_for_body_execution_or_transmission() {
        let (mut peer, mut input) = tokio::io::duplex(16);
        peer.write_all(&[0xcd, 0x01, 0x00]).await.unwrap();
        let (length, deadline) =
            read_frame_ingress(&mut input, 1024, std::time::Duration::from_millis(20))
                .await
                .unwrap();
        assert_eq!(length, 256);
        // Keep the peer alive but withhold the body. Body time consumes the
        // original frame budget, leaving no fresh allowance for dispatch/output.
        let body = tokio::time::timeout_at(
            phase_deadline(deadline, std::time::Duration::from_secs(15)).unwrap(),
            read_call_body_async(
                &mut input,
                length,
                1024,
                Arc::new(Semaphore::new(4096)),
                super::super::memory::pools(crate::SessionLimits::default()).unwrap(),
            ),
        )
        .await;
        assert!(body.is_err());
        let (_stop, mut stop) = watch::channel(false);
        assert!(matches!(
            execute_bounded(Arc::new(Semaphore::new(1)), deadline, &mut stop, || panic!(
                "expired work started"
            ))
            .await
            .unwrap(),
            BoundedExecution::TimedOut
        ));
        let mut output = Vec::new();
        assert!(write_response_until(
            &mut output,
            b"late",
            deadline,
            std::time::Duration::from_secs(15)
        )
        .await
        .is_err());
        assert!(output.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timed_out_execution_remains_bounded_until_work_finishes() {
        let execution = Arc::new(Semaphore::new(1));
        let (_stop, mut receiver) = watch::channel(false);
        let (entered_sender, entered_receiver) = std::sync::mpsc::sync_channel(1);
        let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(1);

        let first = execute_bounded(
            Arc::clone(&execution),
            crate::deadline::Deadline::after(std::time::Duration::from_millis(20)).unwrap(),
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
                crate::deadline::Deadline::after(std::time::Duration::from_secs(1)).unwrap(),
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

        let first_length = read_frame_ingress(
            &mut reader_one,
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            std::time::Duration::from_secs(15),
        )
        .await
        .unwrap()
        .0;
        let first = read_call_body_async(
            &mut reader_one,
            first_length,
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            Arc::clone(&budget),
            super::super::memory::pools(crate::SessionLimits::default()).unwrap(),
        )
        .await
        .unwrap();
        match &first.0 {
            Inbound::Call(call) => assert_eq!(call.argument(), argument),
            other => panic!("expected a CALL_V2 request, not {other:?}"),
        }
        assert_eq!(budget.available_permits(), 0);

        let second_length = read_frame_ingress(
            &mut reader_two,
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            std::time::Duration::from_secs(15),
        )
        .await
        .unwrap()
        .0;
        let mut second = Box::pin(read_call_body_async(
            &mut reader_two,
            second_length,
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
            Arc::clone(&budget),
            super::super::memory::pools(crate::SessionLimits::default()).unwrap(),
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
            Inbound::Call(call) => assert_eq!(call.argument(), argument),
            other => panic!("expected a CALL_V2 request, not {other:?}"),
        }
    }
}
