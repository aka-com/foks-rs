//! Credentials, Merkle advancement, and authenticated user loading.

use super::{
    authenticate_historical_roots_from_latest, decode, derive_device_public, derive_subkey_id,
    encode_get_client_cert_chain_request_at, encode_get_current_merkle_root_request,
    encode_get_historical_merkle_roots_request, encode_get_puk_for_role_request,
    encode_load_user_chain_request_from, encode_merkle_select_vhost_request,
    encode_registration_select_vhost_request, merkle_history_requirements,
    open_puk_parcel_for_role, open_puk_parcel_with_for_role, open_puk_seed_chain,
    restore_merkle_anchor, user_chain_root_epochs, verify_merkle_advance, verify_user_chain,
    verify_user_chain_increment, Acceptance, AuthenticatedMerkleRoots, EntityId, Error, FoksClient,
    HardStateStore, HostchainTail, PinnedHost, PukParcel, Result, Role, SecretSeed, Value,
    VerifiedMerkleAdvance, VerifiedUserState, YubiDevice, ENTITY_USER,
};
use foks_crypto::{open_subkey_box, sign_yubi_typed};
use foks_proto::{RegistrationChallenge, ENTITY_SUBKEY, REG_CHALLENGE_PAYLOAD_TYPE_ID};
use foks_rpc::{encode_get_subkey_box_challenge_request, encode_load_subkey_box_request};

const YUBI_CHALLENGE_WINDOW_MILLISECONDS: u64 = 15 * 60 * 1_000;

/// Device credential material used for mTLS. The master seed is never written
/// by this crate; callers should source it from the encrypted local key store.
pub struct DeviceCredential {
    pub uid: EntityId,
    pub seed: SecretSeed,
    pub certificate_chain: Vec<Vec<u8>>,
}

/// Yubi parent decapsulation/signing plus the software Ed25519 subkey used
/// only for mTLS.
pub struct YubiCredential<'a> {
    pub uid: EntityId,
    pub parent: &'a dyn YubiDevice,
    pub subkey_seed: SecretSeed,
    pub certificate_chain: Vec<Vec<u8>>,
}

#[derive(Debug)]
pub struct AuthenticatedUserOutcome {
    pub merkle_acceptance: Acceptance,
    pub acceptance: Acceptance,
    pub verified: VerifiedUserState,
    pub puks: Vec<UserPrivateKey>,
}

#[derive(Debug)]
pub struct UserPrivateKey {
    pub role: Role,
    pub generation: u64,
    pub seed: SecretSeed,
}

impl AuthenticatedUserOutcome {
    pub fn current_puk(&self) -> Option<&UserPrivateKey> {
        self.puks.last()
    }
}

pub(crate) type UserChainCursor<'a> = (u64, Option<(&'a [u8], u64)>);

pub(crate) fn user_chain_cursor(prior: Option<&VerifiedUserState>) -> Result<UserChainCursor<'_>> {
    match prior {
        Some(prior) => Ok((
            prior
                .chain_seqno()
                .checked_add(1)
                .ok_or(Error::UserBinding("user chain sequence overflow"))?,
            Some((
                prior.username(),
                prior
                    .username_sequence()
                    .checked_add(1)
                    .ok_or(Error::UserBinding("username sequence overflow"))?,
            )),
        )),
        None => Ok((1, None)),
    }
}

impl FoksClient {
    /// Recovers the delegated software subkey from the server using a fresh
    /// challenge signed by the hardware parent. `expected_subkey` is durable
    /// locator metadata and prevents a server from substituting another key.
    pub fn recover_yubi_credential<'a>(
        &self,
        host: &PinnedHost,
        uid: EntityId,
        expected_subkey: &EntityId,
        parent: &'a dyn YubiDevice,
    ) -> Result<YubiCredential<'a>> {
        if uid.entity_type() != ENTITY_USER
            || expected_subkey.entity_type() != ENTITY_SUBKEY
            || parent.entity_id().entity_type() != foks_proto::ENTITY_YUBI
        {
            return Err(Error::CredentialBinding(
                "invalid Yubi recovery credential types",
            ));
        }
        let challenge_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_get_subkey_box_challenge_request(parent.entity_id())?,
        )?;
        let challenge = RegistrationChallenge::decode(&challenge_bytes)?;
        let now = current_milliseconds()?;
        if challenge.payload.entity != *parent.entity_id()
            || challenge.payload.host != *host.host_id()
            || challenge.payload.time.abs_diff(now) > YUBI_CHALLENGE_WINDOW_MILLISECONDS
        {
            return Err(Error::CredentialBinding(
                "Yubi challenge is not bound to the selected card and host",
            ));
        }
        let payload = challenge.payload.encoded()?;
        let signature = sign_yubi_typed(parent, REG_CHALLENGE_PAYLOAD_TYPE_ID, &payload)?;
        let boxed = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_load_subkey_box_request(parent.entity_id(), &challenge, &signature)?,
        )?;
        let subkey_seed = open_subkey_box(&boxed, parent, expected_subkey)?;
        let certificate_chain = self.fetch_subkey_certificate_chain(host, &uid, &subkey_seed)?;
        Ok(YubiCredential {
            uid,
            parent,
            subkey_seed,
            certificate_chain,
        })
    }

    /// Requests the X.509 certificate chain for an already enrolled device.
    /// This registration call is intentionally unauthenticated; possession of
    /// the matching Ed25519 private key is proved by the subsequent mTLS
    /// handshake.
    pub fn fetch_device_certificate_chain(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        seed: &SecretSeed,
    ) -> Result<Vec<Vec<u8>>> {
        if uid.entity_type() != ENTITY_USER {
            return Err(Error::InvalidUserId);
        }
        let device = derive_device_public(seed)?;
        let request =
            encode_get_client_cert_chain_request_at(uid.as_bytes(), device.id.as_bytes(), 1)?;
        let response = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &request,
        )?;
        let Value::Array(certificates) = decode(&response)? else {
            return Err(Error::CertificateChain);
        };
        let certificates = certificates
            .into_iter()
            .map(|certificate| match certificate {
                Value::Binary(bytes) if !bytes.is_empty() => Ok(bytes),
                _ => Err(Error::CertificateChain),
            })
            .collect::<Result<Vec<_>>>()?;
        if certificates.is_empty() {
            return Err(Error::CertificateChain);
        }
        Ok(certificates)
    }

    pub fn fetch_subkey_certificate_chain(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        subkey_seed: &SecretSeed,
    ) -> Result<Vec<Vec<u8>>> {
        if uid.entity_type() != ENTITY_USER {
            return Err(Error::InvalidUserId);
        }
        let subkey = derive_subkey_id(subkey_seed)?;
        let request =
            encode_get_client_cert_chain_request_at(uid.as_bytes(), subkey.as_bytes(), 1)?;
        let response = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &request,
        )?;
        let Value::Array(certificates) = decode(&response)? else {
            return Err(Error::CertificateChain);
        };
        let certificates = certificates
            .into_iter()
            .map(|certificate| match certificate {
                Value::Binary(bytes) if !bytes.is_empty() => Ok(bytes),
                _ => Err(Error::CertificateChain),
            })
            .collect::<Result<Vec<_>>>()?;
        if certificates.is_empty() {
            return Err(Error::CertificateChain);
        }
        Ok(certificates)
    }

    fn load_user_chain(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        prior: Option<&VerifiedUserState>,
    ) -> Result<Vec<u8>> {
        let (start, name) = user_chain_cursor(prior)?;
        let request = encode_load_user_chain_request_from(credential.uid.as_bytes(), start, name)?;
        self.call(host, &host.user, &request, Some(credential))
    }

    fn fetch_puk_parcel(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        role: Role,
    ) -> Result<Vec<u8>> {
        let device = derive_device_public(&credential.seed)?;
        let request = encode_get_puk_for_role_request(role, device.id.as_bytes())?;
        self.call(host, &host.user, &request, Some(credential))
    }

    pub(crate) fn load_puks_for_role(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        verified: &VerifiedUserState,
        role: Role,
    ) -> Result<Vec<UserPrivateKey>> {
        let parcel = PukParcel::decode(&self.fetch_puk_parcel(host, credential, role)?)?;
        let sender = verified
            .devices()
            .iter()
            .find(|device| device.id == parcel.sender)
            .ok_or(Error::UserBinding("PUK parcel sender is not enrolled"))?;
        let role_key = verified.shared_key(role).ok_or(Error::UserBinding(
            "requested PUK role is not in the user chain",
        ))?;
        let clear = open_puk_parcel_for_role(
            &parcel,
            &credential.seed,
            &sender.hepk,
            &role_key.verify_key,
            &role_key.hepk,
            role_key.generation,
            host.host_id(),
            role,
        )?;
        Ok(
            open_puk_seed_chain(clear, &parcel, verified.uid(), host.host_id())?
                .into_iter()
                .map(|key| UserPrivateKey {
                    role: key.role,
                    generation: key.generation,
                    seed: key.into_seed(),
                })
                .collect(),
        )
    }

    pub fn advance_merkle_root(
        &self,
        pinned: &PinnedHost,
    ) -> Result<(Acceptance, VerifiedMerkleAdvance)> {
        let mut store = HardStateStore::open(&pinned.database_path)?;
        let host = store
            .host_for_lookup(&pinned.lookup_name)?
            .ok_or(Error::HostBinding("pinned host is missing"))?;
        if host.host_id.as_slice() != pinned.host_id.as_bytes() {
            return Err(Error::HostBinding("stored HostID changed"));
        }
        let latest_bytes = self.call_after_vhost_selection(
            pinned,
            &pinned.merkle_query,
            &encode_merkle_select_vhost_request(pinned.host_id())?,
            &encode_get_current_merkle_root_request(pinned.host_id(), 1)?,
        )?;
        let latest = foks_proto::MerkleRoot::decode(&latest_bytes)?;
        let history = merkle_history_requirements(latest.epoch, host.merkle_root.epoch)?;
        let historical_bytes = if history.is_empty() {
            foks_snowpack::encode(&Value::Array(vec![Value::Null, Value::Null]))?
        } else {
            self.call_after_vhost_selection(
                pinned,
                &pinned.merkle_query,
                &encode_merkle_select_vhost_request(pinned.host_id())?,
                &encode_get_historical_merkle_roots_request(
                    pinned.host_id(),
                    &history.full_roots,
                    &history.hashes,
                    1,
                )?,
            )?
        };
        let anchor = restore_merkle_anchor(
            host.merkle_root.epoch,
            host.merkle_root.root_hash,
            &host.merkle_root.root_bytes,
            &host.merkle_root.evidence,
            &host.merkle_root.authenticated_roots,
            &host.chain_bytes,
        )?;
        let verified = verify_merkle_advance(
            &anchor,
            &latest_bytes,
            &historical_bytes,
            &HostchainTail {
                seqno: host.chain_seqno,
                hash: host.chain_tail_hash,
            },
        )?;
        let acceptance = store.accept_verified_merkle_root(&host.host_id, verified.snapshot())?;
        Ok((acceptance, verified))
    }

    pub(crate) fn authenticate_user_chain_roots(
        &self,
        host: &PinnedHost,
        latest: &VerifiedMerkleAdvance,
        chain_bytes: &[u8],
    ) -> Result<AuthenticatedMerkleRoots> {
        let targets = user_chain_root_epochs(chain_bytes)?
            .into_iter()
            .filter(|epoch| !latest.authenticated_roots().contains_epoch(*epoch))
            .collect::<Vec<_>>();
        self.authenticate_chain_roots(host, latest, targets, Error::UserBinding)
    }

    pub(crate) fn authenticate_chain_roots(
        &self,
        host: &PinnedHost,
        latest: &VerifiedMerkleAdvance,
        targets: Vec<u64>,
        binding: fn(&'static str) -> Error,
    ) -> Result<AuthenticatedMerkleRoots> {
        if targets.is_empty() {
            return Ok(latest.authenticated_roots().clone());
        }
        let mut full_epochs = std::collections::BTreeSet::new();
        let mut hash_epochs = std::collections::BTreeSet::new();
        for &target in &targets {
            if target == 0 || target >= latest.root().epoch {
                return Err(binding(
                    "user chain references an unauthenticated future Merkle root",
                ));
            }
            full_epochs.insert(target);
            let requirements = merkle_history_requirements(latest.root().epoch, target)?;
            full_epochs.extend(requirements.full_roots);
            hash_epochs.extend(requirements.hashes);
        }
        let full_epochs = full_epochs.into_iter().collect::<Vec<_>>();
        let hash_epochs = hash_epochs.into_iter().collect::<Vec<_>>();
        if full_epochs.len() > 64 || hash_epochs.len() > 64 {
            return Err(binding(
                "user chain requires too many historical Merkle roots",
            ));
        }
        let historical = self.call_after_vhost_selection(
            host,
            &host.merkle_query,
            &encode_merkle_select_vhost_request(host.host_id())?,
            &encode_get_historical_merkle_roots_request(
                host.host_id(),
                &full_epochs,
                &hash_epochs,
                1,
            )?,
        )?;
        Ok(authenticate_historical_roots_from_latest(
            latest,
            &targets,
            &full_epochs,
            &hash_epochs,
            &historical,
        )?)
    }

    /// Advances the host's Merkle pin, authenticates with device mTLS, replays
    /// the user chain, unboxes the enrolled device role's PUK history, and
    /// atomically advances public user hard state in SQLite.
    pub fn authenticate_and_pin(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<AuthenticatedUserOutcome> {
        let derived = derive_device_public(&credential.seed)?;
        let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
        let prior = match self.pinned_user(host, &credential.uid) {
            Ok(prior) => prior,
            Err(Error::Verify(
                foks_verify::Error::PersistedMerkleEvidence
                | foks_verify::Error::UserChainContinuity,
            )) => None,
            Err(error) => return Err(error),
        };
        let chain_bytes = self.load_user_chain(host, credential, prior.as_ref())?;
        let authenticated_roots =
            self.authenticate_user_chain_roots(host, &merkle, &chain_bytes)?;
        let verified = match prior.as_ref() {
            Some(prior) => verify_user_chain_increment(
                &chain_bytes,
                prior,
                &credential.uid,
                &host.host_id,
                &authenticated_roots,
                &merkle.root().hostchain,
            )?,
            None => verify_user_chain(
                &chain_bytes,
                &credential.uid,
                &host.host_id,
                &authenticated_roots,
                &merkle.root().hostchain,
            )?,
        };
        let enrolled = verified
            .devices()
            .iter()
            .find(|device| device.id == derived.id && device.hepk == derived.hepk)
            .ok_or(Error::CredentialBinding(
                "device seed is not enrolled in the verified user chain",
            ))?;
        let role = enrolled.role;
        let parcel_bytes = self.fetch_puk_parcel(host, credential, role)?;
        let parcel = PukParcel::decode(&parcel_bytes)?;
        let sender = verified
            .devices()
            .iter()
            .find(|device| device.id == parcel.sender)
            .ok_or(Error::UserBinding("PUK parcel sender is not enrolled"))?;
        let role_key = verified
            .shared_key(role)
            .ok_or(Error::UserBinding("device role has no PUK"))?;
        let clear = open_puk_parcel_for_role(
            &parcel,
            &credential.seed,
            &sender.hepk,
            &role_key.verify_key,
            &role_key.hepk,
            role_key.generation,
            &host.host_id,
            role,
        )?;
        let puks = open_puk_seed_chain(clear, &parcel, verified.uid(), host.host_id())?
            .into_iter()
            .map(|key| UserPrivateKey {
                role: key.role,
                generation: key.generation,
                seed: key.into_seed(),
            })
            .collect();
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_user(&verified.hard_state_snapshot()?)?;
        Ok(AuthenticatedUserOutcome {
            merkle_acceptance,
            acceptance,
            verified,
            puks,
        })
    }

    /// Authenticates a Yubi-backed device using its software subkey for mTLS
    /// and the hardware parent for PUK decapsulation.
    pub fn authenticate_yubi_and_pin(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<AuthenticatedUserOutcome> {
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
        let prior = self.pinned_user(host, &credential.uid)?;
        let (start, name) = user_chain_cursor(prior.as_ref())?;
        let request = encode_load_user_chain_request_from(credential.uid.as_bytes(), start, name)?;
        let chain_bytes = self.call_with_material(
            host,
            &host.user,
            &request,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        let authenticated_roots =
            self.authenticate_user_chain_roots(host, &merkle, &chain_bytes)?;
        let verified = match prior.as_ref() {
            Some(prior) => verify_user_chain_increment(
                &chain_bytes,
                prior,
                &credential.uid,
                &host.host_id,
                &authenticated_roots,
                &merkle.root().hostchain,
            )?,
            None => verify_user_chain(
                &chain_bytes,
                &credential.uid,
                &host.host_id,
                &authenticated_roots,
                &merkle.root().hostchain,
            )?,
        };
        let parent = verified
            .devices()
            .iter()
            .find(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
            .ok_or(Error::CredentialBinding(
                "Yubi credential is not enrolled in the verified user chain",
            ))?;
        let role = parent.role;
        let request = encode_get_puk_for_role_request(role, parent.id.as_bytes())?;
        let parcel_bytes = self.call_with_material(
            host,
            &host.user,
            &request,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        let parcel = PukParcel::decode(&parcel_bytes)?;
        let sender = verified
            .devices()
            .iter()
            .find(|device| device.id == parcel.sender)
            .ok_or(Error::UserBinding("PUK parcel sender is not enrolled"))?;
        let role_key = verified
            .shared_key(role)
            .ok_or(Error::UserBinding("Yubi parent role has no PUK"))?;
        let clear = open_puk_parcel_with_for_role(
            &parcel,
            credential.parent,
            &sender.hepk,
            &role_key.verify_key,
            &role_key.hepk,
            role_key.generation,
            &host.host_id,
            role,
        )?;
        let puks = open_puk_seed_chain(clear, &parcel, verified.uid(), host.host_id())?
            .into_iter()
            .map(|key| UserPrivateKey {
                role: key.role,
                generation: key.generation,
                seed: key.into_seed(),
            })
            .collect();
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_user(&verified.hard_state_snapshot()?)?;
        Ok(AuthenticatedUserOutcome {
            merkle_acceptance,
            acceptance,
            verified,
            puks,
        })
    }
}

fn current_milliseconds() -> Result<u64> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::CredentialBinding("system clock precedes Unix epoch"))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| Error::CredentialBinding("system clock timestamp overflow"))
}
