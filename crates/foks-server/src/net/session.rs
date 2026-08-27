use std::sync::Arc;

use foks_proto::{
    DecodedSoftwareSignupArgument, EntityId, HistoricalMerkleRoots, MerkleRoot, ProbeResponse,
    SignedBlob, UsernameReservation,
};
use foks_rpc::{
    encode_status_response_at, encode_success_response_at, encode_void_success_response_at,
    RpcStatus,
};
use foks_snowpack::{decode, Value};
use rustls::pki_types::CertificateDer;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

use crate::auth::Principal;
use crate::identity::validate_software_signup;
use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::rpc::{route_call, Listener, RouteError, RoutedCall};
use crate::{Entropy, Result, SessionLimits, WriterHandle};

pub(crate) struct ServerData {
    probe_response: Arc<[u8]>,
    host_id: Vec<u8>,
    canonical_name: String,
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
}

impl ServerData {
    pub(crate) fn from_config(
        config: &crate::Config,
        rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    ) -> Result<Self> {
        let probe_response = Arc::clone(&config.probe_response);
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
                encode_success_response_at(&self.current_probe_response()?, sequence)
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
            ("Reg", "getUIDLookupChallege") => {
                let response = crate::services::registration::issue_uid_lookup_challenge(
                    call.call.argument(),
                    &self.host()?,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                    self.entropy.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "lookupUIDByDevice") => {
                let response = crate::services::registration::lookup_uid_by_device(
                    call.call.argument(),
                    &self.host()?,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "loadUserChain") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::user::load_user_chain(
                    &database,
                    &self.host()?,
                    call.call.argument(),
                    principal,
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "getPukForRole") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::user::puk_for_role(
                    &database,
                    call.call.argument(),
                    principal,
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "provisionDevice") => {
                let principal = principal.ok_or_else(permission_denied)?;
                self.user_mutation(call.call.argument(), principal, true)?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "revokeDevice") => {
                let principal = principal.ok_or_else(permission_denied)?;
                principal.require_ordinary_device()?;
                self.user_mutation(call.call.argument(), principal, false)?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "getHostConfig") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::user::host_config(&database, principal)?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamLoader", "getTeamVOBearerTokenChallenge") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_loader::issue_challenge(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                    self.entropy.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamLoader", "activateTeamVOBearerToken") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_loader::activate(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamLoader", "loadTeamChain") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_loader::load_chain(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.clock.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "reserveTeamname") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let response = crate::services::team_admin::reserve_name(
                    call.call.argument(),
                    principal,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                    self.entropy.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "createTeam" | "createTeamAdHoc") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                crate::services::team_admin::create(
                    call.call.argument(),
                    call.route.method == "createTeam",
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
                    &self.clock,
                    &self.hostchain_tail,
                )?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "editTeam") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_admin::edit(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
                    &self.clock,
                    &self.hostchain_tail,
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "makeInertTeamBearerToken") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_admin::make_inert_token(
                    call.call.argument(),
                    principal,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                    self.entropy.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "activateTeamBearerToken") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                crate::services::team_admin::activate_token(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                )?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "loadRemovalKeyBoxForTeamAdmin") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_admin::load_removal_box(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.clock.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("KvStore", method) => {
                let principal = principal.ok_or_else(permission_denied)?;
                principal.require_ordinary_device()?;
                let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
                let database = self.read_database()?;
                match crate::services::kv::dispatch(
                    method,
                    call.call.argument(),
                    principal,
                    &database,
                    writer,
                    &self.clock,
                )? {
                    crate::services::kv::Response::Data(response) => {
                        encode_success_response_at(&response, sequence)
                            .map_err(|_| RpcStatus::Unsupported)
                    }
                    crate::services::kv::Response::Void => {
                        encode_void_success_response_at(sequence)
                            .map_err(|_| RpcStatus::Unsupported)
                    }
                }
            }
            _ => Err(RpcStatus::Unsupported),
        }
    }

    fn user_mutation(
        &self,
        argument: &[u8],
        principal: &Principal,
        provision: bool,
    ) -> std::result::Result<(), RpcStatus> {
        const RECEIPT_LIFETIME_MICROSECONDS: u64 = 24 * 60 * 60 * 1_000_000;
        let decoded = if provision {
            crate::identity::mutation::Argument::Provision(
                foks_rpc::arguments::decode_provision_device(argument).map_err(bad_arguments)?,
            )
        } else {
            crate::identity::mutation::Argument::Revoke(
                foks_rpc::arguments::decode_revoke_device(argument).map_err(bad_arguments)?,
            )
        };
        let reader = self.read_database()?;
        let exact_link = decoded.link().encoded().map_err(bad_arguments)?;
        let idempotency_key =
            foks_crypto::prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &exact_link);
        let request_hash = foks_crypto::prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, argument);
        let receipt_now = self
            .clock
            .now_micros()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        match reader.request_receipt(&idempotency_key, &request_hash, receipt_now) {
            Ok(Some(receipt)) if receipt.response.is_empty() => return Ok(()),
            Ok(Some(_)) => return Err(RpcStatus::TransactionRetry),
            Ok(None) => {}
            Err(foks_server_db::Error::ReceiptConflict) => {
                return Err(bad_arguments("user mutation retry binding failed"));
            }
            Err(_) => return Err(RpcStatus::TransactionRetry),
        }
        let identity = reader
            .identity_by_active_device(principal.device_id())
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(permission_denied)?;
        let uid = identity.uid;
        let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
        let keys = Arc::clone(self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?);
        let clock = Arc::clone(&self.clock);
        let host = self.host().map_err(|_| RpcStatus::TransactionRetry)?;
        let hostchain_tail = self.hostchain_tail.clone();
        let signer = principal.device_id().to_vec();
        let result = writer.call(move |database| {
            let now = clock.now_micros()?;
            let receipt_expires_at = now
                .checked_add(RECEIPT_LIFETIME_MICROSECONDS)
                .ok_or(crate::Error::Signup("user mutation receipt expiry"))?;
            let authority = database
                .user_authority(&uid)?
                .ok_or(crate::Error::Signup("user mutation authority missing"))?;
            let command = crate::identity::mutation::validate(&authority, &host, &signer, decoded)?;
            let authoritative_root = database
                .current_root()?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            if authoritative_root.epoch != command.expected_root_epoch
                || authoritative_root.root_hash != command.expected_root_hash
            {
                return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
            }
            if now < decode_stored_root(&authoritative_root)?.time {
                return Err(crate::Error::Signup("system clock moved backwards"));
            }
            let chain_key = foks_merkle_store::chain_key(
                0,
                &EntityId::from_bytes(command.uid.clone())?,
                command.sequence,
                Some(&authority.next_tree_location),
            )?;
            let leaves = [(chain_key, command.link_hash)];
            let merkle_commit = foks_merkle_store::prepare(
                &database.node_reader(),
                authoritative_root.root_node,
                &[foks_merkle_store::LeafChange::Set {
                    key: chain_key,
                    value: command.link_hash,
                }],
            )?;
            let root_epoch = authoritative_root
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
            let root_blob = foks_snowpack::encode(&Value::Binary(exact_root.clone()))?;
            let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_typed(
                    merkle_key.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &root_blob,
                )?,
            }
            .encoded()?;
            let added = command.added.as_ref().map(|added| {
                let (role_type, visibility) = crate::identity::mutation::role_parts(added.role);
                foks_server_db::AddedCredential {
                    device_id: &added.device_id,
                    hepk_fingerprint: &added.hepk_fingerprint,
                    exact_hepk: &added.exact_hepk,
                    exact_name: &added.exact_name,
                    role_type,
                    visibility,
                    subkey_id: added.subkey_id.as_deref(),
                }
            });
            let shared_keys = command
                .shared_keys
                .iter()
                .map(|key| {
                    let (role_type, visibility) = crate::identity::mutation::role_parts(key.role);
                    foks_server_db::SharedKeyMutation {
                        role_type,
                        visibility,
                        generation: key.generation,
                        verify_key: &key.verify_key,
                        exact_hepk: &key.exact_hepk,
                    }
                })
                .collect::<Vec<_>>();
            let parcels = command
                .parcels
                .iter()
                .map(|parcel| {
                    let (role_type, visibility) =
                        crate::identity::mutation::role_parts(parcel.role);
                    foks_server_db::ParcelMutation {
                        device_id: &parcel.device_id,
                        sender_id: &command.signer,
                        role_type,
                        visibility,
                        generation: parcel.generation,
                        exact_parcel: &parcel.exact,
                    }
                })
                .collect::<Vec<_>>();
            // Keep exact seed-box bytes alive for the duration of the commit.
            let exact_seed_chain = command
                .seed_chain
                .iter()
                .map(foks_proto::SeedChainBox::encoded)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let seed_chain = command
                .seed_chain
                .iter()
                .zip(&exact_seed_chain)
                .map(|(boxed, exact)| {
                    let (role_type, visibility) = crate::identity::mutation::role_parts(boxed.role);
                    foks_server_db::SeedChainMutation {
                        role_type,
                        visibility,
                        generation: boxed.generation,
                        exact_box: exact,
                    }
                })
                .collect::<Vec<_>>();
            if command.link_hash != idempotency_key {
                return Err(crate::Error::Signup("user mutation link hash changed"));
            }
            database.commit_user_mutation(&foks_server_db::UserMutation {
                uid: &command.uid,
                signer_device_id: &command.signer,
                expected_sequence: command.sequence,
                expected_tail_hash: &command.expected_tail_hash,
                link_hash: &command.link_hash,
                exact_link: &command.exact_link,
                next_tree_location: &command.next_tree_location,
                added_credential: added,
                revoked_device_id: command.revoked.as_deref(),
                shared_keys: &shared_keys,
                parcels: &parcels,
                seed_chain: &seed_chain,
                expected_root_epoch: command.expected_root_epoch,
                expected_root_hash: &command.expected_root_hash,
                merkle_commit: &merkle_commit,
                merkle_leaves: &leaves,
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
            })?;
            Ok(())
        });
        match result {
            Ok(()) => Ok(()),
            Err(crate::Error::Database(foks_server_db::Error::StaleRoot)) => {
                Err(RpcStatus::StaleRoot)
            }
            Err(crate::Error::Database(foks_server_db::Error::QuotaExceeded)) => {
                Err(RpcStatus::QuotaExceeded)
            }
            Err(crate::Error::Database(foks_server_db::Error::ReceiptConflict)) => {
                Err(bad_arguments("user mutation retry binding failed"))
            }
            Err(crate::Error::WriterQueue) => Err(RpcStatus::RateLimited),
            Err(_) => Err(bad_arguments("user mutation validation failed")),
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
        let reader = self.read_database()?;
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
        let device = EntityId::from_bytes(device_id.clone()).map_err(bad_arguments)?;
        if !matches!(
            device.entity_type(),
            foks_proto::ENTITY_DEVICE | foks_proto::ENTITY_BACKUP_KEY | foks_proto::ENTITY_SUBKEY
        ) {
            return Err(bad_arguments("unsupported certificate credential kind"));
        }
        let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
        let keys = Arc::clone(self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?);
        let clock = Arc::clone(&self.clock);
        let entropy = Arc::clone(&self.entropy);
        let canonical_name = self.canonical_name.clone();
        let result = writer.call(move |database| {
            let Some(owner) =
                database.active_credential_owner(uid.as_bytes(), device.as_bytes())?
            else {
                return Ok(None);
            };
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
                &owner,
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

    fn read_database(&self) -> std::result::Result<crate::read_pool::ReadLease, RpcStatus> {
        let pool = self.read_database.as_ref().ok_or(RpcStatus::Unsupported)?;
        match pool.checkout() {
            Ok(database) => Ok(database),
            Err(crate::Error::ReaderPool) => {
                self.metrics.request_rate_limited();
                Err(RpcStatus::RateLimited)
            }
            Err(_) => Err(RpcStatus::TransactionRetry),
        }
    }

    fn host(&self) -> std::result::Result<EntityId, RpcStatus> {
        EntityId::from_bytes(self.host_id.clone()).map_err(bad_arguments)
    }

    fn current_root(&self) -> std::result::Result<Vec<u8>, RpcStatus> {
        if self.read_database.is_none() {
            return Ok(self.current_root.clone());
        }
        let database = self.read_database()?;
        let root = database
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        validated_root(&root)?;
        Ok(root.exact_root)
    }

    fn current_probe_response(&self) -> std::result::Result<Vec<u8>, RpcStatus> {
        if self.read_database.is_none() {
            return Ok(self.probe_response.to_vec());
        }
        let database = self.read_database()?;
        let root = database
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        validated_root(&root)?;
        let signed =
            SignedBlob::decode(&root.exact_signed_root).map_err(|_| RpcStatus::TransactionRetry)?;
        if signed.inner != root.exact_root {
            return Err(RpcStatus::TransactionRetry);
        }
        let mut probe =
            ProbeResponse::decode(&self.probe_response).map_err(|_| RpcStatus::TransactionRetry)?;
        probe.merkle_root = signed;
        probe.encoded().map_err(|_| RpcStatus::TransactionRetry)
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
        if self.read_database.is_none() {
            return Err(RpcStatus::Unsupported);
        }
        let database = self.read_database()?;
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

pub(crate) async fn serve(
    stream: tokio::net::TcpStream,
    peer_ip: std::net::IpAddr,
    listener: Listener,
    tls: &Arc<rustls::ServerConfig>,
    service_data: &Arc<ServerData>,
    limits: SessionLimits,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
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
                read_call_async(&mut stream, limits.maximum_frame_bytes),
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
        service_data.metrics.request_started();
        if !service_data.rate_limiter.allow_request(peer_ip) {
            service_data.metrics.request_rate_limited();
            let response = encode_status_response_at(&RpcStatus::RateLimited, call.sequence())?;
            write_response(&mut stream, &response, limits.io_timeout).await?;
            service_data.metrics.response_completed();
            return Ok(());
        }
        let sequence = call.sequence();
        let data = Arc::clone(service_data);
        let certificate = peer_certificate.clone();
        let outcome = tokio::task::spawn_blocking(move || -> Result<RequestOutcome> {
            let principal = if listener == Listener::Authenticated {
                let certificate = CertificateDer::from(certificate.ok_or(crate::Error::Config(
                    "authenticated TLS session has no client certificate",
                ))?);
                let now = data.clock.now_micros()?;
                let database = match data.read_database() {
                    Ok(database) => database,
                    Err(status) => {
                        return Ok(RequestOutcome {
                            response: encode_status_response_at(&status, sequence)?,
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
            let response = match route_call(call, listener) {
                Ok(call) => {
                    let protocol = call.route.protocol;
                    let method = call.route.method;
                    route = Some((protocol, method));
                    if data.should_disconnect(
                        crate::SessionFaultPoint::BeforeDurableMutation,
                        protocol,
                        method,
                    ) {
                        return Ok(RequestOutcome {
                            response: Vec::new(),
                            route,
                            disconnect_before_response: true,
                        });
                    }
                    match data.response(call, principal.as_ref()) {
                        Ok(response) => response,
                        Err(status) => encode_status_response_at(&status, sequence)?,
                    }
                }
                Err(RouteError::RequestTooLarge { .. }) => encode_status_response_at(
                    &RpcStatus::BadArguments("request exceeds the method limit".to_owned()),
                    sequence,
                )?,
                Err(RouteError::Unknown { .. } | RouteError::WrongListener) => {
                    encode_status_response_at(&RpcStatus::Unsupported, sequence)?
                }
            };
            Ok(RequestOutcome {
                response,
                route,
                disconnect_before_response: false,
            })
        })
        .await
        .map_err(|_| crate::Error::Thread)??;
        if outcome.disconnect_before_response {
            return Ok(());
        }
        let response = outcome.response;
        let route = outcome.route;
        if let Some((protocol, method)) = route {
            let point = if protocol == "KvStore" && method == "fileUploadChunk" {
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
        service_data.metrics.response_completed();
    }
    Ok(())
}

struct RequestOutcome {
    response: Vec<u8>,
    route: Option<(&'static str, &'static str)>,
    disconnect_before_response: bool,
}

async fn read_call_async<R: AsyncRead + Unpin>(
    reader: &mut R,
    maximum: usize,
) -> foks_rpc::Result<foks_rpc::DecodedCall> {
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
    let mut content = vec![0; length];
    reader.read_exact(&mut content).await?;
    foks_rpc::decode_call(&content)
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
