use std::io::Write as _;
use std::net::TcpStream;
use std::sync::Arc;

use foks_proto::{
    DecodedSoftwareSignupArgument, EntityId, HistoricalMerkleRoots, MerkleRoot, SignedBlob,
    UsernameReservation,
};
use foks_rpc::{
    encode_status_response_at, encode_success_response_at, encode_void_success_response_at,
    read_call, RpcStatus,
};
use foks_snowpack::{decode, Value};

use crate::auth::Principal;
use crate::identity::validate_software_signup;
use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::rpc::{route_call, Listener, RouteError, RoutedCall};
use crate::{Entropy, ReadDatabaseConfig, Result, SessionLimits, WriterHandle};

pub(crate) struct ServerData {
    probe_response: Arc<[u8]>,
    host_id: Vec<u8>,
    canonical_name: String,
    current_root: Vec<u8>,
    read_database: Option<ReadDatabaseConfig>,
    writer: Option<WriterHandle>,
    clock: Arc<dyn foks_server_db::Clock>,
    entropy: Arc<dyn Entropy>,
    key_provider: Option<Arc<dyn HostKeyProvider>>,
    hostchain_tail: foks_proto::HostchainTail,
}

impl ServerData {
    pub(crate) fn from_probe(
        probe_response: Arc<[u8]>,
        read_database: Option<ReadDatabaseConfig>,
        writer: Option<WriterHandle>,
        clock: Arc<dyn foks_server_db::Clock>,
        entropy: Arc<dyn Entropy>,
        key_provider: Option<Arc<dyn HostKeyProvider>>,
    ) -> Result<Self> {
        let probe = foks_proto::ProbeResponse::decode(&probe_response)?;
        let first = probe
            .hostchain
            .first()
            .ok_or(crate::Error::Config("empty bootstrap hostchain"))?;
        let host_id = first.decode_change()?.host.into_bytes();
        let zone = foks_proto::PublicZone::decode(&probe.public_zone.inner)?;
        let root = MerkleRoot::decode(&probe.merkle_root.inner)?;
        let canonical_name = endpoint_host(&zone.services.probe)
            .ok_or(crate::Error::Config("invalid bootstrap probe endpoint"))?
            .to_owned();
        Ok(Self {
            probe_response,
            host_id,
            canonical_name,
            current_root: probe.merkle_root.inner,
            read_database,
            writer,
            clock,
            entropy,
            key_provider,
            hostchain_tail: root.hostchain,
        })
    }

    fn response(
        &self,
        call: RoutedCall,
        principal: Option<&Principal>,
    ) -> std::result::Result<Vec<u8>, RpcStatus> {
        let sequence = call.call.sequence();
        match (call.route.protocol, call.route.method) {
            ("Probe", "probe") => {
                self.validate_probe(call.call.argument())?;
                encode_success_response_at(&self.probe_response, sequence)
                    .map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg" | "MerkleQuery" | "KvStore", "selectVHost") => {
                self.validate_host_argument(call.call.argument())?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("MerkleQuery", "getCurrentRoot") => {
                self.validate_host_argument(call.call.argument())?;
                let root = self.current_root()?;
                encode_success_response_at(&root, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("MerkleQuery", "getHistoricalRoots") => {
                let response = self.historical_roots(call.call.argument())?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "reserveUsername") => {
                let response = self.reserve_username(call.call.argument())?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "signup") => {
                self.signup(call.call.argument())?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "getClientCertChain") => {
                let response = self.client_certificate_chain(call.call.argument())?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "loadUserChain") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let response = crate::services::user::load_user_chain(
                    &self.read_database()?,
                    &self.host()?,
                    call.call.argument(),
                    principal,
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "getPukForRole") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let response = crate::services::user::puk_for_role(
                    &self.read_database()?,
                    call.call.argument(),
                    principal,
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            _ => Err(RpcStatus::Unsupported),
        }
    }

    fn reserve_username(&self, argument: &[u8]) -> std::result::Result<Vec<u8>, RpcStatus> {
        const RESERVATION_LIFETIME_MICROSECONDS: u64 = 10 * 60 * 1_000_000;
        let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
            return Err(bad_arguments(
                "username reservation argument is not a struct",
            ));
        };
        let [Value::Text(name)] = fields.as_slice() else {
            return Err(bad_arguments(
                "username reservation argument has the wrong shape",
            ));
        };
        let normalized = foks_verify::normalize_username(name)
            .filter(|normalized| normalized == name)
            .ok_or_else(|| bad_arguments("username is not normalized"))?;
        let mut token = [0; 17];
        self.entropy
            .fill(&mut token)
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let now = self
            .clock
            .now_micros()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let expires_at = now
            .checked_add(RESERVATION_LIFETIME_MICROSECONDS)
            .ok_or(RpcStatus::TransactionRetry)?;
        let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
        let result = writer.call(move |database| {
            database.reserve_name(&normalized, &token, 1, now, expires_at)?;
            Ok(())
        });
        match result {
            Ok(()) => UsernameReservation {
                token,
                sequence: 1,
                expires_at,
            }
            .encoded()
            .map_err(|_| RpcStatus::TransactionRetry),
            Err(crate::Error::Database(foks_server_db::Error::NameInUse)) => {
                Err(RpcStatus::NameInUse)
            }
            Err(_) => Err(RpcStatus::TransactionRetry),
        }
    }

    fn signup(&self, argument: &[u8]) -> std::result::Result<(), RpcStatus> {
        const RECEIPT_LIFETIME_MICROSECONDS: u64 = 24 * 60 * 60 * 1_000_000;
        const SIGNUP_REQUEST_HASH_TYPE_ID: u64 = 0x8f4b_8ab7_464f_4b53;

        let request = DecodedSoftwareSignupArgument::decode(argument).map_err(bad_arguments)?;
        let idempotency_key = request.self_token;
        let request_hash = foks_crypto::prefixed_hash(SIGNUP_REQUEST_HASH_TYPE_ID, argument);
        let database_config = self.read_database.as_ref().ok_or(RpcStatus::Unsupported)?;
        let reader = foks_server_db::ReadDatabase::open(
            &database_config.path,
            database_config.database.clone(),
        )
        .map_err(|_| RpcStatus::TransactionRetry)?;
        let receipt_now = self
            .clock
            .now_micros()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        match reader.request_receipt(&idempotency_key, &request_hash, receipt_now) {
            Ok(Some(receipt)) if receipt.response.is_empty() => return Ok(()),
            Ok(Some(_)) => return Err(RpcStatus::TransactionRetry),
            Ok(None) => {}
            Err(foks_server_db::Error::ReceiptConflict) => {
                return Err(bad_arguments("signup retry binding failed"));
            }
            Err(_) => return Err(RpcStatus::TransactionRetry),
        }
        let current = reader
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        let current_root = validated_root(&current)?;
        let host = EntityId::from_bytes(self.host_id.clone()).map_err(bad_arguments)?;
        let validated = validate_software_signup(&request, &host, &current_root, current.root_hash)
            .map_err(|_| bad_arguments("software signup validation failed"))?;
        let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
        let keys = self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?;
        let keys = Arc::clone(keys);
        let clock = Arc::clone(&self.clock);
        let hostchain_tail = self.hostchain_tail.clone();
        let reservation = request.reservation;
        let expected_root_hash = current.root_hash;
        let result = writer.call(move |database| {
            let now = clock.now_micros()?;
            let receipt_expires_at = now
                .checked_add(RECEIPT_LIFETIME_MICROSECONDS)
                .ok_or(crate::Error::Signup("receipt expiry overflow"))?;
            let authoritative = database
                .current_root()?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            if authoritative.root_hash != expected_root_hash {
                return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
            }
            let authoritative_root = decode_stored_root(&authoritative)?;
            if now < authoritative_root.time {
                return Err(crate::Error::Signup("system clock moved backwards"));
            }
            let changes = validated
                .leaves
                .iter()
                .map(|(key, value)| foks_merkle_store::LeafChange::Set {
                    key: *key,
                    value: *value,
                })
                .collect::<Vec<_>>();
            let merkle_commit = foks_merkle_store::prepare(
                &database.node_reader(),
                authoritative.root_node,
                &changes,
            )?;
            let root_epoch = authoritative
                .epoch
                .checked_add(1)
                .ok_or(crate::Error::Signup("Merkle epoch overflow"))?;
            let pointer_epochs = foks_merkle_store::back_pointer_sequence(root_epoch);
            let pointer_roots = database
                .roots_at(&pointer_epochs)?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            for root in &pointer_roots {
                decode_stored_root(root)?;
            }
            let back_pointers = pointer_roots
                .into_iter()
                .map(|root| (root.epoch, root.root_hash))
                .collect::<Vec<_>>();
            let root = MerkleRoot {
                epoch: root_epoch,
                time: now,
                back_pointers: foks_merkle_store::back_pointer_hash(&back_pointers)?,
                root_node: merkle_commit.root,
                hostchain: hostchain_tail,
            };
            let exact_root = root.encoded()?;
            let root_hash =
                foks_crypto::prefixed_hash(foks_proto::MERKLE_ROOT_TYPE_ID, &exact_root);
            let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
            let root_blob = foks_snowpack::encode(&Value::Binary(exact_root.clone()))?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_typed(
                    merkle_key.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &root_blob,
                )?,
            }
            .encoded()?;
            let mutation = foks_server_db::IdentityMutation {
                normalized_name: &validated.normalized_name,
                reservation_token: &reservation.token,
                reservation_sequence: reservation.sequence,
                reservation_expires_at: reservation.expires_at,
                username_utf8: &validated.username_utf8,
                username_commitment_key: &validated.username_commitment_key,
                uid: validated.uid.as_bytes(),
                device_id: validated.device_id.as_bytes(),
                device_hepk_fingerprint: &validated.device_hepk_fingerprint,
                exact_device_hepk: &validated.exact_device_hepk,
                exact_device_name: &validated.exact_device_name,
                link_hash: &validated.link_hash,
                exact_link: &validated.exact_link,
                tree_location: &validated.next_tree_location,
                shared_role_type: foks_proto::Role::OWNER.protocol_value(),
                shared_visibility: 0,
                shared_generation: 1,
                shared_verify_key: validated.puk_verify_key.as_bytes(),
                exact_shared_hepk: &validated.exact_puk_hepk,
                exact_parcel: &validated.exact_parcel,
                expected_root_hash: Some(expected_root_hash),
                merkle_commit: &merkle_commit,
                merkle_leaves: &validated.leaves,
                root_epoch,
                root_hash: &root_hash,
                exact_root: &exact_root,
                exact_signed_root: &exact_signed_root,
                back_pointers: &back_pointers,
                idempotency_key: &idempotency_key,
                request_hash: &request_hash,
                response: &[],
                now,
                receipt_expires_at,
            };
            database.commit_identity(&mutation)?;
            Ok(())
        });
        match result {
            Ok(()) => Ok(()),
            Err(crate::Error::Database(foks_server_db::Error::NameInUse)) => {
                Err(RpcStatus::NameInUse)
            }
            Err(crate::Error::Database(foks_server_db::Error::StaleRoot)) => {
                Err(RpcStatus::StaleRoot)
            }
            Err(crate::Error::Database(
                foks_server_db::Error::Reservation | foks_server_db::Error::ReceiptConflict,
            )) => Err(bad_arguments("signup reservation or retry binding failed")),
            Err(crate::Error::WriterQueue) => Err(RpcStatus::RateLimited),
            Err(_) => Err(RpcStatus::TransactionRetry),
        }
    }

    fn client_certificate_chain(&self, argument: &[u8]) -> std::result::Result<Vec<u8>, RpcStatus> {
        const CERTIFICATE_LIFETIME_MICROSECONDS: u64 = 7 * 24 * 60 * 60 * 1_000_000;
        const RENEWAL_WINDOW_MICROSECONDS: u64 = 24 * 60 * 60 * 1_000_000;
        const CLOCK_SKEW_MICROSECONDS: u64 = 5 * 60 * 1_000_000;

        let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
            return Err(bad_arguments("certificate argument is not a struct"));
        };
        let [Value::Binary(uid), Value::Binary(device_id)] = fields.as_slice() else {
            return Err(bad_arguments("certificate argument has the wrong shape"));
        };
        let uid = EntityId::from_bytes(uid.clone())
            .and_then(|uid| uid.require_type(foks_proto::ENTITY_USER))
            .map_err(bad_arguments)?;
        let device = EntityId::from_bytes(device_id.clone())
            .and_then(|device| device.require_type(foks_proto::ENTITY_DEVICE))
            .map_err(bad_arguments)?;
        let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
        let keys = Arc::clone(self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?);
        let clock = Arc::clone(&self.clock);
        let entropy = Arc::clone(&self.entropy);
        let canonical_name = self.canonical_name.clone();
        let result = writer.call(move |database| {
            if !database.is_active_device(uid.as_bytes(), device.as_bytes())? {
                return Ok(None);
            }
            let now = clock.now_micros()?;
            if let Some(certificate) =
                database.certificate_for_device(uid.as_bytes(), device.as_bytes())?
            {
                let renewal_at = now
                    .checked_add(RENEWAL_WINDOW_MICROSECONDS)
                    .ok_or(crate::Error::Signup("certificate renewal overflow"))?;
                if certificate.not_before <= now && certificate.not_after > renewal_at {
                    return Ok(Some(certificate.exact_certificate));
                }
            }
            let not_before = now.saturating_sub(CLOCK_SKEW_MICROSECONDS);
            let not_after = now
                .checked_add(CERTIFICATE_LIFETIME_MICROSECONDS)
                .ok_or(crate::Error::Signup("certificate expiry overflow"))?;
            let mut serial = [0; 20];
            entropy.fill(&mut serial)?;
            serial[0] &= 0x7f;
            if serial.iter().all(|byte| *byte == 0) {
                serial[19] = 1;
            }
            let public_key: [u8; 32] = device.as_bytes()[1..]
                .try_into()
                .map_err(|_| crate::Error::Signup("device public key width"))?;
            let certificate = crate::pki::issue_bounded_device_certificate(
                keys.as_ref(),
                &canonical_name,
                public_key,
                &serial,
                not_before,
                not_after,
            )?;
            database.record_certificate(
                &serial,
                uid.as_bytes(),
                device.as_bytes(),
                not_before,
                not_after,
                &certificate,
            )?;
            Ok(Some(certificate))
        });
        match result {
            Ok(Some(certificate)) => {
                foks_snowpack::encode(&Value::Array(vec![Value::Binary(certificate)]))
                    .map_err(|_| RpcStatus::TransactionRetry)
            }
            Ok(None) => Err(RpcStatus::NotFound("active device not found".to_owned())),
            Err(crate::Error::WriterQueue) => Err(RpcStatus::RateLimited),
            Err(_) => Err(RpcStatus::TransactionRetry),
        }
    }

    fn read_database(&self) -> std::result::Result<foks_server_db::ReadDatabase, RpcStatus> {
        let config = self.read_database.as_ref().ok_or(RpcStatus::Unsupported)?;
        foks_server_db::ReadDatabase::open(&config.path, config.database.clone())
            .map_err(|_| RpcStatus::TransactionRetry)
    }

    fn host(&self) -> std::result::Result<EntityId, RpcStatus> {
        EntityId::from_bytes(self.host_id.clone()).map_err(bad_arguments)
    }

    fn current_root(&self) -> std::result::Result<Vec<u8>, RpcStatus> {
        let Some(config) = &self.read_database else {
            return Ok(self.current_root.clone());
        };
        let database = foks_server_db::ReadDatabase::open(&config.path, config.database.clone())
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let root = database
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        validated_root(&root)?;
        Ok(root.exact_root)
    }

    fn historical_roots(&self, argument: &[u8]) -> std::result::Result<Vec<u8>, RpcStatus> {
        let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
            return Err(bad_arguments("historical roots argument is not a struct"));
        };
        let [host, full_epochs, hash_epochs] = fields.as_slice() else {
            return Err(bad_arguments(
                "historical roots argument has the wrong shape",
            ));
        };
        self.validate_host_value(host)?;
        let full_epochs = decode_epochs(full_epochs)?;
        let hash_epochs = decode_epochs(hash_epochs)?;
        let Some(config) = &self.read_database else {
            return Err(RpcStatus::Unsupported);
        };
        let database = foks_server_db::ReadDatabase::open(&config.path, config.database.clone())
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let full = database
            .roots_at(&full_epochs)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        let hashes = database
            .roots_at(&hash_epochs)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        for root in &hashes {
            validated_root(root)?;
        }
        HistoricalMerkleRoots {
            roots: full
                .iter()
                .map(validated_root)
                .collect::<std::result::Result<Vec<_>, _>>()?,
            hashes: hashes.into_iter().map(|root| root.root_hash).collect(),
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    fn validate_host_argument(&self, argument: &[u8]) -> std::result::Result<(), RpcStatus> {
        let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
            return Err(bad_arguments("host argument is not a struct"));
        };
        let [host] = fields.as_slice() else {
            return Err(bad_arguments("host argument has the wrong shape"));
        };
        self.validate_host_value(host)
    }

    fn validate_host_value(&self, host: &Value) -> std::result::Result<(), RpcStatus> {
        let Value::Binary(host) = host else {
            return Err(bad_arguments("host argument is not binary"));
        };
        if host.len() != 33 || host.first() != Some(&foks_proto::ENTITY_HOST) {
            return Err(bad_arguments("host argument is not a HostID"));
        }
        if host != &self.host_id {
            return Err(RpcStatus::NotFound("host not found".to_owned()));
        }
        Ok(())
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

fn validated_root(
    root: &foks_server_db::RootSnapshot,
) -> std::result::Result<MerkleRoot, RpcStatus> {
    decode_stored_root(root).map_err(|_| RpcStatus::TransactionRetry)
}

fn decode_stored_root(root: &foks_server_db::RootSnapshot) -> Result<MerkleRoot> {
    let decoded = MerkleRoot::decode(&root.exact_root)?;
    let hash = foks_crypto::prefixed_hash(foks_proto::MERKLE_ROOT_TYPE_ID, &root.exact_root);
    if decoded.epoch != root.epoch || decoded.root_node != root.root_node || hash != root.root_hash
    {
        return Err(crate::Error::Signup("stored Merkle root binding mismatch"));
    }
    Ok(decoded)
}

fn decode_epochs(value: &Value) -> std::result::Result<Vec<u64>, RpcStatus> {
    let values = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(values) if values.len() <= 64 => values,
        Value::Array(_) => return Err(bad_arguments("too many historical root epochs")),
        _ => return Err(bad_arguments("historical root epochs are not a list")),
    };
    values
        .iter()
        .map(|value| match value {
            Value::Unsigned(epoch) => Ok(*epoch),
            _ => Err(bad_arguments("historical root epoch is not unsigned")),
        })
        .collect()
}

pub(crate) fn serve(
    stream: TcpStream,
    listener: Listener,
    tls: &Arc<rustls::ServerConfig>,
    service_data: &Arc<ServerData>,
    limits: SessionLimits,
) -> Result<()> {
    stream.set_read_timeout(Some(limits.io_timeout))?;
    stream.set_write_timeout(Some(limits.io_timeout))?;
    let connection = rustls::ServerConnection::new(Arc::clone(tls))?;
    let mut stream = rustls::StreamOwned::new(connection, stream);
    let mut principal = None;
    for _ in 0..limits.maximum_requests {
        let call = match read_call(&mut stream, limits.maximum_frame_bytes) {
            Ok(call) => call,
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
        if listener == Listener::Authenticated && principal.is_none() {
            let certificate = stream
                .conn
                .peer_certificates()
                .and_then(|certificates| certificates.first())
                .ok_or(crate::Error::Config(
                    "authenticated TLS session has no client certificate",
                ))?;
            principal = Some(Principal::from_certificate(certificate)?);
        }
        let sequence = call.sequence();
        let response = match route_call(call, listener) {
            Ok(call) => match service_data.response(call, principal.as_ref()) {
                Ok(response) => response,
                Err(status) => encode_status_response_at(&status, sequence)?,
            },
            Err(RouteError::RequestTooLarge { .. }) => encode_status_response_at(
                &RpcStatus::BadArguments("request exceeds the method limit".to_owned()),
                sequence,
            )?,
            Err(RouteError::Unknown { .. } | RouteError::WrongListener) => {
                encode_status_response_at(&RpcStatus::Unsupported, sequence)?
            }
        };
        stream.write_all(&response)?;
        stream.flush()?;
        while stream.conn.wants_write() {
            stream.conn.complete_io(&mut stream.sock)?;
        }
    }
    Ok(())
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
