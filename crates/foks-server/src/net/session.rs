use std::sync::Arc;

mod handlers;
mod kex;

use foks_proto::{
    DecodedSignupArgument, EntityId, HistoricalMerkleRoots, InviteCode, MerkleExistsResponse,
    MerkleLookupResponse, MerkleMultiLookupResponse, MerkleRoot, ProbeResponse, SignedBlob,
    TreeRoot, UsernameReservation,
};
use foks_rpc::{
    arguments::{
        decode_merkle_check_key, decode_merkle_lookup, decode_merkle_multi_lookup,
        decode_merkle_optional_host,
    },
    encode_status_response_at, RpcStatus,
};
use foks_snowpack::{decode, Value};
use rustls::pki_types::CertificateDer;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::auth::Principal;
use crate::identity::validate_signup;
use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::rpc::{route_call, Listener, RouteError};
use crate::{Entropy, Result, SessionLimits, WriterHandle};

pub(crate) struct ServerData {
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
    request_memory: Arc<Semaphore>,
    kex_relay: Arc<kex::Relay>,
}

pub(crate) struct OwnedPassphraseMutation {
    verify_key: Vec<u8>,
    salt: [u8; 16],
    generation: u64,
    exact_skmwk_box: Vec<u8>,
    exact_passphrase_box: Vec<u8>,
    exact_puk_box: Option<Vec<u8>>,
    puk_generation: Option<u64>,
    puk_role: Option<foks_proto::Role>,
    stretch_version: u64,
}

impl OwnedPassphraseMutation {
    pub(crate) fn from_argument(argument: &foks_proto::PassphraseUpdateArgument) -> Result<Self> {
        Ok(Self {
            verify_key: argument.verify_key.as_bytes().to_vec(),
            salt: argument.salt,
            generation: argument.generation,
            exact_skmwk_box: foks_snowpack::encode(&argument.skmwk_box.to_value())?,
            exact_passphrase_box: argument.passphrase_box.encoded()?,
            exact_puk_box: argument
                .puk_box
                .as_ref()
                .map(foks_proto::PpePukBox::encoded)
                .transpose()?,
            puk_generation: argument.puk_box.as_ref().map(|boxed| boxed.puk_generation),
            puk_role: argument.puk_box.as_ref().map(|boxed| boxed.puk_role),
            stretch_version: argument.stretch_version.protocol_value(),
        })
    }

    pub(crate) fn as_database(&self, now: u64) -> foks_server_db::PassphraseMutation<'_> {
        foks_server_db::PassphraseMutation {
            verify_key: &self.verify_key,
            salt: &self.salt,
            generation: self.generation,
            exact_skmwk_box: &self.exact_skmwk_box,
            exact_passphrase_box: &self.exact_passphrase_box,
            exact_puk_box: self.exact_puk_box.as_deref(),
            puk_generation: self.puk_generation,
            puk_role: self.puk_role,
            stretch_version: self.stretch_version,
            now,
        }
    }
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
        let probe_endpoint = zone.services.probe.clone();
        let canonical_name = endpoint_host(&probe_endpoint)
            .ok_or(crate::Error::Config("invalid bootstrap probe endpoint"))?
            .to_owned();
        Ok(Self {
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
            request_memory: Arc::new(Semaphore::new(config.limits.maximum_request_memory_bytes)),
            kex_relay: Arc::new(kex::Relay::default()),
        })
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
        let submitted_change = decoded
            .link()
            .decode_group_change()
            .map_err(bad_arguments)?;
        let signed_root = submitted_change.root.clone();
        let idempotency_key =
            foks_crypto::prefixed_hash_signable(foks_proto::LINK_OUTER_TYPE_ID, &exact_link)
                .map_err(bad_arguments)?;
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
        let signer = identity.device_id.clone();
        let result = writer.call(move |database| {
            let now = clock.now_micros()?;
            let receipt_expires_at = now
                .checked_add(RECEIPT_LIFETIME_MICROSECONDS)
                .ok_or(crate::Error::Signup("user mutation receipt expiry"))?;
            let authority = database
                .user_authority(&uid)?
                .ok_or(crate::Error::Signup("user mutation authority missing"))?;
            if mutation_cites_superseded_user_head(database, &authority, &host, &submitted_change)?
            {
                return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
            }
            let cited_root = require_cited_root(database, &signed_root)?;
            let command = crate::identity::mutation::validate(
                &authority,
                &host,
                &signer,
                decoded,
                signed_root,
            )?;
            let authoritative_root = database
                .current_root()?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            if cited_root.epoch > authoritative_root.epoch {
                return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
            }
            if now / 1_000 < decode_stored_root(&authoritative_root)?.time {
                return Err(crate::Error::Signup("system clock moved backwards"));
            }
            let chain_key = foks_merkle_store::chain_key(
                0,
                &EntityId::from_bytes(command.uid.clone())?,
                command.sequence,
                Some(&authority.next_tree_location),
            )?;
            let mut leaves = vec![(chain_key, command.link_hash)];
            let settings_location = command
                .user_settings
                .as_ref()
                .map(|settings| {
                    let state = database
                        .generic_chain(&command.uid, foks_proto::CHAIN_TYPE_USER_SETTINGS)?
                        .ok_or(crate::Error::Signup("user-settings subchain seed missing"))?;
                    let location = if settings.sequence == 1 {
                        foks_crypto::subchain_tree_location(
                            &state.location_seed,
                            foks_proto::CHAIN_TYPE_USER_SETTINGS,
                        )?
                    } else {
                        let index = usize::try_from(settings.sequence.saturating_sub(2))
                            .map_err(|_| crate::Error::Signup("settings sequence overflow"))?;
                        state
                            .links
                            .get(index)
                            .filter(|link| link.sequence.saturating_add(1) == settings.sequence)
                            .map(|link| link.next_tree_location)
                            .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?
                    };
                    let key = foks_merkle_store::chain_key(
                        foks_proto::CHAIN_TYPE_USER_SETTINGS,
                        &EntityId::from_bytes(command.uid.clone())?,
                        settings.sequence,
                        Some(&location),
                    )?;
                    leaves.push((key, settings.link_hash));
                    Ok::<_, crate::Error>(location)
                })
                .transpose()?;
            let merkle_commit = foks_merkle_store::prepare(
                &database.node_reader(),
                authoritative_root.root_node,
                &leaves
                    .iter()
                    .map(|(key, value)| foks_merkle_store::LeafChange::Set {
                        key: *key,
                        value: *value,
                    })
                    .collect::<Vec<_>>(),
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
                time: now / 1_000,
                back_pointers: foks_merkle_store::back_pointer_hash(root_epoch, &back_pointers)?,
                root_node: merkle_commit.root,
                hostchain: hostchain_tail,
                extensions: Vec::new(),
            };
            let exact_root = root.encoded()?;
            let root_hash =
                foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &exact_root)?;
            let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_blob(
                    merkle_key.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &exact_root,
                )?,
            }
            .encoded()?;
            let added = command.added.as_ref().map(|added| {
                let (role_type, visibility) = crate::identity::mutation::role_parts(added.role);
                foks_server_db::AddedCredential {
                    device_id: &added.device_id,
                    self_token: &added.self_token,
                    hepk_fingerprint: &added.hepk_fingerprint,
                    exact_hepk: &added.exact_hepk,
                    exact_name: &added.exact_name,
                    role_type,
                    visibility,
                    subkey_id: added.subkey_id.as_deref(),
                    exact_subkey_box: added.exact_subkey_box.as_deref(),
                    yubi_pq_hint: added.yubi_pq_hint.as_ref().map(|(slot, id)| (*slot, id)),
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
            let passphrase = command
                .passphrase
                .as_ref()
                .map(|argument| {
                    let current =
                        database
                            .passphrase(&command.uid)?
                            .ok_or(crate::Error::Database(
                                foks_server_db::Error::PassphraseNotFound,
                            ))?;
                    let mut owned = OwnedPassphraseMutation::from_argument(argument)?;
                    owned.salt = current.salt;
                    Ok::<_, crate::Error>(owned)
                })
                .transpose()?;
            let user_settings = command
                .user_settings
                .as_ref()
                .zip(settings_location.as_ref())
                .map(
                    |(settings, current_tree_location)| foks_server_db::GenericLinkMutation {
                        entity_id: &command.uid,
                        chain_type: foks_proto::CHAIN_TYPE_USER_SETTINGS,
                        signer_credential_id: &settings.signer,
                        sequence: settings.sequence,
                        previous: settings.previous.as_ref(),
                        link_root_epoch: settings.root.epoch,
                        link_root_hash: &settings.root.hash,
                        current_tree_location,
                        next_tree_location: &settings.next_tree_location,
                        link_hash: &settings.link_hash,
                        exact_link: &settings.exact_link,
                        passphrase_info: Some(foks_server_db::GenericPassphraseInfo {
                            generation: settings.info.generation,
                            salt: settings.info.salt.as_ref(),
                            stretch_version: settings.info.stretch_version,
                        }),
                    },
                );
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
                current_tree_location: &authority.next_tree_location,
                next_tree_location: &command.next_tree_location,
                added_credential: added,
                revoked_device_id: command.revoked.as_deref(),
                shared_keys: &shared_keys,
                parcels: &parcels,
                seed_chain: &seed_chain,
                passphrase: passphrase
                    .as_ref()
                    .map(|passphrase| passphrase.as_database(now)),
                user_settings,
                cited_root_epoch: cited_root.epoch,
                expected_root_epoch: authoritative_root.epoch,
                expected_root_hash: &authoritative_root.root_hash,
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
            Err(crate::Error::Database(foks_server_db::Error::StaleRoot)) => Err(
                RpcStatus::RevokeRace("user chain or Merkle root changed".to_owned()),
            ),
            Err(crate::Error::Database(foks_server_db::Error::QuotaExceeded)) => {
                Err(RpcStatus::QuotaExceeded)
            }
            Err(crate::Error::Database(foks_server_db::Error::ReceiptConflict)) => {
                Err(bad_arguments("user mutation retry binding failed"))
            }
            Err(crate::Error::WriterQueue) => Err(RpcStatus::RateLimited),
            Err(error) => Err(crate::error::merkle_mint_status(&error)
                .unwrap_or_else(|| bad_arguments("user mutation validation failed"))),
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
        // The reservation is replayed over the Go `lib.Time` wire type, whose
        // millisecond precision must round-trip exactly into the database.
        let expires_at_millis = (now / 1_000)
            .checked_add(RESERVATION_LIFETIME_MICROSECONDS / 1_000)
            .ok_or(RpcStatus::TransactionRetry)?;
        let expires_at = expires_at_millis
            .checked_mul(1_000)
            .ok_or(RpcStatus::TransactionRetry)?;
        let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
        let result = writer.call_with_current_time(
            Arc::clone(&self.clock),
            move |database, current_time| {
                database.reserve_name(&normalized, &token, 1, current_time, expires_at)?;
                Ok(())
            },
        );
        match result {
            Ok(()) => UsernameReservation {
                token,
                sequence: 1,
                expires_at: expires_at_millis,
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

        let request = DecodedSignupArgument::decode(argument).map_err(bad_arguments)?;
        let (invite_hash, invite_kind) = match &request.invite_code {
            InviteCode::Empty => (None, None),
            InviteCode::Standard(_) | InviteCode::MultiUse(_) => (
                Some(crate::services::registration::invite_fingerprint(
                    &request.invite_code,
                )?),
                Some(crate::services::registration::invite_kind(
                    &request.invite_code,
                )?),
            ),
            _ => return Err(RpcStatus::BadInvite),
        };
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
        let signed_root = request.link.decode_eldest().map_err(bad_arguments)?.root;
        let cited = reader
            .root_at(signed_root.epoch)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or(RpcStatus::StaleRoot)?;
        let cited_root = validated_root(&cited)?;
        if cited.root_hash != signed_root.hash {
            return Err(RpcStatus::StaleRoot);
        }
        let host = EntityId::from_bytes(self.host_id.clone()).map_err(bad_arguments)?;
        let validated = validate_signup(&request, &host, &cited_root, cited.root_hash)
            .map_err(|_| bad_arguments("software signup validation failed"))?;
        let passphrase = request
            .passphrase
            .as_ref()
            .map(OwnedPassphraseMutation::from_argument)
            .transpose()
            .map_err(|_| bad_arguments("invalid signup passphrase material"))?;
        let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
        let keys = self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?;
        let keys = Arc::clone(keys);
        let clock = Arc::clone(&self.clock);
        let hostchain_tail = self.hostchain_tail.clone();
        let reservation = request.reservation;
        // lib.Time is milliseconds on the Go wire, while the database stores
        // reservation deadlines in microseconds alongside its clock values.
        let reservation_expires_at = reservation
            .expires_at
            .checked_mul(1_000)
            .ok_or_else(|| bad_arguments("reservation expiry overflows server clock units"))?;
        let result = writer.call(move |database| {
            let now = clock.now_micros()?;
            let receipt_expires_at = now
                .checked_add(RECEIPT_LIFETIME_MICROSECONDS)
                .ok_or(crate::Error::Signup("receipt expiry overflow"))?;
            let authoritative = database
                .current_root()?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            let cited = require_cited_root(database, &signed_root)?;
            if cited.epoch > authoritative.epoch {
                return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
            }
            let authoritative_root = decode_stored_root(&authoritative)?;
            if now / 1_000 < authoritative_root.time {
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
                time: now / 1_000,
                back_pointers: foks_merkle_store::back_pointer_hash(root_epoch, &back_pointers)?,
                root_node: merkle_commit.root,
                hostchain: hostchain_tail,
                extensions: Vec::new(),
            };
            let exact_root = root.encoded()?;
            let root_hash =
                foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &exact_root)?;
            let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_blob(
                    merkle_key.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &exact_root,
                )?,
            }
            .encoded()?;
            let mutation = foks_server_db::IdentityMutation {
                normalized_name: &validated.normalized_name,
                reservation_token: &reservation.token,
                reservation_sequence: reservation.sequence,
                reservation_expires_at,
                username_utf8: &validated.username_utf8,
                username_commitment_key: &validated.username_commitment_key,
                uid: validated.uid.as_bytes(),
                device_id: validated.device_id.as_bytes(),
                device_hepk_fingerprint: &validated.device_hepk_fingerprint,
                self_token: &request.self_token,
                exact_device_hepk: &validated.exact_device_hepk,
                exact_device_name: &validated.exact_device_name,
                subkey_id: validated.subkey_id.as_ref().map(EntityId::as_bytes),
                exact_subkey_box: validated.exact_subkey_box.as_deref(),
                yubi_pq_hint: validated
                    .yubi_pq_hint
                    .as_ref()
                    .map(|(slot, id)| (*slot, id)),
                link_hash: &validated.link_hash,
                exact_link: &validated.exact_link,
                tree_location: &validated.next_tree_location,
                subchain_tree_location_seed: &validated.subchain_tree_location_seed,
                shared_role_type: foks_proto::Role::OWNER.protocol_value(),
                shared_visibility: 0,
                shared_generation: 1,
                shared_verify_key: validated.puk_verify_key.as_bytes(),
                exact_shared_hepk: &validated.exact_puk_hepk,
                exact_parcel: &validated.exact_parcel,
                expected_root_hash: Some(authoritative.root_hash),
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
                invite: match (&invite_hash, invite_kind) {
                    (Some(hash), Some(kind)) => foks_server_db::InviteConsumption::Code {
                        code_hash: hash,
                        kind,
                    },
                    (None, None) => foks_server_db::InviteConsumption::Empty,
                    _ => return Err(crate::Error::Signup("invalid invite binding")),
                },
                passphrase: passphrase
                    .as_ref()
                    .map(|passphrase| passphrase.as_database(now)),
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
            Err(crate::Error::Database(foks_server_db::Error::BadInvite)) => {
                Err(RpcStatus::BadInvite)
            }
            Err(crate::Error::Database(
                foks_server_db::Error::Reservation | foks_server_db::Error::ReceiptConflict,
            )) => Err(bad_arguments("signup reservation or retry binding failed")),
            Err(crate::Error::WriterQueue) => Err(RpcStatus::RateLimited),
            Err(error) => {
                Err(crate::error::merkle_mint_status(&error).unwrap_or(RpcStatus::TransactionRetry))
            }
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

    /// getCurrentRoot @2 returns the bare unsigned MerkleRoot, matching the
    /// v0.1.9 wire contract. The authenticated advance path must instead use
    /// getCurrentRootSigned @5 (below) so it can verify the signature.
    fn current_root(&self) -> std::result::Result<Vec<u8>, RpcStatus> {
        if self.read_database.is_none() {
            let signed =
                SignedBlob::decode(&self.current_root).map_err(|_| RpcStatus::TransactionRetry)?;
            return Ok(signed.inner);
        }
        let root = self.validated_current_root()?;
        Ok(root.exact_root)
    }

    /// getCurrentRootSigned @5 returns the delegated Merkle-signer SignedBlob so
    /// the client can authenticate the current root before accepting it.
    fn current_root_signed(&self) -> std::result::Result<Vec<u8>, RpcStatus> {
        if self.read_database.is_none() {
            return Ok(self.current_root.clone());
        }
        let root = self.validated_current_root()?;
        Ok(root.exact_signed_root)
    }

    fn validated_current_root(
        &self,
    ) -> std::result::Result<foks_server_db::RootSnapshot, RpcStatus> {
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
        Ok(root)
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
        let (full_epochs, hash_epochs) = decode_historical_roots_argument(argument, &self.host_id)?;
        if self.read_database.is_none() {
            return Err(RpcStatus::Unsupported);
        }
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let full = snapshot
            .roots_at(&full_epochs)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        let hashes = snapshot
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

    fn merkle_lookup(&self, argument: &[u8]) -> std::result::Result<Vec<u8>, RpcStatus> {
        let argument = decode_merkle_lookup(argument).map_err(bad_arguments)?;
        self.validate_optional_host(argument.host.as_ref())?;
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let root = select_merkle_lookup_root(&snapshot, argument.signed, argument.root)?;
        let path = foks_merkle_store::proof(&snapshot.node_reader(), root.root_node, argument.key)
            .map_err(map_merkle_proof_error)?;
        MerkleLookupResponse {
            root: validated_root(&root)?,
            path,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    fn merkle_multi_lookup(&self, argument: &[u8]) -> std::result::Result<Vec<u8>, RpcStatus> {
        let argument = decode_merkle_multi_lookup(argument).map_err(bad_arguments)?;
        self.validate_optional_host(argument.host.as_ref())?;
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let root = select_merkle_lookup_root(&snapshot, argument.signed, argument.root)?;
        let reader = snapshot.node_reader();
        let paths = argument
            .keys
            .into_iter()
            .map(|key| {
                foks_merkle_store::proof(&reader, root.root_node, key)
                    .map_err(map_merkle_proof_error)
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        MerkleMultiLookupResponse {
            root: validated_root(&root)?,
            paths,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    fn current_root_hash(&self, argument: &[u8]) -> std::result::Result<Vec<u8>, RpcStatus> {
        let host = decode_merkle_optional_host(argument).map_err(bad_arguments)?;
        self.validate_optional_host(host.as_ref())?;
        let root = self
            .read_database()?
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or(RpcStatus::MerkleNoRoot)?;
        validated_root(&root)?;
        TreeRoot {
            epoch: root.epoch,
            hash: root.root_hash,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    fn merkle_check_key_exists(&self, argument: &[u8]) -> std::result::Result<Vec<u8>, RpcStatus> {
        let argument = decode_merkle_check_key(argument).map_err(bad_arguments)?;
        self.validate_optional_host(argument.host.as_ref())?;
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let epoch = snapshot
            .merkle_leaf_epoch(&argument.key)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or(RpcStatus::MerkleLeafNotFound)?;
        MerkleExistsResponse {
            epoch,
            // Root publication and signing are one atomic standalone commit.
            signed: true,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
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

    fn validate_optional_host(
        &self,
        host: Option<&EntityId>,
    ) -> std::result::Result<(), RpcStatus> {
        if host.is_some_and(|host| host.as_bytes() != self.host_id) {
            return Err(RpcStatus::NotFound("host not found".to_owned()));
        }
        Ok(())
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

fn select_merkle_lookup_root(
    snapshot: &foks_server_db::ReadSnapshot<'_>,
    signed: bool,
    epoch: Option<u64>,
) -> std::result::Result<foks_server_db::RootSnapshot, RpcStatus> {
    if epoch.is_some_and(|epoch| i64::try_from(epoch).is_err()) {
        return Err(RpcStatus::MerkleNoRoot);
    }
    let root = match epoch {
        Some(epoch) => snapshot
            .roots_at(&[epoch])
            .map_err(|_| RpcStatus::TransactionRetry)?
            .and_then(|mut roots| roots.pop()),
        None => snapshot
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?,
    }
    .ok_or(RpcStatus::MerkleNoRoot)?;
    validated_root(&root)?;
    if signed {
        let signed =
            SignedBlob::decode(&root.exact_signed_root).map_err(|_| RpcStatus::TransactionRetry)?;
        if signed.inner != root.exact_root {
            return Err(RpcStatus::TransactionRetry);
        }
    }
    Ok(root)
}

fn map_merkle_proof_error(error: foks_merkle_store::Error) -> RpcStatus {
    match error {
        // Storage access can be transient. Every other proof failure means
        // that the selected authenticated root cannot be traversed as a
        // valid content-addressed tree and is the Go MERKLE_VERIFY_ERROR.
        foks_merkle_store::Error::Storage(_) => RpcStatus::TransactionRetry,
        _ => RpcStatus::MerkleVerify("Merkle proof verification failed".to_owned()),
    }
}

fn validate_host_argument_against(
    argument: &[u8],
    expected_host: &[u8],
    allow_omitted: bool,
) -> std::result::Result<(), RpcStatus> {
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("host argument is not a struct"));
    };
    let [host] = fields.as_slice() else {
        return Err(bad_arguments("host argument has the wrong shape"));
    };
    if allow_omitted && matches!(host, Value::Null) {
        return Ok(());
    }
    validate_host_value_against(host, expected_host)
}

fn validate_host_value_against(
    host: &Value,
    expected_host: &[u8],
) -> std::result::Result<(), RpcStatus> {
    let Value::Binary(host) = host else {
        return Err(bad_arguments("host argument is not binary"));
    };
    if host.len() != 33 || host.first() != Some(&foks_proto::ENTITY_HOST) {
        return Err(bad_arguments("host argument is not a HostID"));
    }
    if host.as_slice() != expected_host {
        return Err(RpcStatus::NotFound("host not found".to_owned()));
    }
    Ok(())
}

fn validate_optional_host_value_against(
    host: &Value,
    expected_host: &[u8],
) -> std::result::Result<(), RpcStatus> {
    if matches!(host, Value::Null) {
        return Ok(());
    }
    validate_host_value_against(host, expected_host)
}

fn decode_historical_roots_argument(
    argument: &[u8],
    expected_host: &[u8],
) -> std::result::Result<(Vec<u64>, Vec<u64>), RpcStatus> {
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("historical roots argument is not a struct"));
    };
    let [host, full_epochs, hash_epochs] = fields.as_slice() else {
        return Err(bad_arguments(
            "historical roots argument has the wrong shape",
        ));
    };
    validate_optional_host_value_against(host, expected_host)?;
    Ok((decode_epochs(full_epochs)?, decode_epochs(hash_epochs)?))
}

fn validated_root(
    root: &foks_server_db::RootSnapshot,
) -> std::result::Result<MerkleRoot, RpcStatus> {
    decode_stored_root(root).map_err(|_| RpcStatus::TransactionRetry)
}

fn decode_stored_root(root: &foks_server_db::RootSnapshot) -> Result<MerkleRoot> {
    let decoded = MerkleRoot::decode(&root.exact_root)?;
    let hash =
        foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &root.exact_root)?;
    if decoded.epoch != root.epoch || decoded.root_node != root.root_node || hash != root.root_hash
    {
        return Err(crate::Error::Signup("stored Merkle root binding mismatch"));
    }
    Ok(decoded)
}

fn require_cited_root(
    database: &foks_server_db::Database,
    cited: &TreeRoot,
) -> Result<foks_server_db::RootSnapshot> {
    let root = database
        .root_at(cited.epoch)?
        .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
    decode_stored_root(&root)?;
    if root.root_hash != cited.hash {
        return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
    }
    Ok(root)
}

fn mutation_cites_superseded_user_head(
    database: &foks_server_db::Database,
    authority: &foks_server_db::UserAuthoritySnapshot,
    host: &EntityId,
    change: &foks_proto::UserGroupChange,
) -> Result<bool> {
    if change.uid.as_bytes() != authority.uid
        || &change.host != host
        || change.seqno > authority.chain_sequence
    {
        return Ok(false);
    }
    let Some(previous_sequence) = change.seqno.checked_sub(1) else {
        return Ok(false);
    };
    if change.previous.is_none() {
        return Ok(false);
    }
    let chain = database
        .user_chain(&authority.uid)?
        .ok_or(crate::Error::Signup("user mutation chain missing"))?;
    let Some(historical_head) = chain
        .links
        .iter()
        .find(|link| link.sequence == previous_sequence)
    else {
        return Ok(false);
    };
    change_cites_historical_user_head(authority, host, change, historical_head)
}

fn change_cites_historical_user_head(
    authority: &foks_server_db::UserAuthoritySnapshot,
    host: &EntityId,
    change: &foks_proto::UserGroupChange,
    historical_head: &foks_server_db::UserChainLinkSnapshot,
) -> Result<bool> {
    let historical_hash = foks_crypto::prefixed_hash_signable(
        foks_proto::LINK_OUTER_TYPE_ID,
        &historical_head.exact_link,
    )?;
    Ok(change.uid.as_bytes() == authority.uid
        && &change.host == host
        && change.seqno <= authority.chain_sequence
        && historical_head.sequence.checked_add(1) == Some(change.seqno)
        && change.previous == Some(historical_hash))
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
            service_data.metrics.request_rate_limited();
            let response = encode_status_response_at(&RpcStatus::RateLimited, 0)?;
            write_response(&mut stream, &response, limits.io_timeout).await?;
            service_data.metrics.response_completed();
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
        let kex_receive = matches!(
            &routed,
            Ok(call) if call.route.id == crate::rpc::RouteId::KexReceive
        );
        let outcome = if kex_receive {
            // Go-compatible KEX receives can remain open for up to an hour.
            // Await the relay directly rather than occupying a blocking worker;
            // the listener's active-connection semaphore remains the bound.
            let _request_memory = request_memory;
            let _handler_timer =
                crate::ServerMetrics::handler_timer(Arc::clone(&service_data.metrics));
            let call = match routed {
                Ok(call) => call,
                Err(_) => unreachable!("KEX receive was recognized above"),
            };
            let route = Some(call.route);
            if service_data.should_disconnect(
                crate::SessionFaultPoint::BeforeDurableMutation,
                call.route.protocol,
                call.route.method,
            ) {
                RequestOutcome {
                    response: Vec::new(),
                    route,
                    disconnect_before_response: true,
                }
            } else {
                let response = tokio::select! {
                    result = handlers::kex_receive_response(service_data.as_ref(), call) => {
                        match result {
                            Ok(response) => response,
                            Err(status) => encode_status_response_at(&status, sequence)?,
                        }
                    }
                    // Snowpack is request/response serial. Readability while
                    // this response is pending is therefore either a client
                    // cancellation/control frame or socket shutdown. Closing
                    // promptly drops the relay future and active-connection
                    // permit instead of retaining an abandoned Go long poll.
                    result = stream.get_ref().0.readable() => {
                        let _ = result;
                        return Ok(());
                    }
                    result = stop.changed() => {
                        let _ = result;
                        break;
                    }
                };
                RequestOutcome {
                    response,
                    route,
                    disconnect_before_response: false,
                }
            }
        } else {
            match execute_bounded(
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
                    let response = match routed {
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
                                    route,
                                    disconnect_before_response: true,
                                });
                            }
                            match handlers::response(data.as_ref(), call, principal.as_ref()) {
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
                },
            )
            .await?
            {
                BoundedExecution::Completed(outcome) => outcome?,
                BoundedExecution::Saturated => {
                    service_data.metrics.request_rate_limited();
                    let response = encode_status_response_at(&RpcStatus::RateLimited, sequence)?;
                    write_response(&mut stream, &response, limits.io_timeout).await?;
                    service_data.metrics.response_completed();
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
            }
        };
        if outcome.disconnect_before_response {
            return Ok(());
        }
        let response = outcome.response;
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
        service_data.metrics.response_completed();
    }
    Ok(())
}

struct RequestOutcome {
    response: Vec<u8>,
    route: Option<&'static crate::rpc::RouteSpec>,
    disconnect_before_response: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_integrity_failures_are_not_laundered_as_retries() {
        assert_eq!(
            map_merkle_proof_error(foks_merkle_store::Error::HashMismatch),
            RpcStatus::MerkleVerify("Merkle proof verification failed".to_owned())
        );
        assert_eq!(
            map_merkle_proof_error(foks_merkle_store::Error::MissingNode([0x41; 32])),
            RpcStatus::MerkleVerify("Merkle proof verification failed".to_owned())
        );
        assert_eq!(
            map_merkle_proof_error(foks_merkle_store::Error::EmptyTree),
            RpcStatus::MerkleVerify("Merkle proof verification failed".to_owned())
        );
        assert_eq!(
            map_merkle_proof_error(foks_merkle_store::Error::Storage("busy".to_owned())),
            RpcStatus::TransactionRetry
        );
    }

    #[test]
    fn only_an_exact_historical_user_head_is_a_retryable_race() {
        let uid = EntityId::from_bytes(
            std::iter::once(foks_proto::ENTITY_USER)
                .chain([0x31; 32])
                .collect(),
        )
        .unwrap();
        let host = EntityId::from_bytes(
            std::iter::once(foks_proto::ENTITY_HOST)
                .chain([0x41; 32])
                .collect(),
        )
        .unwrap();
        let historical = foks_server_db::UserChainLinkSnapshot {
            sequence: 2,
            exact_link: vec![0x91, 0x01],
            root_epoch: 2,
            next_tree_location: [0x51; 32],
        };
        let historical_hash = foks_crypto::prefixed_hash_signable(
            foks_proto::LINK_OUTER_TYPE_ID,
            &historical.exact_link,
        )
        .unwrap();
        let authority = foks_server_db::UserAuthoritySnapshot {
            uid: uid.as_bytes().to_vec(),
            chain_sequence: 3,
            chain_tail_hash: [0x61; 32],
            next_tree_location: [0x71; 32],
            current_root_epoch: 3,
            current_root_hash: [0x81; 32],
            devices: Vec::new(),
            shared_keys: Vec::new(),
            stale_shared_key_roles: Vec::new(),
        };
        let mut change = foks_proto::UserGroupChange {
            seqno: 3,
            previous: Some(historical_hash),
            root: TreeRoot {
                epoch: 2,
                hash: [0x91; 32],
            },
            time: 1,
            next_location_commitment: [0xa1; 32],
            uid,
            host: host.clone(),
            signer: EntityId::from_bytes(
                std::iter::once(foks_proto::ENTITY_DEVICE)
                    .chain([0xb1; 32])
                    .collect(),
            )
            .unwrap(),
            changes: Vec::new(),
            shared_keys: Vec::new(),
            metadata: Vec::new(),
        };

        assert!(
            change_cites_historical_user_head(&authority, &host, &change, &historical).unwrap()
        );
        change.previous = Some([0xc1; 32]);
        assert!(
            !change_cites_historical_user_head(&authority, &host, &change, &historical).unwrap()
        );
        change.previous = Some(historical_hash);
        change.seqno = 4;
        assert!(
            !change_cites_historical_user_head(&authority, &host, &change, &historical).unwrap()
        );
    }

    #[test]
    fn merkle_query_hosts_may_be_omitted_but_vhost_selection_remains_strict() {
        let mut host = vec![0x41; 33];
        host[0] = foks_proto::ENTITY_HOST;
        let omitted = foks_snowpack::encode(&Value::Array(vec![Value::Null])).unwrap();
        let explicit =
            foks_snowpack::encode(&Value::Array(vec![Value::Binary(host.clone())])).unwrap();
        let mut other_host = host.clone();
        other_host[1] ^= 1;
        let mismatched =
            foks_snowpack::encode(&Value::Array(vec![Value::Binary(other_host.clone())])).unwrap();

        validate_host_argument_against(&omitted, &host, true).unwrap();
        validate_host_argument_against(&explicit, &host, true).unwrap();
        validate_host_argument_against(&explicit, &host, false).unwrap();
        validate_optional_host_value_against(&Value::Null, &host).unwrap();
        validate_optional_host_value_against(&Value::Binary(host.clone()), &host).unwrap();
        let omitted_historical = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-historical-roots-omitted-host-request.frame",
            ),
        )
        .unwrap();
        let call = foks_rpc::read_call(
            &mut std::io::Cursor::new(omitted_historical),
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
        )
        .unwrap();
        assert_eq!(
            decode_historical_roots_argument(call.argument(), &host).unwrap(),
            (vec![996], vec![997, 996, 994, 992])
        );
        assert!(matches!(
            validate_host_argument_against(&mismatched, &host, true),
            Err(RpcStatus::NotFound(_))
        ));
        assert!(matches!(
            validate_optional_host_value_against(&Value::Binary(other_host), &host),
            Err(RpcStatus::NotFound(_))
        ));
        assert!(matches!(
            validate_host_argument_against(&omitted, &host, false),
            Err(RpcStatus::BadArguments(_))
        ));
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
