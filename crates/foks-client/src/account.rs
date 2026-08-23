//! Software-account preparation, creation, and reconciliation.

use super::{
    derive_device_public, derive_shared_verify_key, encode_registration_select_vhost_request,
    encode_reserve_username_request_at, encode_signup_request_at, fix_device_name,
    make_software_eldest_link, normalize_device_name, normalize_username, now_microseconds,
    prefixed_hash, random_bytes, seal_initial_puk_box, AuthenticatedUserOutcome, DeviceCredential,
    DeviceLabel, DeviceLabelNameAndCommitmentKey, Duration, EntityId, Error, FoksClient,
    HardStateStore, InitialPukBoxRandomness, InviteCode, KvDirectoryProjection, Path, PinnedHost,
    Result, Role, SecretSeed, SharedKeyBoxSet, SignupOperation, SignupOperationState,
    SoftwareEldestInput, SoftwareEldestMaterial, SoftwareSignupArgument, TreeRoot,
    UsernameReservation, VerifiedMerkleAdvance, Zeroizing, ENTITY_PUK_VERIFY, ENTITY_USER,
};

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
}

pub struct CreatedSoftwareAccount {
    pub operation_id: [u8; 16],
    pub credential: DeviceCredential,
    pub authenticated: AuthenticatedUserOutcome,
    pub kv_projection: Vec<KvDirectoryProjection>,
}

impl FoksClient {
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
                device_type: 0,
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
    ) -> Result<CreatedSoftwareAccount> {
        let reservation = self.reserve_username(host, &request.username_utf8)?;
        let (_, merkle) = self.advance_merkle_root(host)?;
        let prepared =
            self.prepare_software_account(host, &merkle, &request, reservation, &secrets)?;
        let signup_request =
            self.software_signup_request(&prepared, &request, *secrets.self_token)?;
        let created_at = now_microseconds()?;
        let request_hash = prefixed_hash(0x8f4b_8ab7_464f_4b53, &signup_request);
        let operation = SignupOperation {
            operation_id: prepared.operation_id,
            host_id: host.host_id().as_bytes().to_vec(),
            normalized_username: prepared.normalized_username.clone(),
            uid: prepared.eldest.uid.as_bytes().to_vec(),
            device_id: prepared.eldest.device.id.as_bytes().to_vec(),
            request_hash,
            state: SignupOperationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        hard_store.record_signup_operation(&operation)?;
        self.call_void_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &signup_request,
        )?;
        hard_store.advance_signup_operation(
            &prepared.operation_id,
            SignupOperationState::Submitted,
            now_microseconds()?,
        )?;

        let certificate_chain =
            self.fetch_device_certificate_chain(host, &prepared.eldest.uid, &secrets.device_seed)?;
        let credential = DeviceCredential {
            uid: prepared.eldest.uid.clone(),
            seed: secrets.device_seed,
            certificate_chain,
        };
        let authenticated = self.authenticate_new_account(host, &credential)?;
        let supplied_puk = secrets.puk_seed;
        if authenticated.verified.username() != prepared.normalized_username.as_slice()
            || authenticated
                .current_puk()
                .is_none_or(|key| key.seed != supplied_puk)
        {
            return Err(Error::CredentialBinding(
                "created account does not match the requested username and PUK",
            ));
        }
        let kv_projection = {
            let mut session = self.user_kv_write_session(
                host,
                &credential,
                &authenticated.verified,
                &authenticated.puks,
                soft_database_path,
            )?;
            session.ensure_root(Role::OWNER, Role::OWNER)?
        };
        hard_store.advance_signup_operation(
            &prepared.operation_id,
            SignupOperationState::Verified,
            now_microseconds()?,
        )?;
        Ok(CreatedSoftwareAccount {
            operation_id: prepared.operation_id,
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
        secrets: SoftwareAccountSecrets,
        soft_database_path: &Path,
    ) -> Result<CreatedSoftwareAccount> {
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation = hard_store
            .signup_operation(&operation_id)?
            .ok_or(Error::AccountRequest("signup operation is not recorded"))?;
        if operation.host_id != host.host_id().as_bytes() {
            return Err(Error::OperationBinding(
                "signup operation belongs to another host",
            ));
        }
        let device = derive_device_public(&secrets.device_seed)?;
        let puk_verify = derive_shared_verify_key(&secrets.puk_seed, ENTITY_PUK_VERIFY)?;
        let mut uid_bytes = puk_verify.as_bytes().to_vec();
        uid_bytes[0] = ENTITY_USER;
        let uid = EntityId::from_bytes(uid_bytes)?;
        if operation.uid != uid.as_bytes() || operation.device_id != device.id.as_bytes() {
            return Err(Error::OperationBinding(
                "signup operation does not match the supplied credential",
            ));
        }
        let certificate_chain =
            self.fetch_device_certificate_chain(host, &uid, &secrets.device_seed)?;
        let credential = DeviceCredential {
            uid,
            seed: secrets.device_seed,
            certificate_chain,
        };
        let authenticated = self.authenticate_new_account(host, &credential)?;
        if authenticated.verified.username() != operation.normalized_username.as_slice()
            || authenticated
                .current_puk()
                .is_none_or(|key| key.seed != secrets.puk_seed)
        {
            return Err(Error::CredentialBinding(
                "resumed account does not match the journaled username and PUK",
            ));
        }
        let kv_projection = {
            let mut session = self.user_kv_write_session(
                host,
                &credential,
                &authenticated.verified,
                &authenticated.puks,
                soft_database_path,
            )?;
            session.ensure_root(Role::OWNER, Role::OWNER)?
        };
        if operation.state == SignupOperationState::Prepared {
            hard_store.advance_signup_operation(
                &operation_id,
                SignupOperationState::Submitted,
                now_microseconds()?,
            )?;
        }
        if operation.state != SignupOperationState::Verified {
            hard_store.advance_signup_operation(
                &operation_id,
                SignupOperationState::Verified,
                now_microseconds()?,
            )?;
        }
        Ok(CreatedSoftwareAccount {
            operation_id,
            credential,
            authenticated,
            kv_projection,
        })
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
                Err(error) => last_error = Some(error),
            }
            if attempt != 39 {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        Err(last_error.expect("account authentication loop executes at least once"))
    }
}
