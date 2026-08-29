//! Software-account preparation, creation, and reconciliation.

use super::{
    derive_device_public, derive_shared_verify_key, encode_check_invite_code_request,
    encode_registration_select_vhost_request, encode_reserve_username_request_at,
    encode_signup_request_at, fix_device_name, make_software_eldest_link, normalize_device_name,
    normalize_username, now_microseconds, prefixed_hash, random_bytes, seal_initial_puk_box,
    AuthenticatedUserOutcome, DeviceCredential, DeviceLabel, DeviceLabelNameAndCommitmentKey,
    DeviceType, Duration, EntityId, Error, FoksClient, HardStateStore, InitialPukBoxRandomness,
    InviteCode, KvDirectoryProjection, MutationCoordinator, MutationDraft, MutationKind,
    MutationOperation, MutationState, PassphraseUpdateArgument, Path, PinnedHost,
    ProtectedMutationStore, Result, Role, SecretSeed, SharedKeyBoxSet, SoftwareEldestInput,
    SoftwareEldestMaterial, SoftwareSignupArgument, TreeRoot, UsernameReservation,
    VerifiedMerkleAdvance, Zeroizing, ENTITY_PUK_VERIFY, ENTITY_USER,
};

const SIGNUP_REQUEST_HASH_TYPE_ID: u64 = 0x8f4b_8ab7_464f_4b53;
const SIGNUP_MATERIAL_VERSION: u8 = 1;

/// Secret account-creation material. Callers must persist this in their
/// encrypted credential store before calling `create_software_account`; the
/// SQLite mutation journal intentionally cannot recover these values.
pub struct SoftwareAccountSecrets {
    pub device_seed: SecretSeed,
    pub puk_seed: SecretSeed,
    self_token: Zeroizing<[u8; 17]>,
}

impl SoftwareAccountSecrets {
    pub fn new(device_seed: SecretSeed, puk_seed: SecretSeed, self_token: [u8; 17]) -> Self {
        Self {
            device_seed,
            puk_seed,
            self_token: Zeroizing::new(self_token),
        }
    }
}

pub struct SoftwareAccountRequest {
    pub username_utf8: String,
    pub device_name: String,
    pub invite_code: InviteCode,
    pub email: String,
    pub passphrase: Option<foks_crypto::Passphrase>,
}

pub struct PreparedSoftwareAccount {
    pub operation_id: [u8; 16],
    pub normalized_username: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub reservation: UsernameReservation,
    pub eldest: SoftwareEldestMaterial,
    pub puk_box: SharedKeyBoxSet,
    pub username_commitment_key: [u8; 16],
    pub device_name: DeviceLabelNameAndCommitmentKey,
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub passphrase: Option<PassphraseUpdateArgument>,
}

pub struct CreatedSoftwareAccount {
    pub operation_id: [u8; 16],
    pub credential: DeviceCredential,
    pub authenticated: AuthenticatedUserOutcome,
    pub kv_projection: Vec<KvDirectoryProjection>,
}

impl FoksClient {
    pub fn check_invite_code(&self, host: &PinnedHost, code: &InviteCode) -> Result<()> {
        code.validate()?;
        self.call_void_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_check_invite_code_request(code)?,
        )
    }

    pub fn reserve_username(
        &self,
        host: &PinnedHost,
        username_utf8: &str,
    ) -> Result<UsernameReservation> {
        let normalized = normalize_username(username_utf8.as_bytes()).ok_or(
            Error::AccountRequest("username is not valid after normalization"),
        )?;
        let response = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_reserve_username_request_at(&normalized, 1)?,
        )?;
        let reservation = UsernameReservation::decode(&response)?;
        if reservation.sequence == 0 {
            return Err(Error::AccountRequest(
                "registration returned an invalid name sequence",
            ));
        }
        Ok(reservation)
    }

    pub fn prepare_software_account(
        &self,
        host: &PinnedHost,
        merkle: &VerifiedMerkleAdvance,
        request: &SoftwareAccountRequest,
        reservation: UsernameReservation,
        secrets: &SoftwareAccountSecrets,
    ) -> Result<PreparedSoftwareAccount> {
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
        let device_commitment_key = random_bytes()?;
        let next_tree_location = random_bytes()?;
        let subchain_tree_location = random_bytes()?;
        let device_name = DeviceLabelNameAndCommitmentKey {
            label: DeviceLabel {
                device_type: DeviceType::Computer,
                normalized_name: normalized_device_name,
                serial: 1,
            },
            normalization_version: 0,
            display_name: display_device_name.into_bytes(),
            commitment_key: device_commitment_key,
        };
        let tree_root = TreeRoot {
            epoch: merkle.snapshot().epoch(),
            hash: merkle.snapshot().root_hash(),
        };
        let eldest = make_software_eldest_link(
            &SoftwareEldestInput {
                host: host.host_id(),
                root: &tree_root,
                time: now_microseconds()?,
                next_tree_location,
                subchain_tree_location,
                normalized_username: &normalized_username,
                username_sequence: reservation.sequence,
                username_commitment_key,
                device_name: &device_name,
            },
            &secrets.device_seed,
            &secrets.puk_seed,
        )?;
        let puk_box = seal_initial_puk_box(
            host.host_id(),
            &secrets.device_seed,
            &secrets.puk_seed,
            InitialPukBoxRandomness {
                box_id: random_bytes()?,
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
        Ok(PreparedSoftwareAccount {
            operation_id,
            normalized_username,
            username_utf8: request.username_utf8.as_bytes().to_vec(),
            reservation,
            eldest,
            puk_box,
            username_commitment_key,
            device_name,
            next_tree_location,
            subchain_tree_location,
            passphrase,
        })
    }

    fn software_signup_request(
        &self,
        prepared: &PreparedSoftwareAccount,
        request: &SoftwareAccountRequest,
        self_token: [u8; 17],
    ) -> Result<Zeroizing<Vec<u8>>> {
        let encoded = encode_signup_request_at(
            &SoftwareSignupArgument {
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
            },
            1,
        )?;
        Ok(Zeroizing::new(encoded))
    }

    pub fn create_software_account(
        &self,
        host: &PinnedHost,
        request: SoftwareAccountRequest,
        secrets: SoftwareAccountSecrets,
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedSoftwareAccount> {
        self.check_invite_code(host, &request.invite_code)?;
        let reservation = self.reserve_username(host, &request.username_utf8)?;
        let (_, merkle) = self.advance_merkle_root(host)?;
        let prepared =
            self.prepare_software_account(host, &merkle, &request, reservation, &secrets)?;
        let signup_request =
            self.software_signup_request(&prepared, &request, *secrets.self_token)?;
        let request_hash = prefixed_hash(SIGNUP_REQUEST_HASH_TYPE_ID, &signup_request);
        let material =
            encode_signup_material(&secrets, &prepared.normalized_username, &signup_request)?;
        let operation = MutationCoordinator::new(&host.database_path, protected_store).prepare(
            MutationDraft {
                operation_id: prepared.operation_id,
                kind: MutationKind::Signup,
                host_id: host.host_id().as_bytes().to_vec(),
                scope_id: prepared.eldest.device.id.as_bytes().to_vec(),
                subject_id: prepared.eldest.uid.as_bytes().to_vec(),
                expected_version: None,
                request_hash,
            },
            material,
        )?;
        let created = self.submit_or_reconcile_software_account(
            host,
            operation,
            Some(signup_request),
            prepared.normalized_username,
            secrets,
            soft_database_path,
            protected_store,
        )?;
        if let Some(passphrase) = request.passphrase.as_ref() {
            self.verify_passphrase(host, &created.credential, passphrase)?;
        }
        Ok(created)
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_or_reconcile_software_account(
        &self,
        host: &PinnedHost,
        operation: MutationOperation,
        prepared_request: Option<Zeroizing<Vec<u8>>>,
        expected_username: Vec<u8>,
        secrets: SoftwareAccountSecrets,
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedSoftwareAccount> {
        validate_signup_operation_binding(host, &operation, &secrets)?;
        if operation.state == MutationState::Prepared {
            let request = prepared_request.ok_or(Error::OperationBinding(
                "prepared signup is missing its exact request",
            ))?;
            if prefixed_hash(SIGNUP_REQUEST_HASH_TYPE_ID, &request) != operation.request_hash {
                return Err(Error::OperationBinding(
                    "signup request fingerprint changed",
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
        ) {
            return Err(Error::OperationBinding("signup operation is terminal"));
        }

        let uid = EntityId::from_bytes(operation.subject_id.clone())?;
        let certificate_chain =
            self.fetch_device_certificate_chain(host, &uid, &secrets.device_seed)?;
        let credential = DeviceCredential {
            uid,
            seed: secrets.device_seed,
            certificate_chain,
        };
        let authenticated = self.authenticate_new_account(host, &credential)?;
        let supplied_puk = secrets.puk_seed;
        if authenticated.verified.username() != expected_username
            || authenticated
                .current_puk()
                .is_none_or(|key| key.seed != supplied_puk)
        {
            return Err(Error::CredentialBinding(
                "created account does not match the protected PUK",
            ));
        }
        let kv_projection = {
            let mut session = self.user_kv_write_session(
                host,
                &credential,
                &authenticated.verified,
                &authenticated.puks,
                soft_database_path,
                protected_store,
            )?;
            session.ensure_root(Role::OWNER, Role::OWNER)?
        };
        MutationCoordinator::new(&host.database_path, protected_store)
            .remote_verified(&operation.operation_id)?;
        Ok(CreatedSoftwareAccount {
            operation_id: operation.operation_id,
            credential,
            authenticated,
            kv_projection,
        })
    }

    /// Reconciles an interrupted signup after the server may have accepted
    /// it. This does not replay `Reg.signup`; it derives the journaled public
    /// identity from the caller's protected seeds, obtains a certificate,
    /// verifies the Merkle/user projection, and idempotently ensures a KV root.
    pub fn resume_software_account(
        &self,
        host: &PinnedHost,
        operation_id: [u8; 16],
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedSoftwareAccount> {
        let operation = HardStateStore::open(&host.database_path)?
            .mutation(&operation_id)?
            .ok_or(Error::AccountRequest("signup operation is not recorded"))?;
        let material = MutationCoordinator::new(&host.database_path, protected_store)
            .load_bound_material(&operation)?;
        let (secrets, expected_username, request) = decode_signup_material(material)?;
        self.submit_or_reconcile_software_account(
            host,
            operation,
            Some(request),
            expected_username,
            secrets,
            soft_database_path,
            protected_store,
        )
    }

    /// Recovers an interrupted signup when the process died before returning
    /// its randomly generated operation ID. The retained seeds derive the
    /// exact public UID and device ID used to locate the journal row; normal
    /// resume verification then binds the username and current PUK.
    pub fn resume_software_account_for_credential(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        soft_database_path: &Path,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<CreatedSoftwareAccount> {
        let operation = HardStateStore::open(&host.database_path)?
            .pending_mutations(host.host_id().as_bytes())?
            .into_iter()
            .find(|operation| {
                operation.kind == MutationKind::Signup
                    && operation.subject_id == uid.as_bytes()
                    && operation.scope_id == device_id.as_bytes()
            })
            .ok_or(Error::AccountRequest(
                "pending signup operation for credential is not recorded",
            ))?;
        self.resume_software_account(
            host,
            operation.operation_id,
            soft_database_path,
            protected_store,
        )
    }

    fn authenticate_new_account(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<AuthenticatedUserOutcome> {
        let mut last_error = None;
        for attempt in 0..40 {
            match self.authenticate_and_pin(host, credential) {
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
        Err(last_error.expect("account authentication loop executes at least once"))
    }
}

fn encode_signup_material(
    secrets: &SoftwareAccountSecrets,
    normalized_username: &[u8],
    request: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    let username_len = u8::try_from(normalized_username.len())
        .ok()
        .filter(|length| (3..=25).contains(length))
        .ok_or(Error::AccountRequest(
            "normalized signup username is malformed",
        ))?;
    let request_len = u32::try_from(request.len())
        .map_err(|_| Error::AccountRequest("signup request is too large to protect"))?;
    let mut material = Zeroizing::new(Vec::with_capacity(
        87 + normalized_username.len() + request.len(),
    ));
    material.push(SIGNUP_MATERIAL_VERSION);
    material.extend_from_slice(secrets.device_seed.as_slice());
    material.extend_from_slice(secrets.puk_seed.as_slice());
    material.extend_from_slice(secrets.self_token.as_slice());
    material.push(username_len);
    material.extend_from_slice(normalized_username);
    material.extend_from_slice(&request_len.to_be_bytes());
    material.extend_from_slice(request);
    Ok(material)
}

fn decode_signup_material(
    material: Zeroizing<Vec<u8>>,
) -> Result<(SoftwareAccountSecrets, Vec<u8>, Zeroizing<Vec<u8>>)> {
    const PREFIX: usize = 1 + 32 + 32 + 17 + 1;
    if material.len() < PREFIX + 4 || material[0] != SIGNUP_MATERIAL_VERSION {
        return Err(Error::OperationBinding(
            "protected signup material is malformed",
        ));
    }
    let username_len = usize::from(material[82]);
    if !(3..=25).contains(&username_len) || material.len() < PREFIX + username_len + 4 {
        return Err(Error::OperationBinding(
            "protected signup username is malformed",
        ));
    }
    let request_length_offset = PREFIX + username_len;
    let request_len = u32::from_be_bytes(
        material[request_length_offset..request_length_offset + 4]
            .try_into()
            .map_err(|_| Error::OperationBinding("protected signup request length is malformed"))?,
    ) as usize;
    let request_offset = request_length_offset + 4;
    if material.len() != request_offset + request_len {
        return Err(Error::OperationBinding(
            "protected signup request is truncated",
        ));
    }
    let device_seed = SecretSeed::from_slice(&material[1..33])?;
    let puk_seed = SecretSeed::from_slice(&material[33..65])?;
    let self_token: [u8; 17] = material[65..82]
        .try_into()
        .map_err(|_| Error::OperationBinding("protected signup token is malformed"))?;
    let expected_username = material[PREFIX..request_length_offset].to_vec();
    let request = Zeroizing::new(material[request_offset..].to_vec());
    Ok((
        SoftwareAccountSecrets::new(device_seed, puk_seed, self_token),
        expected_username,
        request,
    ))
}

fn validate_signup_operation_binding(
    host: &PinnedHost,
    operation: &MutationOperation,
    secrets: &SoftwareAccountSecrets,
) -> Result<()> {
    if operation.kind != MutationKind::Signup || operation.host_id != host.host_id().as_bytes() {
        return Err(Error::OperationBinding(
            "signup operation belongs to another kind or host",
        ));
    }
    let device = derive_device_public(&secrets.device_seed)?;
    let puk_verify = derive_shared_verify_key(&secrets.puk_seed, ENTITY_PUK_VERIFY)?;
    let mut uid_bytes = puk_verify.as_bytes().to_vec();
    uid_bytes[0] = ENTITY_USER;
    let uid = EntityId::from_bytes(uid_bytes)?;
    if operation.subject_id != uid.as_bytes() || operation.scope_id != device.id.as_bytes() {
        return Err(Error::OperationBinding(
            "signup operation does not match its protected credential",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod material_tests {
    use super::*;

    #[test]
    fn signup_material_round_trips_and_rejects_truncation() {
        let secrets = SoftwareAccountSecrets::new(
            SecretSeed::new([1; 32]),
            SecretSeed::new([2; 32]),
            [3; 17],
        );
        let encoded = encode_signup_material(&secrets, b"alice", b"exact signed request").unwrap();
        let (decoded, username, request) = decode_signup_material(encoded).unwrap();
        assert_eq!(decoded.device_seed.as_slice(), &[1; 32]);
        assert_eq!(decoded.puk_seed.as_slice(), &[2; 32]);
        assert_eq!(decoded.self_token.as_slice(), &[3; 17]);
        assert_eq!(username, b"alice");
        assert_eq!(request.as_slice(), b"exact signed request");

        assert!(decode_signup_material(Zeroizing::new(vec![1; 85])).is_err());
    }
}
