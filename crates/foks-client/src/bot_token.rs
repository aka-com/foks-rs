//! Bot credentials are verified from host-bound lookup and retained only in memory.
use crate::*;
use foks_proto::{LookupUserResult, RegistrationChallenge};
use foks_rpc::{encode_get_uid_lookup_challenge_request, encode_lookup_uid_by_device_request};
const CHALLENGE_WINDOW_MILLISECONDS: u64 = 4 * 60 * 60 * 1000;
impl FoksClient {
    pub fn load_bot_token(
        &self,
        host: &PinnedHost,
        token: &foks_crypto::BotToken,
    ) -> Result<DeviceCredential> {
        let public_id = token.public_material()?.id;
        let challenge_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_get_uid_lookup_challenge_request(&public_id)?,
        )?;
        let challenge = RegistrationChallenge::decode(&challenge_bytes)?;
        let now = now_microseconds()? / 1000;
        if challenge.payload.entity != public_id
            || challenge.payload.host != *host.host_id()
            || challenge.payload.time.abs_diff(now) > CHALLENGE_WINDOW_MILLISECONDS
        {
            return Err(Error::CredentialBinding(
                "registration lookup challenge is not bound to this bot key and host",
            ));
        }
        let signature = token.sign_registration_challenge(&challenge)?;
        let lookup_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_lookup_uid_by_device_request(&public_id, &challenge, &signature)?,
        )?;
        let lookup = LookupUserResult::decode(&lookup_bytes)?;
        if lookup.host != *host.host_id()
            || lookup.role == Role::NONE
            || lookup.yubi_pq_hint.is_some()
        {
            return Err(Error::CredentialBinding(
                "bot lookup result is not bound to the selected host and key type",
            ));
        }
        let certificate_chain = self.fetch_key_certificate_chain(host, &lookup.uid, &public_id)?;
        let credential = DeviceCredential {
            key_kind: SoftwareKeyKind::BotToken,
            uid: lookup.uid,
            seed: token.derived_seed(),
            certificate_chain,
        };
        let authenticated = self.authenticate_and_pin(host, &credential)?;
        if !authenticated
            .verified
            .devices()
            .iter()
            .any(|d| d.id == public_id && d.role == lookup.role)
        {
            return Err(Error::CredentialBinding(
                "bot lookup role differs from authenticated membership",
            ));
        }
        Ok(credential)
    }
}

use crate::device::validate_fresh_puk_seeds;
use foks_crypto::{
    DevicePublicMaterial, PukBoxRandomness, SoftwareProvisionInput, SoftwarePukBoxInput,
    UserMutationBase,
};
use foks_proto::{
    DeviceLabel, DeviceLabelNameAndCommitmentKey, DeviceType, ProvisionDeviceArgument,
};
use foks_rpc::encode_provision_device_request;
use foks_verify::normalize_device_name;
impl FoksClient {
    pub fn prepare_bot_enrollment(
        &self,
        host: &PinnedHost,
        existing: FederationCredential<'_, '_>,
        role: Role,
        token: &foks_crypto::BotToken,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<MutationOperation> {
        self.prepare_bot_enrollment_with_id(
            host,
            existing,
            role,
            token,
            random_bytes()?,
            protected_store,
        )
    }
    pub fn prepare_bot_enrollment_with_id(
        &self,
        host: &PinnedHost,
        existing: FederationCredential<'_, '_>,
        role: Role,
        token: &foks_crypto::BotToken,
        operation_id: [u8; 16],
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<MutationOperation> {
        if let FederationCredential::Software(c) = existing {
            if c.key_kind == SoftwareKeyKind::BotToken {
                return Err(Error::AccountRequest(
                    "use an enrolled permanent owner to administer bot credentials",
                ));
            }
        }
        let authenticated = self.authenticate_credential_and_pin(host, existing)?;
        let introduced_puk_seed = if authenticated.verified.shared_key(role).is_none() {
            Some(SecretSeed::new(random_bytes()?))
        } else {
            None
        };
        let secrets = NewSoftwareDeviceSecrets::new(
            token.derived_seed(),
            introduced_puk_seed,
            random_bytes()?,
        );
        let request = SoftwareDeviceProvisionRequest {
            device_name: token.name(),
            role,
            serial: 1,
        };
        if request.role == Role::NONE || request.serial == 0 {
            return Err(Error::AccountRequest(
                "invalid provisioned device role or serial",
            ));
        }
        let signer_id = existing.device_id()?;
        let enrolled_signer = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == signer_id)
            .ok_or(Error::UserBinding("signing device is not enrolled"))?;
        if enrolled_signer.role != Role::OWNER {
            return Err(Error::AccountRequest(
                "device provisioning requires an owner signer",
            ));
        }
        let existing_role_key = authenticated.verified.shared_key(request.role);
        let loaded_role_puks = existing_role_key
            .map(|_| match existing {
                FederationCredential::Software(c) => {
                    self.load_puks_for_role(host, c, &authenticated.verified, request.role)
                }
                FederationCredential::Yubi(c) => {
                    self.load_puks_for_role_yubi(host, c, &authenticated.verified, request.role)
                }
            })
            .transpose()?;
        let (puk_seed, generation) = if let Some(public) = existing_role_key {
            if secrets.introduced_puk_seed.is_some() {
                return Err(Error::AccountRequest("role already has a PUK"));
            }
            let private = loaded_role_puks
                .as_deref()
                .unwrap_or_default()
                .iter()
                .find(|key| key.role == request.role && key.generation == public.generation)
                .ok_or(Error::AccountRequest("current role PUK is not loaded"))?;
            (&private.seed, public.generation)
        } else {
            let seed = secrets
                .introduced_puk_seed
                .as_ref()
                .ok_or(Error::AccountRequest(
                    "new role requires a generation-1 PUK",
                ))?;
            validate_fresh_puk_seeds(&authenticated.verified, std::iter::once(seed))?;
            (seed, 1)
        };
        let display_name = token.name();
        let normalized_name = normalize_device_name(display_name.as_bytes())
            .ok_or(Error::AccountRequest("invalid provisioned device name"))?;
        let device_name = DeviceLabelNameAndCommitmentKey {
            label: DeviceLabel {
                device_type: DeviceType::BotToken,
                normalized_name,
                serial: request.serial,
            },
            normalization_version: 0,
            display_name: display_name.into_bytes(),
            commitment_key: random_bytes()?,
        };
        let new_device = token.public_material()?;
        if authenticated
            .verified
            .devices()
            .iter()
            .any(|device| device.id == new_device.id)
        {
            return Err(Error::AccountRequest("device is already enrolled"));
        }
        let next_tree_location = random_bytes()?;
        let input = &SoftwareProvisionInput {
            base: UserMutationBase {
                uid: authenticated.verified.uid(),
                host: authenticated.verified.host(),
                seqno: authenticated
                    .verified
                    .chain_seqno()
                    .checked_add(1)
                    .ok_or(Error::AccountRequest("user sequence overflow"))?,
                previous: authenticated.verified.chain_tail_hash(),
                root: &authenticated.verified.tree_root(),
                time: now_milliseconds()?,
                next_tree_location,
            },
            role: request.role,
            device_label: &device_name.label,
            device_name_commitment_key: device_name.commitment_key,
        };
        let material = match existing {
            FederationCredential::Software(c) => foks_crypto::make_software_key_provision_link(
                input,
                &c.seed,
                c.key_kind,
                &secrets.device_seed,
                SoftwareKeyKind::BotToken,
                secrets.introduced_puk_seed.as_ref(),
            )?,
            FederationCredential::Yubi(c) => foks_crypto::make_bot_provision_link_from_yubi(
                input,
                c.parent,
                token,
                secrets.introduced_puk_seed.as_ref(),
            )?,
        };
        let mut recipients = vec![new_device.clone()];
        if existing_role_key.is_none() {
            recipients.extend(
                authenticated
                    .verified
                    .devices()
                    .iter()
                    .filter(|device| device.role >= request.role)
                    .map(|device| DevicePublicMaterial {
                        id: device.id.clone(),
                        hepk: device.hepk.clone(),
                    }),
            );
        }
        let box_inputs = recipients
            .iter()
            .map(|receiver| SoftwarePukBoxInput {
                seed: puk_seed,
                generation,
                role: request.role,
                receiver,
            })
            .collect::<Vec<_>>();
        let randomness = (0..box_inputs.len())
            .map(|_| {
                Ok(PukBoxRandomness {
                    kem_message: random_bytes()?,
                    nonce: random_bytes()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let puk_boxes = match existing {
            FederationCredential::Software(c) => foks_crypto::seal_software_puk_boxes_mixed(
                host.host_id(),
                &c.seed,
                random_bytes()?,
                &box_inputs,
                &randomness,
                foks_crypto::SoftwarePukBoxSetRandomness {
                    ephemeral_secret: crate::device::random_nonzero_p256_secret()?,
                    time: now_milliseconds()?,
                },
            )?,
            FederationCredential::Yubi(c) => {
                let inputs = box_inputs
                    .iter()
                    .map(|i| foks_crypto::YubiPukBoxInput {
                        seed: i.seed,
                        generation: i.generation,
                        role: i.role,
                        receiver: i.receiver,
                    })
                    .collect::<Vec<_>>();
                foks_crypto::seal_yubi_puk_boxes(
                    host.host_id(),
                    c.parent,
                    random_bytes()?,
                    &inputs,
                    &randomness,
                    foks_crypto::YubiPukBoxSetRandomness {
                        ephemeral_secret: crate::device::random_nonzero_p256_secret()?,
                        time: now_milliseconds()?,
                    },
                )?
            }
        };
        let mut hepks = vec![material.device.hepk.clone()];
        if let Some(puk) = material.introduced_puk.as_ref() {
            hepks.push(puk.hepk.clone());
        }
        let encoded = Zeroizing::new(encode_provision_device_request(&ProvisionDeviceArgument {
            link: &material.link,
            puk_boxes: &puk_boxes,
            device_name: &device_name,
            next_tree_location,
            self_token: *secrets.self_token,
            hepks: &hepks,
            subkey_box: None,
            yubi_pq_hint: None,
        })?);
        MutationCoordinator::new(&host.database_path, protected_store).prepare(
            MutationDraft {
                operation_id,
                kind: MutationKind::BotEnrollment,
                host_id: host.host_id().as_bytes().to_vec(),
                scope_id: authenticated.verified.uid().as_bytes().to_vec(),
                subject_id: new_device.id.as_bytes().to_vec(),
                expected_version: Some(authenticated.verified.chain_seqno() + 1),
                request_hash: foks_crypto::prefixed_hash(
                    crate::device::USER_MUTATION_REQUEST_HASH_TYPE_ID,
                    &encoded,
                ),
            },
            encoded,
        )?;
        HardStateStore::open(&host.database_path)?
            .mutation(&operation_id)?
            .ok_or(Error::OperationBinding("bot enrollment was not recorded"))
    }
}

impl FoksClient {
    pub fn bot_enrollment_progress(
        &self,
        host: &PinnedHost,
        existing: FederationCredential<'_, '_>,
        id: [u8; 16],
        attempt: bool,
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<MutationOperation> {
        self.bot_mutation_progress(
            host,
            existing,
            id,
            MutationKind::BotEnrollment,
            attempt,
            protected,
        )
    }
    pub fn bot_revocation_progress(
        &self,
        host: &PinnedHost,
        existing: FederationCredential<'_, '_>,
        id: [u8; 16],
        attempt: bool,
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<MutationOperation> {
        self.bot_mutation_progress(
            host,
            existing,
            id,
            MutationKind::DeviceRevoke,
            attempt,
            protected,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn bot_mutation_progress(
        &self,
        host: &PinnedHost,
        existing: FederationCredential<'_, '_>,
        id: [u8; 16],
        kind: MutationKind,
        attempt: bool,
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<MutationOperation> {
        let mut op = HardStateStore::open(&host.database_path)?
            .mutation(&id)?
            .ok_or(Error::OperationBinding("bot enrollment is missing"))?;
        if op.kind != kind
            || op.host_id != host.host_id().as_bytes()
            || op.scope_id != existing.uid().as_bytes()
        {
            return Err(Error::OperationBinding(
                "bot enrollment belongs to another host or account",
            ));
        }
        EntityId::from_bytes(op.subject_id.clone())?
            .require_type(foks_proto::ENTITY_BOT_TOKEN_KEY)?;
        if op.state.is_terminal() {
            crate::mutation::remove_terminal_material(protected, &op.material_ref)?;
            return Ok(op);
        }
        self.bound_user_mutation(host, id, kind, existing.uid(), protected)?;
        let frame =
            MutationCoordinator::new(&host.database_path, protected).load_bound_material(&op)?;
        let call = foks_rpc::read_call(
            &mut std::io::Cursor::new(&*frame),
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
        )?;
        if call.protocol_id() != foks_rpc::USER_PROTOCOL_ID
            || call.method_position()
                != if kind == MutationKind::BotEnrollment {
                    foks_rpc::USER_PROVISION_DEVICE_METHOD_POSITION
                } else {
                    foks_rpc::USER_REVOKE_DEVICE_METHOD_POSITION
                }
        {
            return Err(Error::OperationBinding("bot material contains another RPC"));
        }
        let link = if kind == MutationKind::BotEnrollment {
            foks_proto::DecodedProvisionDeviceArgument::decode(call.argument())?.link
        } else {
            foks_proto::DecodedRevokeDeviceArgument::decode(call.argument())?.link
        };
        let change = link.decode_group_change()?;
        if op.expected_version != Some(change.seqno)
            || change.signer != existing.device_id()?
            || change.changes.len() != 1
            || change.changes[0].entity.as_bytes() != op.subject_id
        {
            return Err(Error::OperationBinding(
                "bot enrollment signer or target changed",
            ));
        }
        if attempt && op.state == MutationState::Prepared {
            self.authenticate_credential_and_pin(host, existing)?;
            MutationCoordinator::new(&host.database_path, protected).begin_submission(&id)?;
            let (seed, certs) = existing.transport();
            if let Err(error) = self.call_void_with_material(host, &host.user, &frame, seed, certs)
            {
                MutationCoordinator::new(&host.database_path, protected).submission_unknown(&id)?;
                return Err(error);
            }
            op = HardStateStore::open(&host.database_path)?
                .mutation(&id)?
                .ok_or(Error::OperationBinding("missing bot receipt"))?;
        }
        if matches!(
            op.state,
            MutationState::Submitting | MutationState::SubmissionUnknown
        ) {
            let outcome = self.authenticate_credential_and_pin(host, existing);
            match outcome {
                Ok(current) => {
                    let observed = current.verified.authenticated_link(change.seqno)?;
                    if kind == MutationKind::DeviceRevoke
                        && observed.as_ref().map(|v| v.encoded()).transpose()?
                            == Some(link.encoded()?)
                    {
                        match existing {
                            FederationCredential::Software(c) => {
                                self.confirm_persisted_passphrase_annex(host, c, &op, protected)?
                            }
                            FederationCredential::Yubi(c) => self
                                .confirm_persisted_passphrase_annex_yubi(host, c, &op, protected)?,
                        }
                    }
                    let mut coordinator = MutationCoordinator::new(&host.database_path, protected);
                    match observed {
                        Some(observed) if observed.encoded()? == link.encoded()? => {
                            coordinator.remote_verified(&id)?
                        }
                        Some(_) => coordinator.rejected(&id)?,
                        None if op.state == MutationState::Submitting => {
                            coordinator.submission_unknown(&id)?
                        }
                        None => {}
                    }
                }
                Err(Error::Verify(foks_verify::Error::UserMerkleProof)) => {
                    if op.state == MutationState::Submitting {
                        MutationCoordinator::new(&host.database_path, protected)
                            .submission_unknown(&id)?;
                    }
                }
                Err(error) => return Err(error),
            }
            op = HardStateStore::open(&host.database_path)?
                .mutation(&id)?
                .ok_or(Error::OperationBinding("missing bot receipt"))?;
        }
        Ok(op)
    }
}
