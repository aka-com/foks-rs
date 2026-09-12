//! Named-team creation and crash reconciliation.

use std::time::Duration;

use foks_client_db::{HardStateStore, TeamMutationKind, TeamMutationOperation, TeamMutationState};
use foks_crypto::{
    derive_shared_public, derive_subkey_id, make_single_owner_named_team,
    make_single_owner_named_team_yubi, named_team_id_from_admin_seed, prefixed_hash,
    seal_shared_key_boxes, seal_team_removal_key, NamedTeamInput, NamedTeamMaterial,
    PukBoxRandomness, SharedKeyBoxInput,
};
use foks_proto::{
    EntityId, NamedTeamCreateArgument, Role, SecretSeed, TeamRemovalKeyMetadata, ENTITY_PUK_VERIFY,
};
use foks_rpc::{
    decode_team_name_reservation, encode_create_named_team_request,
    encode_reserve_team_name_request, STATUS_TX_RETRY_ERROR,
};
use foks_verify::normalize_username;

use super::AuthenticatedTeamOutcome;
use crate::{
    current_owner_puk, now_microseconds, now_milliseconds, random_bytes,
    require_nonstale_shared_key, AuthenticatedUserOutcome, DeviceCredential, Error, FoksClient,
    PinnedHost, Result, UserPrivateKey, YubiCredential, TEAM_MUTATION_OPERATION_ID_TYPE_ID,
    TEAM_MUTATION_REQUEST_HASH_TYPE_ID,
};

/// Caller-durable secrets needed to create, reconcile, and later administer a
/// single-owner named team.
pub struct NamedTeamSecrets {
    pub member_min: SecretSeed,
    pub member: SecretSeed,
    pub admin: SecretSeed,
    pub owner: SecretSeed,
    pub removal_key: SecretSeed,
    pub team_name_commitment_key: [u8; 16],
}

impl NamedTeamSecrets {
    fn ordered_ptks(&self) -> [&SecretSeed; 4] {
        [&self.member_min, &self.member, &self.admin, &self.owner]
    }

    pub fn team_id(&self) -> Result<EntityId> {
        Ok(named_team_id_from_admin_seed(&self.admin)?)
    }

    pub fn operation_id(&self) -> Result<[u8; 16]> {
        let hash = prefixed_hash(
            TEAM_MUTATION_OPERATION_ID_TYPE_ID,
            self.team_id()?.as_bytes(),
        );
        Ok(hash[..16].try_into().expect("hash prefix has fixed length"))
    }
}

pub struct CreatedNamedTeam {
    pub operation_id: [u8; 16],
    pub team: EntityId,
    pub authenticated: AuthenticatedTeamOutcome,
}

impl FoksClient {
    /// Creates one local-owner named team. All supplied secrets must already
    /// be durable in the caller's encrypted credential store.
    pub fn create_single_owner_named_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team_name_utf8: &str,
        secrets: &NamedTeamSecrets,
    ) -> Result<CreatedNamedTeam> {
        let (authenticated_user, membership) = self.retry_chain_load(host, |current| {
            let authenticated = self.authenticate_and_pin(current, credential)?;
            let membership = self.load_membership_chain_tail(
                current,
                &credential.uid,
                &credential.seed,
                &credential.certificate_chain,
                &authenticated.verified,
            )?;
            Ok((authenticated, membership))
        })?;
        require_nonstale_shared_key(&authenticated_user.verified, Role::OWNER)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = credential.public_material()?.id;
        let device = authenticated_user
            .verified
            .devices()
            .iter()
            .find(|device| device.id == device_id)
            .ok_or(Error::CredentialBinding(
                "named-team creator device is not enrolled",
            ))?;
        if device.role != Role::OWNER {
            return Err(Error::CredentialBinding(
                "named-team creator device is not an owner",
            ));
        }
        self.create_named_team_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            membership,
            team_name_utf8,
            secrets,
            |input, seeds, removal| {
                make_single_owner_named_team(input, &credential.seed, &owner.seed, seeds, removal)
            },
        )
    }

    /// Yubi-backed variant of [`Self::create_single_owner_named_team`].
    pub fn create_single_owner_named_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team_name_utf8: &str,
        secrets: &NamedTeamSecrets,
    ) -> Result<CreatedNamedTeam> {
        let (authenticated_user, membership) = self.retry_chain_load(host, |current| {
            let authenticated = self.authenticate_yubi_and_pin(current, credential)?;
            let membership = self.load_membership_chain_tail(
                current,
                &credential.uid,
                &credential.subkey_seed,
                &credential.certificate_chain,
                &authenticated.verified,
            )?;
            Ok((authenticated, membership))
        })?;
        require_nonstale_shared_key(&authenticated_user.verified, Role::OWNER)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        let device = authenticated_user
            .verified
            .devices()
            .iter()
            .find(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
            .ok_or(Error::CredentialBinding(
                "Yubi named-team creator is not enrolled",
            ))?;
        if device.role != Role::OWNER {
            return Err(Error::CredentialBinding(
                "Yubi named-team creator is not an owner",
            ));
        }
        self.create_named_team_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            membership,
            team_name_utf8,
            secrets,
            |input, seeds, removal| {
                make_single_owner_named_team_yubi(
                    input,
                    credential.parent,
                    &owner.seed,
                    seeds,
                    removal,
                )
            },
        )
    }

    /// Reconciles a journaled named-team creation without replaying it.
    pub fn resume_single_owner_named_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team_name_utf8: &str,
        secrets: &NamedTeamSecrets,
    ) -> Result<CreatedNamedTeam> {
        let device_id = credential.public_material()?.id;
        self.ensure_named_team_operation_binding(host, &credential.uid, &device_id, secrets)?;
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_named_team_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            team_name_utf8,
            secrets,
        )
    }

    /// Yubi-backed variant of [`Self::resume_single_owner_named_team`].
    pub fn resume_single_owner_named_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team_name_utf8: &str,
        secrets: &NamedTeamSecrets,
    ) -> Result<CreatedNamedTeam> {
        self.ensure_named_team_operation_binding(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            secrets,
        )?;
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_named_team_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            team_name_utf8,
            secrets,
        )
    }

    fn ensure_named_team_operation_binding(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        secrets: &NamedTeamSecrets,
    ) -> Result<()> {
        let operation = HardStateStore::open(&host.database_path)?
            .team_mutation(&secrets.operation_id()?)?
            .ok_or(Error::TeamRequest(
                "named-team creation operation is not recorded",
            ))?;
        let team = secrets.team_id()?;
        if operation.kind != TeamMutationKind::NamedCreation
            || operation.expected_seqno != 1
            || operation.host_id != host.host_id().as_bytes()
            || operation.actor_id != uid.as_bytes()
            || operation.device_id != device_id.as_bytes()
            || operation.team_id != team.as_bytes()
        {
            return Err(Error::OperationBinding(
                "named-team operation does not match supplied identities",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn resume_named_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        authenticated_user: &AuthenticatedUserOutcome,
        owner_seed: &SecretSeed,
        team_name_utf8: &str,
        secrets: &NamedTeamSecrets,
    ) -> Result<CreatedNamedTeam> {
        let normalized = normalize_username(team_name_utf8.as_bytes()).ok_or(
            Error::TeamRequest("team name is not valid under FOKS v0.1.9 normalization"),
        )?;
        let operation_id = secrets.operation_id()?;
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation = hard_store
            .team_mutation(&operation_id)?
            .ok_or(Error::TeamRequest(
                "named-team creation operation is not recorded",
            ))?;
        let team = secrets.team_id()?;
        if operation.kind != TeamMutationKind::NamedCreation
            || operation.expected_seqno != 1
            || operation.host_id != host.host_id().as_bytes()
            || operation.actor_id != uid.as_bytes()
            || operation.device_id != device_id.as_bytes()
            || operation.team_id != team.as_bytes()
        {
            return Err(Error::OperationBinding(
                "named-team operation does not match supplied identities",
            ));
        }
        let authenticated = self.wait_for_named_team(
            host,
            uid,
            auth_seed,
            certificate_chain,
            authenticated_user,
            owner_seed,
            &team,
            &normalized,
            secrets,
        )?;
        super::membership::finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        Ok(CreatedNamedTeam {
            operation_id,
            team,
            authenticated,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn create_named_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        authenticated_user: &AuthenticatedUserOutcome,
        owner: &UserPrivateKey,
        membership: super::MembershipChainTail,
        team_name_utf8: &str,
        secrets: &NamedTeamSecrets,
        make_material: impl FnOnce(
            &NamedTeamInput<'_>,
            [&SecretSeed; 4],
            &SecretSeed,
        ) -> foks_crypto::Result<NamedTeamMaterial>,
    ) -> Result<CreatedNamedTeam> {
        let normalized = normalize_username(team_name_utf8.as_bytes()).ok_or(
            Error::TeamRequest("team name is not valid under FOKS v0.1.9 normalization"),
        )?;
        let reservation = decode_team_name_reservation(&self.call_with_material(
            host,
            &host.user,
            &encode_reserve_team_name_request(&normalized)?,
            auth_seed,
            certificate_chain,
        )?)?;
        let material = make_material(
            &NamedTeamInput {
                user: uid,
                host: host.host_id(),
                root: &authenticated_user.verified.tree_root(),
                time: now_milliseconds()?,
                owner_puk_generation: owner.generation,
                membership_sequence: membership.sequence,
                membership_previous: membership.previous,
                normalized_name: &normalized,
                name_sequence: reservation.sequence,
                team_name_commitment_key: secrets.team_name_commitment_key,
                next_tree_location: random_bytes()?,
                subchain_tree_location: random_bytes()?,
                membership_next_tree_location: random_bytes()?,
            },
            secrets.ordered_ptks(),
            &secrets.removal_key,
        )?;
        self.finish_named_team_creation(
            host,
            uid,
            device_id,
            auth_seed,
            certificate_chain,
            authenticated_user,
            owner,
            team_name_utf8,
            &normalized,
            &reservation,
            material,
            secrets,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_named_team_creation(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        authenticated_user: &AuthenticatedUserOutcome,
        owner: &UserPrivateKey,
        team_name_utf8: &str,
        normalized_name: &[u8],
        reservation: &foks_proto::TeamNameReservation,
        material: NamedTeamMaterial,
        secrets: &NamedTeamSecrets,
    ) -> Result<CreatedNamedTeam> {
        let owner_public = derive_shared_public(&owner.seed, ENTITY_PUK_VERIFY)?;
        let ordered_seeds = secrets.ordered_ptks();
        let roles = [
            Role::member(-0x4000),
            Role::member(0),
            Role::ADMIN,
            Role::OWNER,
        ];
        let box_inputs = ordered_seeds
            .iter()
            .zip(roles)
            .map(|(seed, role)| SharedKeyBoxInput {
                seed,
                generation: 1,
                role,
                receiver_id: uid,
                receiver_host: None,
                receiver_hepk: &owner_public.hepk,
                receiver_role: Role::OWNER,
                receiver_generation: owner.generation,
            })
            .collect::<Vec<_>>();
        let box_randomness = (0..box_inputs.len())
            .map(|_| {
                Ok(PukBoxRandomness {
                    kem_message: random_bytes()?,
                    nonce: random_bytes()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let ptk_boxes = seal_shared_key_boxes(
            host.host_id(),
            &owner.seed,
            &owner_public.hepk,
            random_bytes()?,
            &box_inputs,
            &box_randomness,
        )?;
        let admin = material
            .ptks
            .iter()
            .zip(roles)
            .find(|(_, role)| *role == Role::ADMIN)
            .map(|(key, _)| key)
            .ok_or(Error::KeyBinding("named team has no admin PTK"))?;
        let removal_metadata = TeamRemovalKeyMetadata {
            team: material.team.clone(),
            host: host.host_id().clone(),
            member: uid.clone(),
            member_host: host.host_id().clone(),
            source_role: Role::OWNER,
            destination_role: Role::OWNER,
            team_sequence: 1,
        };
        let removal_box = seal_team_removal_key(
            &owner.seed,
            &owner_public.hepk,
            &admin.hepk,
            Role::ADMIN,
            1,
            &owner_public.hepk,
            Role::OWNER,
            owner.generation,
            &secrets.removal_key,
            removal_metadata,
            [
                PukBoxRandomness {
                    kem_message: random_bytes()?,
                    nonce: random_bytes()?,
                },
                PukBoxRandomness {
                    kem_message: random_bytes()?,
                    nonce: random_bytes()?,
                },
            ],
        )?;
        if removal_box.commitment != material.removal_key_commitment {
            return Err(Error::KeyBinding(
                "removal box does not match named-team commitment",
            ));
        }
        let hepks = material
            .ptks
            .iter()
            .map(|ptk| ptk.hepk.clone())
            .collect::<Vec<_>>();
        let request = encode_create_named_team_request(&NamedTeamCreateArgument {
            name_utf8: team_name_utf8.as_bytes(),
            team_name_commitment_key: secrets.team_name_commitment_key,
            subchain_tree_location: material.subchain_tree_location,
            reservation,
            link: &material.link,
            next_tree_location: material.next_tree_location,
            ptk_boxes: &ptk_boxes,
            removal_keys: &[removal_box],
            hepks: &hepks,
            membership_link: &material.membership_link,
            membership_next_tree_location: material.membership_next_tree_location,
        })?;
        let operation_id = secrets.operation_id()?;
        let created_at = now_microseconds()?;
        let operation = TeamMutationOperation {
            operation_id,
            kind: TeamMutationKind::NamedCreation,
            host_id: host.host_id().as_bytes().to_vec(),
            actor_id: uid.as_bytes().to_vec(),
            device_id: device_id.as_bytes().to_vec(),
            team_id: material.team.as_bytes().to_vec(),
            expected_seqno: 1,
            request_hash: prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &request),
            state: TeamMutationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        hard_store.record_and_begin_team_mutation(&operation, now_microseconds()?)?;
        let post = || {
            self.call_void_with_material(host, &host.user, &request, auth_seed, certificate_chain)
        };
        let post_error = match post() {
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: STATUS_TX_RETRY_ERROR,
                ..
            })) => post().err(),
            Err(error) => Some(error),
            Ok(()) => None,
        };
        if post_error.is_none() {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::Submitted,
                now_microseconds()?,
            )?;
        }
        if post_error.is_some() {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::SubmissionUnknown,
                now_microseconds()?,
            )?;
        }
        let authenticated = match self.wait_for_named_team(
            host,
            uid,
            auth_seed,
            certificate_chain,
            authenticated_user,
            &owner.seed,
            &material.team,
            normalized_name,
            secrets,
        ) {
            Ok(value) => value,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        super::membership::finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        Ok(CreatedNamedTeam {
            operation_id,
            team: material.team,
            authenticated,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_for_named_team(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &AuthenticatedUserOutcome,
        owner_seed: &SecretSeed,
        team: &EntityId,
        normalized_name: &[u8],
        secrets: &NamedTeamSecrets,
    ) -> Result<AuthenticatedTeamOutcome> {
        let expected = [
            (Role::member(-0x4000), &secrets.member_min),
            (Role::member(0), &secrets.member),
            (Role::ADMIN, &secrets.admin),
            (Role::OWNER, &secrets.owner),
        ];
        let mut last_error = None;
        for attempt in 0..40 {
            match self.load_and_pin_team_with_material(
                host,
                uid,
                auth_seed,
                certificate_chain,
                &user.verified,
                owner_seed,
                team,
            ) {
                Ok(authenticated)
                    if authenticated.verified.team_name() == normalized_name
                        && authenticated.ptks.len() == expected.len()
                        && expected.iter().all(|(role, seed)| {
                            authenticated.ptks.iter().any(|key| {
                                key.role == *role && key.generation == 1 && key.seed == **seed
                            })
                        }) =>
                {
                    return Ok(authenticated);
                }
                Ok(_) => {
                    last_error = Some(Error::TransitionNotObserved(
                        "named team does not match prepared name and PTKs",
                    ));
                }
                Err(error) => last_error = Some(error),
            }
            if attempt != 39 {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        Err(last_error.expect("team transition loop executes at least once"))
    }
}
