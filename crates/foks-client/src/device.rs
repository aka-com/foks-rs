//! Software-device provisioning, revocation, and PUK rotation.

use super::{
    derive_device_public, derive_shared_verify_key, encode_provision_device_request,
    encode_revoke_device_request, fix_device_name, make_software_provision_link,
    make_software_puk_rotation_link, make_software_revoke_link, normalize_device_name,
    now_microseconds, random_bytes, seal_puk_seed_chain_box, seal_software_puk_boxes,
    AuthenticatedUserOutcome, BTreeSet, DeviceCredential, DeviceLabel,
    DeviceLabelNameAndCommitmentKey, DevicePublicMaterial, DeviceType, Duration, EntityId, Error,
    FoksClient, HardStateStore, MutationCoordinator, MutationDraft, MutationKind,
    MutationOperation, MutationState, PinnedHost, ProtectedMutationStore, ProvisionDeviceArgument,
    PukBoxRandomness, PukRotation, Result, RevokeDeviceArgument, Role, SecretSeed,
    SoftwareProvisionInput, SoftwarePukBoxInput, UserMutationBase, VerifiedUserState, Zeroizing,
    ENTITY_PUK_VERIFY,
};

const USER_MUTATION_REQUEST_HASH_TYPE_ID: u64 = 0xc530_72ae_e24c_91d4;

/// Secrets for a software device being provisioned. They must be committed to
/// the encrypted credential store before the mutation is posted.
pub struct NewSoftwareDeviceSecrets {
    pub device_seed: SecretSeed,
    pub introduced_puk_seed: Option<SecretSeed>,
    pub(crate) self_token: Zeroizing<[u8; 17]>,
}

impl NewSoftwareDeviceSecrets {
    pub fn new(
        device_seed: SecretSeed,
        introduced_puk_seed: Option<SecretSeed>,
        self_token: [u8; 17],
    ) -> Self {
        Self {
            device_seed,
            introduced_puk_seed,
            self_token: Zeroizing::new(self_token),
        }
    }
}

pub struct SoftwareDeviceProvisionRequest {
    pub role: Role,
    pub device_name: String,
    pub serial: u64,
}

pub struct ProvisionedSoftwareDevice {
    pub credential: DeviceCredential,
    pub authenticated: AuthenticatedUserOutcome,
}

/// One caller-durable PUK transition used by revocation and standalone
/// rotation. Both seeds are required to construct historical recovery boxes.
pub struct UserPukRotation {
    pub role: Role,
    pub previous_generation: u64,
    pub previous_seed: SecretSeed,
    pub new_seed: SecretSeed,
}

/// Explicit caller assertion required before rotating an owner PUK without a
/// passphrase annex. Construct it only when the account has no passphrase.
pub struct NoPassphraseConfigured;

impl FoksClient {
    /// Provisions one software device through the exact v0.1.9 user-chain
    /// mutation. This convenience path holds both signing seeds; physically
    /// separated countersigning through FOKS's interactive KEX is not exposed.
    pub fn provision_software_device(
        &self,
        host: &PinnedHost,
        existing: &DeviceCredential,
        request: SoftwareDeviceProvisionRequest,
        secrets: NewSoftwareDeviceSecrets,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<ProvisionedSoftwareDevice> {
        if request.role == Role::NONE || request.serial == 0 {
            return Err(Error::AccountRequest(
                "invalid provisioned device role or serial",
            ));
        }
        let authenticated = self.authenticate_and_pin(host, existing)?;
        let signer = derive_device_public(&existing.seed)?;
        let enrolled_signer = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == signer.id)
            .ok_or(Error::UserBinding("signing device is not enrolled"))?;
        if enrolled_signer.role != Role::OWNER {
            return Err(Error::AccountRequest(
                "device provisioning requires an owner signer",
            ));
        }
        let existing_role_key = authenticated.verified.shared_key(request.role);
        let loaded_role_puks = existing_role_key
            .map(|_| self.load_puks_for_role(host, existing, &authenticated.verified, request.role))
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
            let _ = derive_shared_verify_key(seed, ENTITY_PUK_VERIFY)?;
            (seed, 1)
        };
        let display_name = fix_device_name(&request.device_name);
        let normalized_name = normalize_device_name(display_name.as_bytes())
            .ok_or(Error::AccountRequest("invalid provisioned device name"))?;
        let device_name = DeviceLabelNameAndCommitmentKey {
            label: DeviceLabel {
                device_type: DeviceType::Computer,
                normalized_name,
                serial: request.serial,
            },
            normalization_version: 0,
            display_name: display_name.into_bytes(),
            commitment_key: random_bytes()?,
        };
        let new_device = derive_device_public(&secrets.device_seed)?;
        if authenticated
            .verified
            .devices()
            .iter()
            .any(|device| device.id == new_device.id)
        {
            return Err(Error::AccountRequest("device is already enrolled"));
        }
        let next_tree_location = random_bytes()?;
        let material = make_software_provision_link(
            &SoftwareProvisionInput {
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
                    time: now_microseconds()?,
                    next_tree_location,
                },
                role: request.role,
                device_label: &device_name.label,
                device_name_commitment_key: device_name.commitment_key,
            },
            &existing.seed,
            &secrets.device_seed,
            secrets.introduced_puk_seed.as_ref(),
        )?;
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
        let puk_boxes = seal_software_puk_boxes(
            host.host_id(),
            &existing.seed,
            random_bytes()?,
            &box_inputs,
            &randomness,
        )?;
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
        })?);
        let operation_id = self.prepare_user_mutation(
            host,
            MutationKind::DeviceProvision,
            authenticated.verified.uid(),
            &new_device.id,
            authenticated.verified.chain_seqno() + 1,
            &encoded,
            protected_store,
        )?;
        let post_error =
            self.submit_user_mutation(host, existing, operation_id, &encoded, protected_store)?;
        let certificate_chain =
            match self.fetch_device_certificate_chain(host, &existing.uid, &secrets.device_seed) {
                Ok(chain) => chain,
                Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
                Err(error) => return Err(error),
            };
        let credential = DeviceCredential {
            uid: existing.uid.clone(),
            seed: secrets.device_seed,
            certificate_chain,
        };
        let authenticated = self.wait_for_user_transition(host, &credential, |user| {
            user.devices()
                .iter()
                .any(|device| device.id == new_device.id && device.role == request.role)
        })?;
        MutationCoordinator::new(&host.database_path, protected_store).verified(&operation_id)?;
        Ok(ProvisionedSoftwareDevice {
            credential,
            authenticated,
        })
    }

    /// Revokes a different enrolled user credential with a software signer and
    /// rotates exactly every PUK role the target could read. Rotation seeds
    /// must already be caller-durable.
    pub fn revoke_user_credential_with_software_device(
        &self,
        host: &PinnedHost,
        signer_credential: &DeviceCredential,
        target: &EntityId,
        rotations: &[UserPukRotation],
        no_passphrase: Option<NoPassphraseConfigured>,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<AuthenticatedUserOutcome> {
        let authenticated = self.authenticate_and_pin(host, signer_credential)?;
        let signer = derive_device_public(&signer_credential.seed)?;
        if &signer.id == target {
            return Err(Error::AccountRequest(
                "self-revocation requires an alternate verifier",
            ));
        }
        let signer_state = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == signer.id)
            .ok_or(Error::UserBinding("signing device is not enrolled"))?;
        let target_state = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| &device.id == target)
            .ok_or(Error::AccountRequest("revocation target is not enrolled"))?;
        if signer_state.role != Role::OWNER {
            return Err(Error::AccountRequest("revocation requires an owner signer"));
        }
        if target_state.role == Role::OWNER
            && authenticated
                .verified
                .devices()
                .iter()
                .filter(|device| device.role == Role::OWNER)
                .count()
                == 1
        {
            return Err(Error::AccountRequest(
                "cannot revoke the final owner device",
            ));
        }
        let expected_roles = authenticated
            .verified
            .shared_keys()
            .iter()
            .filter(|key| key.role <= target_state.role)
            .map(|key| key.role)
            .collect::<Vec<_>>();
        if rotations.len() != expected_roles.len()
            || rotations
                .iter()
                .zip(&expected_roles)
                .any(|(rotation, role)| {
                    rotation.role != *role
                        || authenticated.verified.shared_key(*role).is_none_or(|key| {
                            rotation.previous_generation != key.generation
                                || derive_shared_verify_key(
                                    &rotation.previous_seed,
                                    ENTITY_PUK_VERIFY,
                                )
                                .map_or(true, |verify| verify != key.verify_key)
                        })
                })
        {
            return Err(Error::AccountRequest(
                "revocation PUK rotation set is incomplete",
            ));
        }
        if expected_roles.contains(&Role::OWNER) && no_passphrase.is_none() {
            return Err(Error::AccountRequest(
                "owner PUK rotation requires confirmation that no passphrase is configured",
            ));
        }
        let next_tree_location = random_bytes()?;
        let rotation_refs = rotations
            .iter()
            .map(|rotation| {
                Ok(PukRotation {
                    role: rotation.role,
                    generation: rotation
                        .previous_generation
                        .checked_add(1)
                        .ok_or(Error::AccountRequest("PUK generation overflow"))?,
                    seed: &rotation.new_seed,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let link = make_software_revoke_link(
            &UserMutationBase {
                uid: authenticated.verified.uid(),
                host: authenticated.verified.host(),
                seqno: authenticated
                    .verified
                    .chain_seqno()
                    .checked_add(1)
                    .ok_or(Error::AccountRequest("user sequence overflow"))?,
                previous: authenticated.verified.chain_tail_hash(),
                root: &authenticated.verified.tree_root(),
                time: now_microseconds()?,
                next_tree_location,
            },
            &signer_credential.seed,
            target,
            &rotation_refs,
        )?;
        let recipients = authenticated
            .verified
            .devices()
            .iter()
            .filter(|device| &device.id != target)
            .map(|device| {
                (
                    device.role,
                    DevicePublicMaterial {
                        id: device.id.clone(),
                        hepk: device.hepk.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut box_inputs = Vec::new();
        for rotation in rotations {
            for receiver in recipients
                .iter()
                .filter(|(role, _)| *role >= rotation.role)
                .map(|(_, receiver)| receiver)
            {
                box_inputs.push(SoftwarePukBoxInput {
                    seed: &rotation.new_seed,
                    generation: rotation
                        .previous_generation
                        .checked_add(1)
                        .ok_or(Error::AccountRequest("PUK generation overflow"))?,
                    role: rotation.role,
                    receiver,
                });
            }
        }
        let randomness = (0..box_inputs.len())
            .map(|_| {
                Ok(PukBoxRandomness {
                    kem_message: random_bytes()?,
                    nonce: random_bytes()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let puk_boxes = seal_software_puk_boxes(
            host.host_id(),
            &signer_credential.seed,
            random_bytes()?,
            &box_inputs,
            &randomness,
        )?;
        let mut seed_chain = Vec::with_capacity(rotations.len());
        for rotation in rotations {
            seed_chain.push(seal_puk_seed_chain_box(
                &rotation.new_seed,
                &rotation.previous_seed,
                authenticated.verified.uid(),
                host.host_id(),
                rotation.previous_generation,
                rotation.role,
                random_bytes()?,
            )?);
        }
        let hepks = rotations
            .iter()
            .map(|rotation| {
                foks_crypto::derive_shared_public(&rotation.new_seed, ENTITY_PUK_VERIFY)
                    .map(|public| public.hepk)
            })
            .collect::<std::result::Result<Vec<_>, foks_crypto::Error>>()?;
        let encoded = Zeroizing::new(encode_revoke_device_request(&RevokeDeviceArgument {
            link: &link,
            puk_boxes: &puk_boxes,
            seed_chain: &seed_chain,
            next_tree_location,
            hepks: &hepks,
        })?);
        let operation_id = self.prepare_user_mutation(
            host,
            MutationKind::DeviceRevoke,
            authenticated.verified.uid(),
            target,
            authenticated.verified.chain_seqno() + 1,
            &encoded,
            protected_store,
        )?;
        let post_error = self.submit_user_mutation(
            host,
            signer_credential,
            operation_id,
            &encoded,
            protected_store,
        )?;
        let updated = match self.wait_for_user_transition(host, signer_credential, |user| {
            !user.devices().iter().any(|device| &device.id == target)
        }) {
            Ok(updated) => updated,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        for rotation in rotations {
            let verify = derive_shared_verify_key(&rotation.new_seed, ENTITY_PUK_VERIFY)?;
            if updated
                .verified
                .shared_key(rotation.role)
                .is_none_or(|key| {
                    key.generation != rotation.previous_generation + 1 || key.verify_key != verify
                })
            {
                return Err(Error::UserBinding(
                    "rotated PUK does not match the verified user transition",
                ));
            }
        }
        MutationCoordinator::new(&host.database_path, protected_store).verified(&operation_id)?;
        Ok(updated)
    }

    /// Rotates a complete prefix of user PUK roles without changing device
    /// membership. Every supplied seed must already be caller-durable.
    pub fn rotate_software_puks(
        &self,
        host: &PinnedHost,
        signer_credential: &DeviceCredential,
        rotations: &[UserPukRotation],
        no_passphrase: Option<NoPassphraseConfigured>,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<AuthenticatedUserOutcome> {
        let authenticated = self.authenticate_and_pin(host, signer_credential)?;
        let signer = derive_device_public(&signer_credential.seed)?;
        let signer_state = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == signer.id)
            .ok_or(Error::UserBinding("signing device is not enrolled"))?;
        if signer_state.role != Role::OWNER {
            return Err(Error::AccountRequest(
                "PUK rotation requires an owner signer",
            ));
        }
        let upper_role = rotations
            .last()
            .map(|rotation| rotation.role)
            .ok_or(Error::AccountRequest("PUK rotation set is empty"))?;
        let expected_roles = authenticated
            .verified
            .shared_keys()
            .iter()
            .filter(|key| key.role <= upper_role)
            .map(|key| key.role)
            .collect::<Vec<_>>();
        if rotations.len() != expected_roles.len()
            || rotations
                .iter()
                .zip(&expected_roles)
                .any(|(rotation, role)| rotation.role != *role)
        {
            return Err(Error::AccountRequest(
                "PUK rotation must be a complete ordered role prefix",
            ));
        }
        if expected_roles.contains(&Role::OWNER) && no_passphrase.is_none() {
            return Err(Error::AccountRequest(
                "owner PUK rotation requires confirmation that no passphrase is configured",
            ));
        }
        let mut new_verify_keys = BTreeSet::new();
        for rotation in rotations {
            let current =
                authenticated
                    .verified
                    .shared_key(rotation.role)
                    .ok_or(Error::UserBinding(
                        "PUK role is missing from the user chain",
                    ))?;
            let previous_verify =
                derive_shared_verify_key(&rotation.previous_seed, ENTITY_PUK_VERIFY)?;
            let new_verify = derive_shared_verify_key(&rotation.new_seed, ENTITY_PUK_VERIFY)?;
            if rotation.previous_generation != current.generation
                || previous_verify != current.verify_key
                || new_verify == current.verify_key
                || !new_verify_keys.insert(new_verify.as_bytes().to_vec())
            {
                return Err(Error::AccountRequest("invalid replacement PUK material"));
            }
        }

        let next_tree_location = random_bytes()?;
        let rotation_refs = rotations
            .iter()
            .map(|rotation| {
                Ok(PukRotation {
                    role: rotation.role,
                    generation: rotation
                        .previous_generation
                        .checked_add(1)
                        .ok_or(Error::AccountRequest("PUK generation overflow"))?,
                    seed: &rotation.new_seed,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let link = make_software_puk_rotation_link(
            &UserMutationBase {
                uid: authenticated.verified.uid(),
                host: authenticated.verified.host(),
                seqno: authenticated
                    .verified
                    .chain_seqno()
                    .checked_add(1)
                    .ok_or(Error::AccountRequest("user sequence overflow"))?,
                previous: authenticated.verified.chain_tail_hash(),
                root: &authenticated.verified.tree_root(),
                time: now_microseconds()?,
                next_tree_location,
            },
            &signer_credential.seed,
            &rotation_refs,
        )?;
        let recipients = authenticated
            .verified
            .devices()
            .iter()
            .map(|device| {
                (
                    device.role,
                    DevicePublicMaterial {
                        id: device.id.clone(),
                        hepk: device.hepk.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut box_inputs = Vec::new();
        for rotation in rotations {
            let generation = rotation
                .previous_generation
                .checked_add(1)
                .ok_or(Error::AccountRequest("PUK generation overflow"))?;
            for receiver in recipients
                .iter()
                .filter(|(role, _)| *role >= rotation.role)
                .map(|(_, receiver)| receiver)
            {
                box_inputs.push(SoftwarePukBoxInput {
                    seed: &rotation.new_seed,
                    generation,
                    role: rotation.role,
                    receiver,
                });
            }
        }
        let randomness = (0..box_inputs.len())
            .map(|_| {
                Ok(PukBoxRandomness {
                    kem_message: random_bytes()?,
                    nonce: random_bytes()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let puk_boxes = seal_software_puk_boxes(
            host.host_id(),
            &signer_credential.seed,
            random_bytes()?,
            &box_inputs,
            &randomness,
        )?;
        let mut seed_chain = Vec::with_capacity(rotations.len());
        for rotation in rotations {
            seed_chain.push(seal_puk_seed_chain_box(
                &rotation.new_seed,
                &rotation.previous_seed,
                authenticated.verified.uid(),
                host.host_id(),
                rotation.previous_generation,
                rotation.role,
                random_bytes()?,
            )?);
        }
        let hepks = rotations
            .iter()
            .map(|rotation| {
                foks_crypto::derive_shared_public(&rotation.new_seed, ENTITY_PUK_VERIFY)
                    .map(|public| public.hepk)
            })
            .collect::<std::result::Result<Vec<_>, foks_crypto::Error>>()?;
        let encoded = Zeroizing::new(encode_revoke_device_request(&RevokeDeviceArgument {
            link: &link,
            puk_boxes: &puk_boxes,
            seed_chain: &seed_chain,
            next_tree_location,
            hepks: &hepks,
        })?);
        let operation_id = self.prepare_user_mutation(
            host,
            MutationKind::PukRotation,
            authenticated.verified.uid(),
            authenticated.verified.uid(),
            authenticated.verified.chain_seqno() + 1,
            &encoded,
            protected_store,
        )?;
        let post_error = self.submit_user_mutation(
            host,
            signer_credential,
            operation_id,
            &encoded,
            protected_store,
        )?;
        let updated = match self.wait_for_user_transition(host, signer_credential, |user| {
            rotations.iter().all(|rotation| {
                let Some(expected_generation) = rotation.previous_generation.checked_add(1) else {
                    return false;
                };
                let Ok(expected_verify) =
                    derive_shared_verify_key(&rotation.new_seed, ENTITY_PUK_VERIFY)
                else {
                    return false;
                };
                user.shared_key(rotation.role).is_some_and(|key| {
                    key.generation == expected_generation && key.verify_key == expected_verify
                })
            })
        }) {
            Ok(updated) => updated,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        MutationCoordinator::new(&host.database_path, protected_store).verified(&operation_id)?;
        Ok(updated)
    }

    /// Reconciles an interrupted software-device provision. A request is sent
    /// only while the WAL is still `Prepared`; ambiguous submissions are
    /// resolved exclusively from the authenticated user chain.
    pub fn resume_software_device_provision(
        &self,
        host: &PinnedHost,
        existing: &DeviceCredential,
        operation_id: [u8; 16],
        new_device_seed: SecretSeed,
        role: Role,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<ProvisionedSoftwareDevice> {
        let operation = self.bound_user_mutation(
            host,
            operation_id,
            MutationKind::DeviceProvision,
            &existing.uid,
            protected_store,
        )?;
        let new_device = derive_device_public(&new_device_seed)?;
        if operation.subject_id != new_device.id.as_bytes() {
            return Err(Error::OperationBinding(
                "device-provision journal targets another credential",
            ));
        }
        let post_error =
            self.resume_user_mutation_submission(host, existing, &operation, protected_store)?;
        let certificate_chain =
            match self.fetch_device_certificate_chain(host, &existing.uid, &new_device_seed) {
                Ok(chain) => chain,
                Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
                Err(error) => return Err(error),
            };
        let credential = DeviceCredential {
            uid: existing.uid.clone(),
            seed: new_device_seed,
            certificate_chain,
        };
        let authenticated = self.wait_for_user_transition(host, &credential, |user| {
            user.devices()
                .iter()
                .any(|device| device.id == new_device.id && device.role == role)
        })?;
        MutationCoordinator::new(&host.database_path, protected_store).verified(&operation_id)?;
        Ok(ProvisionedSoftwareDevice {
            credential,
            authenticated,
        })
    }

    pub fn resume_user_credential_revocation(
        &self,
        host: &PinnedHost,
        signer: &DeviceCredential,
        operation_id: [u8; 16],
        target: &EntityId,
        rotations: &[UserPukRotation],
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<AuthenticatedUserOutcome> {
        let operation = self.bound_user_mutation(
            host,
            operation_id,
            MutationKind::DeviceRevoke,
            &signer.uid,
            protected_store,
        )?;
        if operation.subject_id != target.as_bytes() {
            return Err(Error::OperationBinding(
                "device-revocation journal targets another credential",
            ));
        }
        let post_error =
            self.resume_user_mutation_submission(host, signer, &operation, protected_store)?;
        let updated = match self.wait_for_user_transition(host, signer, |user| {
            !user.devices().iter().any(|device| &device.id == target)
                && rotations.iter().all(|rotation| {
                    let Some(generation) = rotation.previous_generation.checked_add(1) else {
                        return false;
                    };
                    let Ok(verify) =
                        derive_shared_verify_key(&rotation.new_seed, ENTITY_PUK_VERIFY)
                    else {
                        return false;
                    };
                    user.shared_key(rotation.role)
                        .is_some_and(|key| key.generation == generation && key.verify_key == verify)
                })
        }) {
            Ok(updated) => updated,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        MutationCoordinator::new(&host.database_path, protected_store).verified(&operation_id)?;
        Ok(updated)
    }

    pub fn resume_software_puk_rotation(
        &self,
        host: &PinnedHost,
        signer: &DeviceCredential,
        operation_id: [u8; 16],
        rotations: &[UserPukRotation],
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<AuthenticatedUserOutcome> {
        let operation = self.bound_user_mutation(
            host,
            operation_id,
            MutationKind::PukRotation,
            &signer.uid,
            protected_store,
        )?;
        let post_error =
            self.resume_user_mutation_submission(host, signer, &operation, protected_store)?;
        let updated = match self.wait_for_user_transition(host, signer, |user| {
            rotations.iter().all(|rotation| {
                let Some(generation) = rotation.previous_generation.checked_add(1) else {
                    return false;
                };
                let Ok(verify) = derive_shared_verify_key(&rotation.new_seed, ENTITY_PUK_VERIFY)
                else {
                    return false;
                };
                user.shared_key(rotation.role)
                    .is_some_and(|key| key.generation == generation && key.verify_key == verify)
            })
        }) {
            Ok(updated) => updated,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        MutationCoordinator::new(&host.database_path, protected_store).verified(&operation_id)?;
        Ok(updated)
    }

    fn bound_user_mutation(
        &self,
        host: &PinnedHost,
        operation_id: [u8; 16],
        kind: MutationKind,
        uid: &EntityId,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<MutationOperation> {
        let operation = HardStateStore::open(&host.database_path)?
            .mutation(&operation_id)?
            .ok_or(Error::OperationBinding("user mutation is not recorded"))?;
        if operation.kind != kind
            || operation.host_id != host.host_id().as_bytes()
            || operation.scope_id != uid.as_bytes()
            || operation.expected_version.is_none()
        {
            return Err(Error::OperationBinding(
                "user mutation journal binding changed",
            ));
        }
        let request = MutationCoordinator::new(&host.database_path, protected_store)
            .load_bound_material(&operation)?;
        if foks_crypto::prefixed_hash(USER_MUTATION_REQUEST_HASH_TYPE_ID, &request)
            != operation.request_hash
        {
            return Err(Error::OperationBinding(
                "user mutation request fingerprint changed",
            ));
        }
        Ok(operation)
    }

    fn resume_user_mutation_submission(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        operation: &MutationOperation,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<Option<Error>> {
        match operation.state {
            MutationState::Prepared => {
                let request = MutationCoordinator::new(&host.database_path, protected_store)
                    .load_bound_material(operation)?;
                self.submit_user_mutation(
                    host,
                    credential,
                    operation.operation_id,
                    &request,
                    protected_store,
                )
            }
            MutationState::Submitting | MutationState::SubmissionUnknown => Ok(None),
            MutationState::Verified | MutationState::Rejected => {
                Err(Error::OperationBinding("user mutation is terminal"))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_user_mutation(
        &self,
        host: &PinnedHost,
        kind: MutationKind,
        uid: &EntityId,
        subject: &EntityId,
        expected_seqno: u64,
        encoded: &[u8],
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<[u8; 16]> {
        let operation_id = random_bytes()?;
        MutationCoordinator::new(&host.database_path, protected_store).prepare(
            MutationDraft {
                operation_id,
                kind,
                host_id: host.host_id().as_bytes().to_vec(),
                scope_id: uid.as_bytes().to_vec(),
                subject_id: subject.as_bytes().to_vec(),
                expected_version: Some(expected_seqno),
                request_hash: foks_crypto::prefixed_hash(
                    USER_MUTATION_REQUEST_HASH_TYPE_ID,
                    encoded,
                ),
            },
            Zeroizing::new(encoded.to_vec()),
        )?;
        Ok(operation_id)
    }

    fn submit_user_mutation(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        operation_id: [u8; 16],
        encoded: &[u8],
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<Option<Error>> {
        MutationCoordinator::new(&host.database_path, protected_store)
            .begin_submission(&operation_id)?;
        match self.call_void(host, &host.user, encoded, credential) {
            Ok(()) => Ok(None),
            Err(error @ Error::Rpc(foks_rpc::Error::RemoteStatus { .. })) => {
                MutationCoordinator::new(&host.database_path, protected_store)
                    .rejected(&operation_id)?;
                Err(error)
            }
            Err(error) => {
                MutationCoordinator::new(&host.database_path, protected_store)
                    .submission_unknown(&operation_id)?;
                Ok(Some(error))
            }
        }
    }

    pub(crate) fn wait_for_user_transition(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        accepted: impl Fn(&VerifiedUserState) -> bool,
    ) -> Result<AuthenticatedUserOutcome> {
        let mut last_error = None;
        for attempt in 0..40 {
            match self.authenticate_and_pin(host, credential) {
                Ok(outcome) if accepted(&outcome.verified) => return Ok(outcome),
                Ok(_) => {
                    last_error = Some(Error::TransitionNotObserved("user state did not advance"));
                }
                Err(error) => last_error = Some(error),
            }
            if attempt != 39 {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        Err(last_error.expect("transition loop executes at least once"))
    }
}
