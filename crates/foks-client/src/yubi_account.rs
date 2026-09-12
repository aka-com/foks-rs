//! Yubi-backed account creation and crash reconciliation.

use std::path::Path;
use std::time::Duration;

use foks_crypto::{
    derive_shared_verify_key, derive_subkey_id, make_yubi_eldest_link, seal_initial_yubi_puk_box,
    seal_yubi_subkey_box, InitialPukBoxRandomness, SoftwareEldestInput, YubiDevice,
    YubiEldestMaterial, YubiSubkeyBoxRandomness,
};
use foks_proto::{
    DecodedSignupArgument, DeviceLabel, DeviceLabelNameAndCommitmentKey, DeviceType, EntityId,
    HybridBox, InviteCode, PassphraseUpdateArgument, Role, SecretSeed, SharedKeyBoxSet, TreeRoot,
    UsernameReservation, YubiSignupArgument, YubiSlotAndPqKeyId, ENTITY_PUK_VERIFY, ENTITY_USER,
};
use foks_rpc::{encode_registration_select_vhost_request, encode_yubi_signup_request_at};
use zeroize::{Zeroize as _, Zeroizing};

use crate::{
    fix_device_name, normalize_device_name, normalize_username, now_milliseconds, prefixed_hash,
    random_bytes, AuthenticatedUserOutcome, Error, FoksClient, HardStateStore,
    KvDirectoryProjection, MutationCoordinator, MutationDraft, MutationKind, MutationOperation,
    MutationState, PinnedHost, ProtectedMutationStore, Result, YubiCredential,
};

const SIGNUP_REQUEST_HASH_TYPE_ID: u64 = 0x8f4b_8ab7_464f_4b53;
const YUBI_SIGNUP_MATERIAL_VERSION: u8 = 2;

/// Secret signup state that must be protected before any registration write.
/// The hardware locator is owned by the application credential store and is
/// deliberately not copied into the hard-state database or mutation journal.
pub struct YubiAccountSecrets {
    pub subkey_seed: SecretSeed,
    pub puk_seed: SecretSeed,
    self_token: Zeroizing<[u8; 17]>,
}

impl YubiAccountSecrets {
    pub fn new(subkey_seed: SecretSeed, puk_seed: SecretSeed, self_token: [u8; 17]) -> Self {
        Self {
            subkey_seed,
            puk_seed,
            self_token: Zeroizing::new(self_token),
        }
    }
}

pub struct YubiAccountRequest {
    pub username_utf8: String,
    pub device_name: String,
    pub invite_code: InviteCode,
    pub email: String,
    pub passphrase: Option<foks_crypto::Passphrase>,
    pub pq_hint: YubiSlotAndPqKeyId,
}

pub struct PreparedYubiAccount {
    pub operation_id: [u8; 16],
    pub normalized_username: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub reservation: UsernameReservation,
    pub eldest: YubiEldestMaterial,
    pub puk_box: SharedKeyBoxSet,
    pub subkey_box: HybridBox,
    pub username_commitment_key: [u8; 16],
    pub device_name: DeviceLabelNameAndCommitmentKey,
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub passphrase: Option<PassphraseUpdateArgument>,
    pub pq_hint: YubiSlotAndPqKeyId,
}

pub struct CreatedYubiAccount<'a> {
    pub operation_id: [u8; 16],
    pub credential: YubiCredential<'a>,
    pub authenticated: AuthenticatedUserOutcome,
    pub kv_projection: Vec<KvDirectoryProjection>,
}

impl FoksClient {
    pub fn prepare_yubi_account(
        &self,
        host: &PinnedHost,
        merkle: &foks_verify::VerifiedMerkleAdvance,
        parent: &dyn YubiDevice,
        request: &YubiAccountRequest,
        reservation: UsernameReservation,
        secrets: &YubiAccountSecrets,
    ) -> Result<PreparedYubiAccount> {
        if parent.entity_id().entity_type() != foks_proto::ENTITY_YUBI
            || request.pq_hint.id != parent.pq_key_id()
        {
            return Err(Error::AccountRequest(
                "Yubi signup hint does not match the selected hardware parent",
            ));
        }
        let normalized_username = normalize_username(request.username_utf8.as_bytes()).ok_or(
            Error::AccountRequest("username is not valid after normalization"),
        )?;
        let display_device_name = fix_device_name(&request.device_name);
        let normalized_device_name = normalize_device_name(display_device_name.as_bytes()).ok_or(
            Error::AccountRequest("device name is not valid after normalization"),
        )?;
        if merkle.root().hostchain.seqno == 0 {
            return Err(Error::AccountRequest(
                "Merkle root has no host-chain binding",
            ));
        }
        let operation_id = random_bytes()?;
        let username_commitment_key = random_bytes()?;
        let next_tree_location = random_bytes()?;
        let subchain_tree_location = random_bytes()?;
        let device_name = DeviceLabelNameAndCommitmentKey {
            label: DeviceLabel {
                device_type: DeviceType::YubiKey,
                normalized_name: normalized_device_name,
                serial: 1,
            },
            normalization_version: 0,
            display_name: display_device_name.into_bytes(),
            commitment_key: random_bytes()?,
        };
        let tree_root = TreeRoot {
            epoch: merkle.snapshot().epoch(),
            hash: merkle.snapshot().root_hash(),
        };
        let eldest = make_yubi_eldest_link(
            &SoftwareEldestInput {
                host: host.host_id(),
                root: &tree_root,
                time: now_milliseconds()?,
                next_tree_location,
                subchain_tree_location,
                normalized_username: &normalized_username,
                username_sequence: reservation.sequence,
                username_commitment_key,
                device_name: &device_name,
            },
            parent,
            &secrets.subkey_seed,
            &secrets.puk_seed,
        )?;
        let puk_box = seal_initial_yubi_puk_box(
            host.host_id(),
            parent,
            &secrets.puk_seed,
            InitialPukBoxRandomness {
                box_id: random_bytes()?,
                kem_message: random_bytes()?,
                nonce: random_bytes()?,
            },
        )?;
        let subkey_box = seal_yubi_subkey_box(
            parent,
            &secrets.subkey_seed,
            YubiSubkeyBoxRandomness {
                kem_message: random_bytes()?,
                nonce: random_bytes()?,
            },
        )?;
        let passphrase = request
            .passphrase
            .as_ref()
            .map(|passphrase| {
                foks_crypto::create_passphrase_enrollment(
                    passphrase,
                    &eldest.uid,
                    host.host_id(),
                    &secrets.puk_seed,
                    1,
                    foks_proto::StretchVersion::V1,
                )
                .map(|update| update.argument())
            })
            .transpose()?;
        Ok(PreparedYubiAccount {
            operation_id,
            normalized_username,
            username_utf8: request.username_utf8.as_bytes().to_vec(),
            reservation,
            eldest,
            puk_box,
            subkey_box,
            username_commitment_key,
            device_name,
            next_tree_location,
            subchain_tree_location,
            passphrase,
            pq_hint: request.pq_hint.clone(),
        })
    }

    fn yubi_signup_request(
        &self,
        prepared: &PreparedYubiAccount,
        request: &YubiAccountRequest,
        self_token: [u8; 17],
    ) -> Result<Zeroizing<Vec<u8>>> {
        Ok(Zeroizing::new(encode_yubi_signup_request_at(
            &YubiSignupArgument {
                username_utf8: &prepared.username_utf8,
                reservation: &prepared.reservation,
                link: &prepared.eldest.link,
                puk_box: &prepared.puk_box,
                username_commitment_key: prepared.username_commitment_key,
                device_name: &prepared.device_name,
                next_tree_location: prepared.next_tree_location,
                invite_code: &request.invite_code,
                email: request.email.as_bytes(),
                subchain_tree_location: prepared.subchain_tree_location,
                self_token,
                puk_hepk: &prepared.eldest.puk.hepk,
                device_hepk: &prepared.eldest.device.hepk,
                passphrase: prepared.passphrase.as_ref(),
                subkey_box: &prepared.subkey_box,
                yubi_pq_hint: &prepared.pq_hint,
            },
            1,
        )?))
    }

    pub fn create_yubi_account<'a>(
        &self,
        host: &PinnedHost,
        parent: &'a dyn YubiDevice,
        request: YubiAccountRequest,
        secrets: YubiAccountSecrets,
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedYubiAccount<'a>> {
        self.create_yubi_account_authorized(
            host,
            parent,
            request,
            secrets,
            soft_database_path,
            protected_store,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_yubi_account_with_sso<'a>(
        &self,
        host: &PinnedHost,
        parent: &'a dyn YubiDevice,
        request: YubiAccountRequest,
        secrets: YubiAccountSecrets,
        soft_database_path: &Path,
        authorization: &crate::SsoSignupAuthorization,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedYubiAccount<'a>> {
        self.create_yubi_account_authorized(
            host,
            parent,
            request,
            secrets,
            soft_database_path,
            protected_store,
            Some(authorization),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_yubi_account_authorized<'a>(
        &self,
        host: &PinnedHost,
        parent: &'a dyn YubiDevice,
        request: YubiAccountRequest,
        secrets: YubiAccountSecrets,
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
        authorization: Option<&crate::SsoSignupAuthorization>,
    ) -> Result<CreatedYubiAccount<'a>> {
        self.check_invite_code(host, &request.invite_code)?;
        let reservation = if let Some(auth) = authorization {
            auth.validate(self, host)?;
            if request.username_utf8 != auth.username {
                return Err(Error::Sso(
                    "signup username must match the provider reservation",
                ));
            }
            auth.reservation.clone()
        } else {
            self.reserve_username(host, &request.username_utf8)?
        };
        let (_, merkle) = self.advance_merkle_root(host)?;
        let prepared =
            self.prepare_yubi_account(host, &merkle, parent, &request, reservation, &secrets)?;
        let signup_request = self.yubi_signup_request(&prepared, &request, *secrets.self_token)?;
        let signup_request = if let Some(auth) = authorization {
            if prepared.eldest.uid != auth.uid || prepared.eldest.device.id != auth.device {
                return Err(Error::Sso("signup keys do not match OAuth binding"));
            }
            Zeroizing::new(foks_rpc::attach_signup_sso(&signup_request, &auth.args)?)
        } else {
            signup_request
        };
        let request_hash = prefixed_hash(SIGNUP_REQUEST_HASH_TYPE_ID, &signup_request);
        let material =
            encode_yubi_signup_material(&secrets, &prepared.normalized_username, &signup_request)?;
        let draft = MutationDraft {
            operation_id: prepared.operation_id,
            kind: MutationKind::Signup,
            host_id: host.host_id().as_bytes().to_vec(),
            scope_id: prepared.eldest.device.id.as_bytes().to_vec(),
            subject_id: prepared.eldest.uid.as_bytes().to_vec(),
            expected_version: None,
            request_hash,
        };
        let mut coordinator = MutationCoordinator::new(&host.database_path, protected_store);
        let operation = if let Some(auth) = authorization {
            coordinator.prepare_sso_signup(draft, material, auth.flow_id)?
        } else {
            coordinator.prepare(draft, material)?
        };
        let created = self.submit_or_reconcile_yubi_account(
            host,
            parent,
            operation,
            Some(signup_request),
            prepared.normalized_username,
            secrets,
            soft_database_path,
            protected_store,
        )?;
        if let Some(passphrase) = request.passphrase.as_ref() {
            self.verify_passphrase_yubi(host, &created.credential, passphrase)?;
        }
        if let Some(auth) = authorization {
            HardStateStore::open(&host.database_path)?.sso_transition(
                &auth.flow_id,
                foks_client_db::SsoFlowState::Binding,
                foks_client_db::SsoFlowState::Complete,
                None,
            )?;
            self.sso_progress(host, auth.flow_id, protected_store)?;
        }
        Ok(created)
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_or_reconcile_yubi_account<'a>(
        &self,
        host: &PinnedHost,
        parent: &'a dyn YubiDevice,
        operation: MutationOperation,
        prepared_request: Option<Zeroizing<Vec<u8>>>,
        expected_username: Vec<u8>,
        secrets: YubiAccountSecrets,
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedYubiAccount<'a>> {
        validate_yubi_signup_operation_binding(host, parent, &operation, &secrets)?;
        let signup_request_was_available = prepared_request.is_some();
        let signup_passphrase = prepared_request
            .as_deref()
            .map(|request| yubi_signup_passphrase_from_request(request.as_slice()))
            .transpose()?
            .flatten();
        let already_verified = matches!(
            operation.state,
            MutationState::RemoteVerified | MutationState::Finalized
        );
        if operation.state == MutationState::Prepared {
            let request = prepared_request.ok_or(Error::OperationBinding(
                "prepared Yubi signup is missing its exact request",
            ))?;
            if prefixed_hash(SIGNUP_REQUEST_HASH_TYPE_ID, &request) != operation.request_hash {
                return Err(Error::OperationBinding(
                    "Yubi signup request fingerprint changed",
                ));
            }
            MutationCoordinator::new(&host.database_path, protected_store)
                .begin_submission(&operation.operation_id)?;
            if let Err(error) = self.call_void_after_vhost_selection(
                host,
                &host.registration,
                &encode_registration_select_vhost_request(host.host_id())?,
                &request,
            ) {
                let mut coordinator =
                    MutationCoordinator::new(&host.database_path, protected_store);
                coordinator.submission_unknown(&operation.operation_id)?;
                return Err(error);
            }
        } else if !matches!(
            operation.state,
            MutationState::Submitting
                | MutationState::SubmissionUnknown
                | MutationState::RemoteVerified
                | MutationState::Finalized
        ) {
            return Err(Error::OperationBinding("Yubi signup operation is terminal"));
        }

        let uid = EntityId::from_bytes(operation.subject_id.clone())?;
        let certificate_chain =
            self.fetch_subkey_certificate_chain(host, &uid, &secrets.subkey_seed)?;
        let credential = YubiCredential {
            uid,
            parent,
            subkey_seed: secrets.subkey_seed,
            certificate_chain,
        };
        let mut authenticated = self.authenticate_new_yubi_account(host, &credential)?;
        let supplied_puk = secrets.puk_seed;
        if authenticated.verified.username() != expected_username
            || authenticated
                .current_puk()
                .is_none_or(|key| key.seed != supplied_puk)
        {
            return Err(Error::CredentialBinding(
                "created Yubi account does not match the protected PUK",
            ));
        }
        if let Some(passphrase) = signup_passphrase.as_ref() {
            self.bootstrap_user_settings_from_signup_yubi(host, &credential, passphrase)?;
            authenticated = self.authenticate_new_yubi_account(host, &credential)?;
        } else if signup_request_was_available {
            HardStateStore::open(&host.database_path)?.attest_user_has_no_passphrase(
                host.host_id().as_bytes(),
                credential.uid.as_bytes(),
            )?;
        }
        let kv_projection = {
            let mut session = self.user_kv_write_session_yubi(
                host,
                &credential,
                &authenticated.verified,
                &authenticated.puks,
                soft_database_path,
                protected_store,
            )?;
            session.ensure_root(Role::OWNER, Role::OWNER)?
        };
        if !already_verified {
            MutationCoordinator::new(&host.database_path, protected_store)
                .remote_verified(&operation.operation_id)?;
        }
        Ok(CreatedYubiAccount {
            operation_id: operation.operation_id,
            credential,
            authenticated,
            kv_projection,
        })
    }

    pub fn resume_yubi_account<'a>(
        &self,
        host: &PinnedHost,
        parent: &'a dyn YubiDevice,
        operation_id: [u8; 16],
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedYubiAccount<'a>> {
        let operation = HardStateStore::open(&host.database_path)?
            .mutation(&operation_id)?
            .ok_or(Error::AccountRequest(
                "Yubi signup operation is not recorded",
            ))?;
        let material = MutationCoordinator::new(&host.database_path, protected_store)
            .load_bound_material(&operation)?;
        let (secrets, expected_username, request) = decode_yubi_signup_material(material)?;
        self.submit_or_reconcile_yubi_account(
            host,
            parent,
            operation,
            Some(request),
            expected_username,
            secrets,
            soft_database_path,
            protected_store,
        )
    }

    /// Reconciles application-owned pending signup state after authoritative
    /// verification. `RemoteVerified` retains core material until the app
    /// stores the credential; `Finalized` is also accepted when cleanup was
    /// interrupted after that application commit.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_yubi_account_with_pending<'a>(
        &self,
        host: &PinnedHost,
        parent: &'a dyn YubiDevice,
        operation_id: [u8; 16],
        pending_username: &str,
        pending_secrets: YubiAccountSecrets,
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedYubiAccount<'a>> {
        let operation = HardStateStore::open(&host.database_path)?
            .mutation(&operation_id)?
            .ok_or(Error::AccountRequest(
                "Yubi signup operation is not recorded",
            ))?;
        let (secrets, expected_username, request) = if matches!(
            operation.state,
            MutationState::RemoteVerified | MutationState::Finalized
        ) {
            let expected_username = normalize_username(pending_username.as_bytes()).ok_or(
                Error::OperationBinding("pending Yubi signup username is invalid"),
            )?;
            (pending_secrets, expected_username, None)
        } else {
            let material = MutationCoordinator::new(&host.database_path, protected_store)
                .load_bound_material(&operation)?;
            let (secrets, expected_username, request) = decode_yubi_signup_material(material)?;
            (secrets, expected_username, Some(request))
        };
        self.submit_or_reconcile_yubi_account(
            host,
            parent,
            operation,
            request,
            expected_username,
            secrets,
            soft_database_path,
            protected_store,
        )
    }

    fn authenticate_new_yubi_account(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<AuthenticatedUserOutcome> {
        let mut last_error = None;
        for attempt in 0..40 {
            match self.authenticate_yubi_and_pin(host, credential) {
                Ok(outcome) => return Ok(outcome),
                Err(error @ Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1020, .. })) => {
                    return Err(error);
                }
                Err(error) => last_error = Some(error),
            }
            if attempt != 39 {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        Err(last_error.expect("Yubi account authentication loop executes at least once"))
    }
}

fn yubi_signup_passphrase_from_request(request: &[u8]) -> Result<Option<PassphraseUpdateArgument>> {
    let mut framed = std::io::Cursor::new(request);
    let call = foks_rpc::read_call(&mut framed, foks_rpc::DEFAULT_MAX_FRAME_LENGTH)
        .map_err(|_| Error::OperationBinding("persisted Yubi signup request is malformed"))?;
    if usize::try_from(framed.position()).ok() != Some(request.len())
        || call.protocol_id() != foks_rpc::REG_PROTOCOL_ID
        || call.method_position() != foks_rpc::REG_SIGNUP_METHOD_POSITION
    {
        return Err(Error::OperationBinding(
            "persisted Yubi signup request targets another route",
        ));
    }
    Ok(DecodedSignupArgument::decode(call.argument())?.passphrase)
}

fn encode_yubi_signup_material(
    secrets: &YubiAccountSecrets,
    normalized_username: &[u8],
    request: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    let username_len = u8::try_from(normalized_username.len())
        .ok()
        .filter(|length| (3..=25).contains(length))
        .ok_or(Error::AccountRequest(
            "normalized Yubi signup username is malformed",
        ))?;
    let request_len = u32::try_from(request.len())
        .map_err(|_| Error::AccountRequest("Yubi signup request is too large to protect"))?;
    let mut material = Zeroizing::new(Vec::with_capacity(
        87 + normalized_username.len() + request.len(),
    ));
    material.push(YUBI_SIGNUP_MATERIAL_VERSION);
    material.extend_from_slice(secrets.subkey_seed.as_slice());
    material.extend_from_slice(secrets.puk_seed.as_slice());
    material.extend_from_slice(secrets.self_token.as_slice());
    material.push(username_len);
    material.extend_from_slice(normalized_username);
    material.extend_from_slice(&request_len.to_be_bytes());
    material.extend_from_slice(request);
    Ok(material)
}

fn decode_yubi_signup_material(
    mut material: Zeroizing<Vec<u8>>,
) -> Result<(YubiAccountSecrets, Vec<u8>, Zeroizing<Vec<u8>>)> {
    const PREFIX: usize = 1 + 32 + 32 + 17 + 1;
    if material.len() < PREFIX + 4 || material[0] != YUBI_SIGNUP_MATERIAL_VERSION {
        return Err(Error::OperationBinding(
            "protected Yubi signup material is malformed",
        ));
    }
    let username_len = usize::from(material[82]);
    if !(3..=25).contains(&username_len) || material.len() < PREFIX + username_len + 4 {
        return Err(Error::OperationBinding(
            "protected Yubi signup username is malformed",
        ));
    }
    let request_length_offset = PREFIX + username_len;
    let request_len = u32::from_be_bytes(
        material[request_length_offset..request_length_offset + 4]
            .try_into()
            .map_err(|_| {
                Error::OperationBinding("protected Yubi signup request length is malformed")
            })?,
    ) as usize;
    let request_offset = request_length_offset + 4;
    if material.len() != request_offset + request_len {
        return Err(Error::OperationBinding(
            "protected Yubi signup request is truncated",
        ));
    }
    let subkey_seed = SecretSeed::from_slice(&material[1..33])?;
    let puk_seed = SecretSeed::from_slice(&material[33..65])?;
    let self_token: [u8; 17] = material[65..82]
        .try_into()
        .map_err(|_| Error::OperationBinding("protected Yubi signup token is malformed"))?;
    let expected_username = material[PREFIX..request_length_offset].to_vec();
    let request = Zeroizing::new(material[request_offset..].to_vec());
    material.zeroize();
    Ok((
        YubiAccountSecrets::new(subkey_seed, puk_seed, self_token),
        expected_username,
        request,
    ))
}

fn validate_yubi_signup_operation_binding(
    host: &PinnedHost,
    parent: &dyn YubiDevice,
    operation: &MutationOperation,
    secrets: &YubiAccountSecrets,
) -> Result<()> {
    if operation.kind != MutationKind::Signup || operation.host_id != host.host_id().as_bytes() {
        return Err(Error::OperationBinding(
            "Yubi signup operation belongs to another kind or host",
        ));
    }
    let puk_verify = derive_shared_verify_key(&secrets.puk_seed, ENTITY_PUK_VERIFY)?;
    let mut uid_bytes = puk_verify.as_bytes().to_vec();
    uid_bytes[0] = ENTITY_USER;
    let uid = EntityId::from_bytes(uid_bytes)?;
    let _ = derive_subkey_id(&secrets.subkey_seed)?;
    if operation.subject_id != uid.as_bytes() || operation.scope_id != parent.entity_id().as_bytes()
    {
        return Err(Error::OperationBinding(
            "Yubi signup operation does not match its protected credential",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_yubi_signup_material_round_trips_and_rejects_truncation() {
        let secrets =
            YubiAccountSecrets::new(SecretSeed::new([1; 32]), SecretSeed::new([2; 32]), [3; 17]);
        let encoded =
            encode_yubi_signup_material(&secrets, b"alice", b"exact signed request").unwrap();
        let (decoded, username, request) = decode_yubi_signup_material(encoded).unwrap();
        assert_eq!(decoded.subkey_seed.as_slice(), &[1; 32]);
        assert_eq!(decoded.puk_seed.as_slice(), &[2; 32]);
        assert_eq!(decoded.self_token.as_slice(), &[3; 17]);
        assert_eq!(username, b"alice");
        assert_eq!(request.as_slice(), b"exact signed request");

        assert!(decode_yubi_signup_material(Zeroizing::new(vec![2; 85])).is_err());
    }
}
