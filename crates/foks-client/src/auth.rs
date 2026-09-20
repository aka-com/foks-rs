//! Credentials, Merkle advancement, and authenticated user loading.

use super::{
    authenticate_historical_roots_from_latest, decode, derive_device_public, derive_subkey_id,
    encode_clear_device_nag_request, encode_get_client_cert_chain_request_at,
    encode_get_current_merkle_root_hash_request, encode_get_current_merkle_root_signed_request,
    encode_get_device_nag_request, encode_get_historical_merkle_roots_request,
    encode_get_puk_for_role_request, encode_load_user_chain_as_local_team_request,
    encode_load_user_chain_open_host_request, encode_load_user_chain_request_from,
    encode_merkle_check_key_exists_request, encode_merkle_lookup_request,
    encode_merkle_multi_lookup_request, encode_merkle_select_vhost_request,
    encode_registration_select_vhost_request, encode_resolve_username_request,
    encode_user_ping_request, merkle_history_requirements, open_puk_parcel_with_for_role,
    open_puk_seed_chain, user_chain_root_epochs, verify_non_self_user_chain,
    verify_non_self_user_chain_increment, verify_signed_merkle_advance, verify_user_chain,
    verify_user_chain_increment, Acceptance, AuthenticatedMerkleRoots, DeviceNagInfo, EntityId,
    Error, FoksClient, HardStateStore, HostchainTail, PinnedHost, PukParcel, Result, Role,
    SecretSeed, Value, VerifiedMerkleAdvance, VerifiedUserState, YubiDevice, ENTITY_USER,
};
use foks_crypto::{open_subkey_box, sign_yubi_typed};
use foks_proto::{
    RegistrationChallenge, TeamChain, UserChain, ENTITY_SUBKEY, REG_CHALLENGE_PAYLOAD_TYPE_ID,
};
use foks_rpc::{encode_get_subkey_box_challenge_request, encode_load_subkey_box_request};
use foks_verify::team_chain_root_epochs;

const YUBI_CHALLENGE_WINDOW_MILLISECONDS: u64 = 15 * 60 * 1_000;

/// Device credential material used for mTLS. The master seed is never written
/// by this crate; callers should source it from the encrypted local key store.
pub use foks_crypto::SoftwareKeyKind;
pub struct DeviceCredential {
    pub key_kind: SoftwareKeyKind,
    pub uid: EntityId,
    pub seed: SecretSeed,
    pub certificate_chain: Vec<Vec<u8>>,
}

impl DeviceCredential {
    pub fn public_material(&self) -> Result<foks_crypto::DevicePublicMaterial> {
        Ok(foks_crypto::derive_software_public(
            &self.seed,
            self.key_kind,
        )?)
    }
}

/// Yubi parent decapsulation/signing plus the software Ed25519 subkey used
/// only for mTLS.
pub struct YubiCredential<'a> {
    pub uid: EntityId,
    pub parent: &'a dyn YubiDevice,
    pub subkey_seed: SecretSeed,
    pub certificate_chain: Vec<Vec<u8>>,
}

/// One acting credential for an operation that can be driven either by a
/// software device or by an already-unlocked Yubi parent. Both forms present
/// the same transport shape to the host: an Ed25519 mTLS seed plus its
/// certificate chain. A Yubi parent's private key never appears here; only
/// its delegated software subkey is used for mTLS, exactly as
/// [`FoksClient::authenticate_yubi_and_pin`] does.
#[derive(Clone, Copy)]
pub enum FederationCredential<'a, 'device> {
    Software(&'a DeviceCredential),
    Yubi(&'a YubiCredential<'device>),
}

impl<'a, 'device> FederationCredential<'a, 'device> {
    pub fn uid(&self) -> &'a EntityId {
        match self {
            Self::Software(credential) => &credential.uid,
            Self::Yubi(credential) => &credential.uid,
        }
    }

    /// True when hardware holds the acting chain device's private key.
    pub fn is_hardware(&self) -> bool {
        matches!(self, Self::Yubi(_))
    }

    /// mTLS material. For a Yubi credential this is the delegated subkey, so
    /// no hardware secret is ever copied out of the device.
    pub(crate) fn transport(&self) -> (&'a SecretSeed, &'a [Vec<u8>]) {
        match self {
            Self::Software(credential) => (&credential.seed, &credential.certificate_chain),
            Self::Yubi(credential) => (&credential.subkey_seed, &credential.certificate_chain),
        }
    }

    /// The chain device that must be enrolled for this credential to act:
    /// the software device itself, or the Yubi parent.
    pub fn device_id(&self) -> Result<EntityId> {
        match self {
            Self::Software(credential) => Ok(credential.public_material()?.id),
            Self::Yubi(credential) => Ok(credential.parent.entity_id().clone()),
        }
    }

    /// Fails closed unless `user` is this credential's own verified chain and
    /// enrolls exactly this device. A Yubi credential must additionally match
    /// the parent's HEPK and its delegated subkey; matching only the UID is
    /// insufficient to authenticate the hardware identity.
    pub(crate) fn require_enrolled(&self, host: &EntityId, user: &VerifiedUserState) -> Result<()> {
        if user.uid() != self.uid() || user.host() != host {
            return Err(Error::UserBinding(
                "transport user does not match the credential and pinned host",
            ));
        }
        match self {
            Self::Software(credential) => {
                let derived = credential.public_material()?;
                user.devices()
                    .iter()
                    .find(|device| device.id == derived.id && device.hepk == derived.hepk)
                    .map(|_| ())
                    .ok_or(Error::CredentialBinding(
                        "device seed is not enrolled in the verified user chain",
                    ))
            }
            Self::Yubi(credential) => {
                let subkey = derive_subkey_id(&credential.subkey_seed)?;
                user.devices()
                    .iter()
                    .find(|device| {
                        device.id == *credential.parent.entity_id()
                            && device.hepk == *credential.parent.hepk()
                            && device.subkey.as_ref() == Some(&subkey)
                    })
                    .map(|_| ())
                    .ok_or(Error::CredentialBinding(
                        "Yubi credential is not enrolled in the verified user chain",
                    ))
            }
        }
    }
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
    pub fn merkle_lookup(
        &self,
        host: &PinnedHost,
        key: [u8; 32],
        signed: bool,
        root: Option<u64>,
    ) -> Result<foks_proto::MerkleLookupResponse> {
        let response = self.call_after_vhost_selection(
            host,
            &host.merkle_query,
            &encode_merkle_select_vhost_request(host.host_id())?,
            &encode_merkle_lookup_request(Some(host.host_id()), key, signed, root, 1)?,
        )?;
        Ok(foks_proto::MerkleLookupResponse::decode(&response)?)
    }

    pub fn merkle_multi_lookup(
        &self,
        host: &PinnedHost,
        keys: &[[u8; 32]],
        signed: bool,
        root: Option<u64>,
    ) -> Result<foks_proto::MerkleMultiLookupResponse> {
        let response = self.call_after_vhost_selection(
            host,
            &host.merkle_query,
            &encode_merkle_select_vhost_request(host.host_id())?,
            &encode_merkle_multi_lookup_request(Some(host.host_id()), keys, signed, root, 1)?,
        )?;
        Ok(foks_proto::MerkleMultiLookupResponse::decode(&response)?)
    }

    pub fn current_merkle_root_hash(&self, host: &PinnedHost) -> Result<foks_proto::TreeRoot> {
        let response = self.call_after_vhost_selection(
            host,
            &host.merkle_query,
            &encode_merkle_select_vhost_request(host.host_id())?,
            &encode_get_current_merkle_root_hash_request(Some(host.host_id()), 1)?,
        )?;
        Ok(foks_proto::TreeRoot::decode(&response)?)
    }

    pub fn merkle_key_exists(
        &self,
        host: &PinnedHost,
        key: [u8; 32],
    ) -> Result<foks_proto::MerkleExistsResponse> {
        let response = self.call_after_vhost_selection(
            host,
            &host.merkle_query,
            &encode_merkle_select_vhost_request(host.host_id())?,
            &encode_merkle_check_key_exists_request(Some(host.host_id()), key, 1)?,
        )?;
        Ok(foks_proto::MerkleExistsResponse::decode(&response)?)
    }

    pub fn resolve_username(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        username_utf8: &str,
        open_host: bool,
    ) -> Result<EntityId> {
        let normalized = foks_verify::normalize_username(username_utf8.as_bytes()).ok_or(
            Error::UserBinding("username is not valid after normalization"),
        )?;
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_resolve_username_request(&normalized, open_host)?,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        let Value::Binary(uid) = decode(&response)? else {
            return Err(Error::CredentialBinding(
                "username resolution returned a non-UID value",
            ));
        };
        EntityId::from_bytes(uid)?
            .require_type(ENTITY_USER)
            .map_err(Into::into)
    }

    pub fn device_nag(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<DeviceNagInfo> {
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_get_device_nag_request()?,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        DeviceNagInfo::decode(&response).map_err(Into::into)
    }

    pub fn clear_device_nag(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        cleared: bool,
    ) -> Result<()> {
        self.call_void_with_material(
            host,
            &host.user,
            &encode_clear_device_nag_request(cleared)?,
            &credential.seed,
            &credential.certificate_chain,
        )
    }

    pub fn ping(&self, host: &PinnedHost, credential: &DeviceCredential) -> Result<EntityId> {
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_user_ping_request()?,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        let Value::Binary(uid) = decode(&response)? else {
            return Err(Error::CredentialBinding(
                "user ping returned a non-UID value",
            ));
        };
        let uid = EntityId::from_bytes(uid)?.require_type(ENTITY_USER)?;
        if uid != credential.uid {
            return Err(Error::CredentialBinding(
                "user ping returned a different UID",
            ));
        }
        Ok(uid)
    }

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

    /// Loads another local user's public chain through an already activated
    /// team-view token. This is the Go TeamLoader authorization used by CLKR:
    /// the caller learns only authenticated public PUK material and never a
    /// target user's private parcel.
    pub fn load_and_pin_user_as_local_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        target: &EntityId,
        team_view_token: &[u8; 16],
    ) -> Result<VerifiedUserState> {
        self.load_and_pin_other_local_user_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
            target,
            |start, name| {
                encode_load_user_chain_as_local_team_request(
                    target.as_bytes(),
                    start,
                    name,
                    team_view_token,
                )
            },
        )
    }

    /// Hardware-backed transport variant of
    /// [`Self::load_and_pin_user_as_local_team`]. The Yubi parent remains the
    /// authenticated chain actor while its delegated subkey supplies mTLS.
    pub fn load_and_pin_user_as_local_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        target: &EntityId,
        team_view_token: &[u8; 16],
    ) -> Result<VerifiedUserState> {
        self.load_and_pin_other_local_user_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
            target,
            |start, name| {
                encode_load_user_chain_as_local_team_request(
                    target.as_bytes(),
                    start,
                    name,
                    team_view_token,
                )
            },
        )
    }

    /// Loads a prospective local member through the host's authenticated
    /// public-user view. This is intentionally separate from `AsLocalTeam`:
    /// the target is not a roster member yet and therefore has no team token.
    pub fn load_and_pin_open_local_user(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        target: &EntityId,
    ) -> Result<VerifiedUserState> {
        self.load_and_pin_other_local_user_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
            target,
            |start, name| encode_load_user_chain_open_host_request(target.as_bytes(), start, name),
        )
    }

    fn load_and_pin_other_local_user_with_material(
        &self,
        host: &PinnedHost,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        target: &EntityId,
        encode_request: impl Fn(u64, Option<(&[u8], u64)>) -> foks_rpc::Result<Vec<u8>>,
    ) -> Result<VerifiedUserState> {
        let _pinning = crate::pinning::span(&host.database_path);
        target.clone().require_type(ENTITY_USER)?;
        self.retry_chain_load(host, |current| {
            let (_, merkle) = self.advance_merkle_root(current)?;
            let prior = match self.pinned_user(current, target) {
                Ok(prior) => prior,
                Err(Error::Verify(
                    foks_verify::Error::PersistedMerkleEvidence
                    | foks_verify::Error::UserChainContinuity,
                )) => None,
                Err(error) => return Err(error),
            };
            let (start, name) = user_chain_cursor(prior.as_ref())?;
            let request = encode_request(start, name)?;
            let chain_bytes = self.call_with_material(
                current,
                &current.user,
                &request,
                auth_seed,
                certificate_chain,
            )?;
            let authenticated_roots =
                self.authenticate_user_chain_roots(current, &merkle, &chain_bytes)?;
            let verified = match prior.as_ref() {
                Some(prior) => verify_non_self_user_chain_increment(
                    &chain_bytes,
                    prior,
                    target,
                    current.host_id(),
                    &authenticated_roots,
                    &merkle,
                )?,
                None => verify_non_self_user_chain(
                    &chain_bytes,
                    target,
                    current.host_id(),
                    &authenticated_roots,
                    &merkle,
                )?,
            };
            HardStateStore::open(&current.database_path)?
                .accept_verified_user(&verified.hard_state_snapshot()?)?;
            Ok(verified)
        })
    }

    fn fetch_puk_parcel(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        role: Role,
    ) -> Result<Vec<u8>> {
        let device = credential.public_material()?;
        let request = encode_get_puk_for_role_request(role, device.id.as_bytes())?;
        self.call(host, &host.user, &request, Some(credential))
    }

    /// Loads and authenticates the complete PUK history visible at `role`.
    ///
    /// Owner-side background rotation uses this to assemble every role in a
    /// stale prefix instead of assuming the authentication role parcel
    /// contains lower-role histories.
    pub fn load_puks_for_role(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        verified: &VerifiedUserState,
        role: Role,
    ) -> Result<Vec<UserPrivateKey>> {
        let parcel = PukParcel::decode(&self.fetch_puk_parcel(host, credential, role)?)?;
        let role_key = verified.shared_key(role).ok_or(Error::UserBinding(
            "requested PUK role is not in the user chain",
        ))?;
        let senders = verified.device_history(&parcel.sender)?;
        let clear = senders
            .iter()
            .rev()
            .find_map(|sender| {
                open_puk_parcel_with_for_role(
                    &parcel,
                    &foks_crypto::SoftwareDecapsulator::for_kind(
                        &credential.seed,
                        credential.key_kind,
                    )
                    .ok()?,
                    &sender.hepk,
                    &role_key.verify_key,
                    &role_key.hepk,
                    role_key.generation,
                    host.host_id(),
                    role,
                )
                .ok()
            })
            .ok_or(Error::KeyBinding(
                "no authenticated historical sender opens the PUK parcel",
            ))?;
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

    /// Loads and authenticates the complete PUK history visible at `role`
    /// while using a hardware parent for parcel decapsulation.
    ///
    /// The parcel returned during Yubi authentication covers only the
    /// parent's role. Owner-side stale-PUK rotation must load each lower role
    /// independently so it never substitutes one role's seed for another.
    pub fn load_puks_for_role_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        verified: &VerifiedUserState,
        role: Role,
    ) -> Result<Vec<UserPrivateKey>> {
        let request =
            encode_get_puk_for_role_request(role, credential.parent.entity_id().as_bytes())?;
        let parcel = PukParcel::decode(&self.call_with_material(
            host,
            &host.user,
            &request,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?)?;
        let role_key = verified.shared_key(role).ok_or(Error::UserBinding(
            "requested PUK role is not in the user chain",
        ))?;
        let senders = verified.device_history(&parcel.sender)?;
        let clear = senders
            .iter()
            .rev()
            .find_map(|sender| {
                open_puk_parcel_with_for_role(
                    &parcel,
                    credential.parent,
                    &sender.hepk,
                    &role_key.verify_key,
                    &role_key.hepk,
                    role_key.generation,
                    host.host_id(),
                    role,
                )
                .ok()
            })
            .ok_or(Error::KeyBinding(
                "no authenticated historical sender opens the PUK parcel",
            ))?;
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
        let _pinning = crate::pinning::span(&pinned.database_path);
        let mut store = HardStateStore::open(&pinned.database_path)?;
        // The anchor this advance is verified against is the one the pinned
        // host was restored with, so both come from one replay of the stored
        // evidence rather than two.
        let host = crate::host::restored_host(&store, &pinned.lookup_name, &pinned.database_path)?;
        if host.host.host_id.as_bytes() != pinned.host_id.as_bytes() {
            return Err(Error::HostBinding("stored HostID changed"));
        }
        let latest_bytes = self.call_after_vhost_selection(
            pinned,
            &pinned.merkle_query,
            &encode_merkle_select_vhost_request(pinned.host_id())?,
            &encode_get_current_merkle_root_signed_request(pinned.host_id(), 1)?,
        )?;
        let signed = foks_proto::SignedBlob::decode(&latest_bytes)
            .map_err(|_| Error::HostBinding("current Merkle root is not signed"))?;
        let latest = foks_proto::MerkleRoot::decode(&signed.inner)?;
        let history = merkle_history_requirements(latest.epoch, host.anchor.epoch())?;
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
        let verified = verify_signed_merkle_advance(
            &host.anchor,
            &latest_bytes,
            &historical_bytes,
            &host.chain_bytes,
            &HostchainTail {
                seqno: host.chain_seqno,
                hash: host.chain_tail_hash,
            },
        )?;
        let acceptance =
            store.accept_verified_merkle_root(host.host.host_id.as_bytes(), verified.snapshot())?;
        Ok((acceptance, verified))
    }

    pub(crate) fn authenticate_user_chain_roots(
        &self,
        host: &PinnedHost,
        latest: &VerifiedMerkleAdvance,
        chain_bytes: &[u8],
    ) -> Result<AuthenticatedMerkleRoots> {
        let chain = UserChain::decode(chain_bytes)?;
        if chain.merkle.root().epoch < latest.root().epoch {
            return Err(Error::UserBinding(
                "chain response is older than the authenticated Merkle root",
            ));
        }
        let anchor = if chain.merkle.root().epoch > latest.root().epoch {
            let (_, advanced) = self.advance_merkle_root(host)?;
            if advanced.root().epoch < chain.merkle.root().epoch {
                return Err(Error::UserBinding(
                    "user chain references an unauthenticated future Merkle root",
                ));
            }
            advanced
        } else {
            latest.clone()
        };
        let mut targets = user_chain_root_epochs(chain_bytes)?;
        targets.push(chain.merkle.root().epoch);
        let targets = targets
            .into_iter()
            .filter(|epoch| !anchor.authenticated_roots().contains_epoch(*epoch))
            .collect::<Vec<_>>();
        self.authenticate_chain_roots(host, &anchor, targets, Error::UserBinding)
    }

    pub(crate) fn authenticate_team_chain_roots(
        &self,
        host: &PinnedHost,
        latest: &VerifiedMerkleAdvance,
        chain_bytes: &[u8],
    ) -> Result<AuthenticatedMerkleRoots> {
        let chain = TeamChain::decode(chain_bytes)?;
        if chain.merkle.root().epoch < latest.root().epoch {
            return Err(Error::TeamBinding(
                "chain response is older than the authenticated Merkle root",
            ));
        }
        let anchor = if chain.merkle.root().epoch > latest.root().epoch {
            let (_, advanced) = self.advance_merkle_root(host)?;
            if advanced.root().epoch < chain.merkle.root().epoch {
                return Err(Error::TeamBinding(
                    "team chain references an unauthenticated future Merkle root",
                ));
            }
            advanced
        } else {
            latest.clone()
        };
        let mut targets = team_chain_root_epochs(chain_bytes)?;
        targets.push(chain.merkle.root().epoch);
        let targets = targets
            .into_iter()
            .filter(|epoch| !anchor.authenticated_roots().contains_epoch(*epoch))
            .collect();
        self.authenticate_chain_roots(host, &anchor, targets, Error::TeamBinding)
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
        let mut authenticated = latest.authenticated_roots().clone();
        for batch in historical_root_batches(latest.root().epoch, targets, binding)? {
            let roots = self.authenticate_chain_root_batch(
                host,
                latest,
                &batch.targets,
                &batch.full_epochs,
                &batch.hash_epochs,
            )?;
            authenticated.merge(&roots)?;
        }
        Ok(authenticated)
    }

    fn authenticate_chain_root_batch(
        &self,
        host: &PinnedHost,
        latest: &VerifiedMerkleAdvance,
        targets: &[u64],
        full_epochs: &std::collections::BTreeSet<u64>,
        hash_epochs: &std::collections::BTreeSet<u64>,
    ) -> Result<AuthenticatedMerkleRoots> {
        let full_epochs = full_epochs.iter().copied().collect::<Vec<_>>();
        let hash_epochs = hash_epochs.iter().copied().collect::<Vec<_>>();
        let mut roots = std::collections::BTreeMap::new();
        let mut hashes = std::collections::BTreeMap::new();
        for (requested_full, requested_hashes) in
            historical_epoch_requests(&full_epochs, &hash_epochs)
        {
            let response = self.call_after_vhost_selection(
                host,
                &host.merkle_query,
                &encode_merkle_select_vhost_request(host.host_id())?,
                &encode_get_historical_merkle_roots_request(
                    host.host_id(),
                    &requested_full,
                    &requested_hashes,
                    1,
                )?,
            )?;
            let response = foks_proto::HistoricalMerkleRoots::decode(&response)?;
            if response.roots.len() != requested_full.len()
                || response.hashes.len() != requested_hashes.len()
            {
                return Err(foks_verify::Error::MerkleHistoryShape.into());
            }
            for (epoch, root) in requested_full.into_iter().zip(response.roots) {
                if roots.insert(epoch, root).is_some() {
                    return Err(foks_verify::Error::MerkleHistoryShape.into());
                }
            }
            for (epoch, hash) in requested_hashes.into_iter().zip(response.hashes) {
                if hashes.insert(epoch, hash).is_some() {
                    return Err(foks_verify::Error::MerkleHistoryShape.into());
                }
            }
        }
        let historical = foks_proto::HistoricalMerkleRoots {
            roots: full_epochs
                .iter()
                .map(|epoch| {
                    roots
                        .remove(epoch)
                        .ok_or(foks_verify::Error::MerkleHistoryShape)
                })
                .collect::<std::result::Result<Vec<_>, _>>()?,
            hashes: hash_epochs
                .iter()
                .map(|epoch| {
                    hashes
                        .remove(epoch)
                        .ok_or(foks_verify::Error::MerkleHistoryShape)
                })
                .collect::<std::result::Result<Vec<_>, _>>()?,
        }
        .encoded()?;
        Ok(authenticate_historical_roots_from_latest(
            latest,
            targets,
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
        self.retry_chain_load(host, |current| {
            self.authenticate_and_pin_once(current, credential)
        })
    }

    fn authenticate_and_pin_once(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<AuthenticatedUserOutcome> {
        let _pinning = crate::pinning::span(&host.database_path);
        let derived = credential.public_material()?;
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
                &merkle,
            )?,
            None => verify_user_chain(
                &chain_bytes,
                &credential.uid,
                &host.host_id,
                &authenticated_roots,
                &merkle,
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
        let role_key = verified
            .shared_key(role)
            .ok_or(Error::UserBinding("device role has no PUK"))?;
        let senders = verified.device_history(&parcel.sender)?;
        let clear = senders
            .iter()
            .rev()
            .find_map(|sender| {
                open_puk_parcel_with_for_role(
                    &parcel,
                    &foks_crypto::SoftwareDecapsulator::for_kind(
                        &credential.seed,
                        credential.key_kind,
                    )
                    .ok()?,
                    &sender.hepk,
                    &role_key.verify_key,
                    &role_key.hepk,
                    role_key.generation,
                    &host.host_id,
                    role,
                )
                .ok()
            })
            .ok_or(Error::KeyBinding(
                "no authenticated historical sender opens the PUK parcel",
            ))?;
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

    /// Authenticates whichever credential form the caller holds. This is the
    /// single entry point used by federation so that a hardware-only
    /// administrator reaches exactly the same verification as a software one.
    pub fn authenticate_credential_and_pin(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
    ) -> Result<AuthenticatedUserOutcome> {
        match credential {
            FederationCredential::Software(credential) => {
                self.authenticate_and_pin(host, credential)
            }
            FederationCredential::Yubi(credential) => {
                self.authenticate_yubi_and_pin(host, credential)
            }
        }
    }

    /// Authenticates a Yubi-backed device using its software subkey for mTLS
    /// and the hardware parent for PUK decapsulation.
    pub fn authenticate_yubi_and_pin(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<AuthenticatedUserOutcome> {
        self.retry_chain_load(host, |current| {
            self.authenticate_yubi_and_pin_once(current, credential)
        })
    }

    fn authenticate_yubi_and_pin_once(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<AuthenticatedUserOutcome> {
        let _pinning = crate::pinning::span(&host.database_path);
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
                &merkle,
            )?,
            None => verify_user_chain(
                &chain_bytes,
                &credential.uid,
                &host.host_id,
                &authenticated_roots,
                &merkle,
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
        let role_key = verified
            .shared_key(role)
            .ok_or(Error::UserBinding("Yubi parent role has no PUK"))?;
        let senders = verified.device_history(&parcel.sender)?;
        let clear = senders
            .iter()
            .rev()
            .find_map(|sender| {
                open_puk_parcel_with_for_role(
                    &parcel,
                    credential.parent,
                    &sender.hepk,
                    &role_key.verify_key,
                    &role_key.hepk,
                    role_key.generation,
                    &host.host_id,
                    role,
                )
                .ok()
            })
            .ok_or(Error::KeyBinding(
                "no authenticated historical sender opens the PUK parcel",
            ))?;
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

    pub(crate) fn retry_chain_load<T>(
        &self,
        host: &PinnedHost,
        mut operation: impl FnMut(&PinnedHost) -> Result<T>,
    ) -> Result<T> {
        const ATTEMPTS: usize = 3;
        let mut current = host.clone();
        for attempt in 0..ATTEMPTS {
            match operation(&current) {
                Ok(value) => return Ok(value),
                Err(error) if attempt + 1 < ATTEMPTS && retryable_chain_load_error(&error) => {
                    std::thread::sleep(std::time::Duration::from_millis(10_u64 << attempt));
                    current = self
                        .probe_and_pin_host_id(
                            &current.probe,
                            &current.host_id,
                            &current.database_path,
                        )?
                        .pinned;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded retry loop always returns")
    }
}

#[derive(Debug, Eq, PartialEq)]
struct HistoricalRootBatch {
    targets: Vec<u64>,
    full_epochs: std::collections::BTreeSet<u64>,
    hash_epochs: std::collections::BTreeSet<u64>,
}

fn historical_root_batches(
    latest_epoch: u64,
    targets: Vec<u64>,
    binding: fn(&'static str) -> Error,
) -> Result<Vec<HistoricalRootBatch>> {
    const MAXIMUM_TARGETS_PER_BATCH: usize = 64;

    let mut batches = Vec::new();
    let mut batch = HistoricalRootBatch {
        targets: Vec::new(),
        full_epochs: std::collections::BTreeSet::new(),
        hash_epochs: std::collections::BTreeSet::new(),
    };
    for target in targets
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    {
        if target == 0 || target >= latest_epoch {
            return Err(binding(
                "user chain references an unauthenticated future Merkle root",
            ));
        }
        let requirements = merkle_history_requirements(latest_epoch, target)?;
        let mut target_full = requirements
            .full_roots
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        target_full.insert(target);
        let target_hashes = requirements
            .hashes
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        if batch.targets.len() == MAXIMUM_TARGETS_PER_BATCH {
            batches.push(batch);
            batch = HistoricalRootBatch {
                targets: Vec::new(),
                full_epochs: std::collections::BTreeSet::new(),
                hash_epochs: std::collections::BTreeSet::new(),
            };
        }
        batch.targets.push(target);
        batch.full_epochs.extend(target_full);
        batch.hash_epochs.extend(target_hashes);
    }
    if !batch.targets.is_empty() {
        batches.push(batch);
    }
    Ok(batches)
}

fn historical_epoch_requests(
    full_epochs: &[u64],
    hash_epochs: &[u64],
) -> Vec<(Vec<u64>, Vec<u64>)> {
    const MAXIMUM_EPOCHS_PER_REQUEST: usize = 64;

    let count = full_epochs
        .len()
        .max(hash_epochs.len())
        .div_ceil(MAXIMUM_EPOCHS_PER_REQUEST);
    (0..count)
        .map(|index| {
            let start = index * MAXIMUM_EPOCHS_PER_REQUEST;
            let full_end = (start + MAXIMUM_EPOCHS_PER_REQUEST).min(full_epochs.len());
            let hash_end = (start + MAXIMUM_EPOCHS_PER_REQUEST).min(hash_epochs.len());
            (
                full_epochs
                    .get(start..full_end)
                    .unwrap_or_default()
                    .to_vec(),
                hash_epochs
                    .get(start..hash_end)
                    .unwrap_or_default()
                    .to_vec(),
            )
        })
        .collect()
}

pub(crate) fn retryable_chain_load_error(error: &Error) -> bool {
    match error {
        Error::UserBinding(message) | Error::TeamBinding(message) => {
            matches!(
                *message,
                "user chain references an unauthenticated future Merkle root"
                    | "chain response is not anchored at the latest Merkle root"
            )
        }
        Error::Verify(
            foks_verify::Error::UntrustedUserRoot
            | foks_verify::Error::MerkleHostchainMismatch
            | foks_verify::Error::MissingDelegatedKey("Merkle signer")
            | foks_verify::Error::DelegatedSignature("Merkle signer"),
        ) => true,
        Error::GenericChainRootChanged
        | Error::Database(foks_client_db::Error::MerkleRollback { .. }) => true,
        _ => false,
    }
}

fn current_milliseconds() -> Result<u64> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::CredentialBinding("system clock precedes Unix epoch"))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| Error::CredentialBinding("system clock timestamp overflow"))
}

#[cfg(test)]
mod tests {
    use super::{
        historical_epoch_requests, historical_root_batches, retryable_chain_load_error,
        DeviceCredential, EntityId, Error, FederationCredential, SecretSeed, YubiCredential,
    };

    /// Stand-in for a hardware parent. Only the identity surface is exercised
    /// here; nothing in this test performs a hardware operation.
    struct StubYubiParent {
        id: EntityId,
        hepk: foks_proto::Hepk,
    }

    impl foks_crypto::HybridSecretDecapsulator for StubYubiParent {
        fn entity_id(&self) -> &EntityId {
            &self.id
        }

        fn hepk(&self) -> &foks_proto::Hepk {
            &self.hepk
        }

        fn derive_dh_shared(
            &self,
            _peer: &foks_proto::DhPublicKey,
        ) -> foks_crypto::Result<zeroize::Zeroizing<[u8; 32]>> {
            unreachable!("identity-only stub")
        }

        fn decapsulate_mlkem768(
            &self,
            _ciphertext: &[u8],
        ) -> foks_crypto::Result<zeroize::Zeroizing<[u8; 32]>> {
            unreachable!("identity-only stub")
        }
    }

    impl foks_crypto::YubiDevice for StubYubiParent {
        fn pq_key_id(&self) -> [u8; 32] {
            [0x33; 32]
        }

        fn sign_sha512_256(&self, _digest: &[u8; 32]) -> foks_crypto::Result<Vec<u8>> {
            unreachable!("identity-only stub")
        }
    }

    /// The transport material a federation credential exposes must never be a
    /// hardware secret, and the acting chain device must be the Yubi parent
    /// rather than the delegated mTLS subkey. Getting either backwards would
    /// either leak a hardware key into an ordinary buffer or let a caller
    /// present the subkey where the parent's authority is required.
    #[test]
    fn federation_credentials_expose_transport_material_and_the_acting_device() {
        let uid =
            EntityId::from_bytes([vec![foks_proto::ENTITY_USER], vec![0x41; 32]].concat()).unwrap();
        let device = DeviceCredential {
            key_kind: crate::SoftwareKeyKind::Device,
            uid: uid.clone(),
            seed: SecretSeed::new([0x51; 32]),
            certificate_chain: vec![vec![1, 2, 3]],
        };
        let software = FederationCredential::Software(&device);
        assert_eq!(software.uid(), &uid);
        assert!(!software.is_hardware());
        let (seed, chain) = software.transport();
        assert_eq!(seed.as_slice(), device.seed.as_slice());
        assert_eq!(chain, device.certificate_chain.as_slice());
        assert_eq!(
            software.device_id().unwrap(),
            device.public_material().unwrap().id
        );

        let parent_id =
            EntityId::from_bytes([vec![foks_proto::ENTITY_YUBI], vec![0x61; 33]].concat()).unwrap();
        let parent = StubYubiParent {
            id: parent_id.clone(),
            hepk: foks_crypto::derive_device_public(&SecretSeed::new([0x71; 32]))
                .unwrap()
                .hepk,
        };
        let yubi = YubiCredential {
            uid: uid.clone(),
            parent: &parent,
            subkey_seed: SecretSeed::new([0x81; 32]),
            certificate_chain: vec![vec![4, 5, 6]],
        };
        let hardware = FederationCredential::Yubi(&yubi);
        assert_eq!(hardware.uid(), &uid);
        assert!(hardware.is_hardware());
        let (seed, chain) = hardware.transport();
        assert_eq!(
            seed.as_slice(),
            yubi.subkey_seed.as_slice(),
            "mTLS must use the delegated subkey, never a hardware secret"
        );
        assert_eq!(chain, yubi.certificate_chain.as_slice());
        assert_eq!(
            hardware.device_id().unwrap(),
            parent_id,
            "the acting chain device is the Yubi parent, not its subkey"
        );
    }

    #[test]
    fn historical_root_requests_are_batched_without_dropping_targets() {
        let targets = (1..=100).collect::<Vec<_>>();
        let batches = historical_root_batches(10_000, targets.clone(), Error::UserBinding).unwrap();
        assert!(batches.len() > 1);
        assert!(batches.iter().all(|batch| batch.targets.len() <= 64));
        for batch in &batches {
            let full = batch.full_epochs.iter().copied().collect::<Vec<_>>();
            let hashes = batch.hash_epochs.iter().copied().collect::<Vec<_>>();
            assert!(historical_epoch_requests(&full, &hashes)
                .iter()
                .all(|(full, hashes)| full.len() <= 64 && hashes.len() <= 64));
        }
        assert_eq!(
            batches
                .into_iter()
                .flat_map(|batch| batch.targets)
                .collect::<Vec<_>>(),
            targets
        );
    }

    #[test]
    fn historical_root_batches_reject_future_roots() {
        assert!(matches!(
            historical_root_batches(100, vec![100], Error::TeamBinding),
            Err(Error::TeamBinding(
                "user chain references an unauthenticated future Merkle root"
            ))
        ));
    }

    #[test]
    fn only_trust_refresh_races_are_retried() {
        assert!(retryable_chain_load_error(&Error::GenericChainRootChanged));
        assert!(retryable_chain_load_error(&Error::UserBinding(
            "user chain references an unauthenticated future Merkle root"
        )));
        assert!(retryable_chain_load_error(&Error::TeamBinding(
            "chain response is not anchored at the latest Merkle root"
        )));
        assert!(retryable_chain_load_error(&Error::Verify(
            foks_verify::Error::MerkleHostchainMismatch
        )));
        assert!(!retryable_chain_load_error(&Error::Verify(
            foks_verify::Error::UserChainContinuity
        )));
        assert!(retryable_chain_load_error(&Error::Database(
            foks_client_db::Error::MerkleRollback {
                stored: 2,
                received: 1,
            }
        )));
    }
}
