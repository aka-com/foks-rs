//! Named-team membership changes with mandatory PTK rotation.

mod reconcile;
mod support;
mod types;

use support::*;
pub use types::*;

use foks_client_db::{HardStateStore, TeamMutationKind, TeamMutationOperation, TeamMutationState};
use foks_crypto::{
    derive_device_public, derive_subkey_id, make_change_team_member_link, make_team_removal_proof,
    prefixed_hash, seal_puk_seed_chain_box, team_removal_key_commitment, ChangeTeamMemberInput,
    SharedPublicMaterial, TeamPtkRotation,
};
use foks_proto::{
    EntityId, RemoveTeamMemberArgument, Role, SecretSeed, TeamRemovalMacPayload, TreeRoot,
    ENTITY_NAMED_TEAM, MERKLE_ROOT_TYPE_ID,
};
use foks_rpc::{
    decode_team_edit_result, encode_remove_team_member_request, STATUS_TEAM_RACE_ERROR,
    STATUS_TX_RETRY_ERROR,
};
use foks_verify::{VerifiedSharedKey, VerifiedTeamMemberState};

use super::membership::{authorized_actor_member, finish_team_mutation_journal};
use super::{AuthenticatedTeamOutcome, TeamPrivateKey};
use crate::{
    current_owner_puk, now_microseconds, now_milliseconds, random_bytes, user_key_for_seed,
    AuthenticatedUserOutcome, DeviceCredential, Error, FoksClient, PinnedHost, Result,
    UserPrivateKey, YubiCredential, TEAM_MUTATION_REQUEST_HASH_TYPE_ID,
};

struct RotationBinding {
    target: EntityId,
    target_host: Option<EntityId>,
    target_source_role: Role,
    removal_key_commitment: [u8; 32],
    destination_role: Role,
    replacement: Option<(u64, EntityId)>,
    expected_seqno: u64,
    introduced: Vec<(Role, u64, EntityId)>,
}

/// The three distinguishable states of a pending PTK rotation when its
/// submission response was lost: our rotation committed, a conflicting
/// transition took the seqno, or the position is not yet observable.
pub(super) enum RotationOutcome {
    Committed(Box<AuthenticatedTeamOutcome>),
    Conflict,
    Unresolved,
}

struct Receiver<'a> {
    member: VerifiedTeamMemberState,
    key: &'a VerifiedSharedKey,
}

impl FoksClient {
    /// Removes one local user from a named team and rotates every PTK the
    /// removed member could read, following FOKS v0.1.9's exact gameplan.
    pub fn remove_local_user_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &RemoveLocalTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        let remaining = request
            .remaining_users
            .iter()
            .map(|user| VerifiedMemberParty::User(user))
            .collect::<Vec<_>>();
        self.change_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            TeamMemberSelector {
                party: request.target_user,
                host: None,
                source_role: Role::OWNER,
            },
            Role::NONE,
            None,
            Some(request.removal_key),
            request.rotations,
            &remaining,
        )
    }

    /// Hardware-backed variant of [`Self::remove_local_user_and_rotate_ptks`].
    pub fn remove_local_user_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        request: &RemoveLocalTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if !authenticated_user.verified.devices().iter().any(|device| {
            device.id == *credential.parent.entity_id()
                && device.hepk == *credential.parent.hepk()
                && device.subkey.as_ref() == Some(&subkey)
                && device.role == Role::OWNER
        }) {
            return Err(Error::CredentialBinding(
                "Yubi team editor is not an enrolled owner",
            ));
        }
        let remaining = request
            .remaining_users
            .iter()
            .map(|user| VerifiedMemberParty::User(user))
            .collect::<Vec<_>>();
        self.change_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            TeamMemberSelector {
                party: request.target_user,
                host: None,
                source_role: Role::OWNER,
            },
            Role::NONE,
            None,
            Some(request.removal_key),
            request.rotations,
            &remaining,
        )
    }

    /// Removes a local user after retrieving its committed removal key through
    /// a short-lived, PTK-authenticated TeamAdmin bearer token.
    pub fn remove_team_member_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &RemoveTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        let remaining = request
            .remaining_users
            .iter()
            .map(|user| VerifiedMemberParty::User(user))
            .collect::<Vec<_>>();
        self.change_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            TeamMemberSelector {
                party: request.target,
                host: None,
                source_role: Role::OWNER,
            },
            Role::NONE,
            None,
            None,
            request.rotations,
            &remaining,
        )
    }

    /// Hardware-backed variant of [`Self::remove_team_member_and_rotate_ptks`].
    pub fn remove_team_member_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        request: &RemoveTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let remaining = request
            .remaining_users
            .iter()
            .map(|user| VerifiedMemberParty::User(user))
            .collect::<Vec<_>>();
        self.change_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            TeamMemberSelector {
                party: request.target,
                host: None,
                source_role: Role::OWNER,
            },
            Role::NONE,
            None,
            None,
            request.rotations,
            &remaining,
        )
    }

    pub fn change_team_member_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &ChangeTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        self.change_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            request.target,
            request.destination_role,
            request.replacement,
            None,
            request.rotations,
            request.remaining_parties,
        )
    }

    pub fn change_team_member_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        request: &ChangeTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if !authenticated_user.verified.devices().iter().any(|device| {
            device.id == *credential.parent.entity_id()
                && device.hepk == *credential.parent.hepk()
                && device.subkey.as_ref() == Some(&subkey)
                && device.role == Role::OWNER
        }) {
            return Err(Error::CredentialBinding(
                "Yubi team editor is not an enrolled owner",
            ));
        }
        self.change_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            request.target,
            request.destination_role,
            request.replacement,
            None,
            request.rotations,
            request.remaining_parties,
        )
    }

    /// Reconciles a journaled generalized member transition without needing
    /// the removal key or reposting the signed edit.
    pub fn resume_change_team_member_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        expected_seqno: u64,
        request: &ChangeTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        self.resume_change_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            team,
            expected_seqno,
            request,
        )
    }

    pub fn resume_change_team_member_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        expected_seqno: u64,
        request: &ChangeTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_change_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            team,
            expected_seqno,
            request,
        )
    }

    /// Reconciles an already journaled removal without reposting it.
    pub fn resume_remove_local_user_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        expected_seqno: u64,
        request: &RemoveLocalTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        self.resume_removal_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            expected_seqno,
            request,
        )
    }

    /// Hardware-backed reconciliation variant. It never reposts the edit.
    pub fn resume_remove_local_user_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        expected_seqno: u64,
        request: &RemoveLocalTeamMemberRequest<'_>,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_removal_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            expected_seqno,
            request,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn change_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk: &UserPrivateKey,
        team: &EntityId,
        selector: TeamMemberSelector<'_>,
        destination_role: Role,
        replacement: Option<VerifiedMemberParty<'_>>,
        retained_removal_key: Option<&SecretSeed>,
        supplied_rotations: &[TeamPtkRotationSeed<'_>],
        remaining_parties: &[VerifiedMemberParty<'_>],
    ) -> Result<RotatedTeamPtks> {
        team.clone().require_type(ENTITY_NAMED_TEAM)?;
        let authenticated_team = self.load_and_pin_team_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            &actor_user.verified,
            &actor_puk.seed,
            team,
        )?;
        let actor_public = user_key_for_seed(&actor_user.verified, &actor_puk.seed)?;
        let actor_member =
            authorized_actor_member(&authenticated_team.verified, uid, actor_public)?;
        let target = unique_target(&authenticated_team.verified, selector)?;
        let self_target = target.party == *uid && target.scoped_host.is_none();
        if (self_target && destination_role == Role::NONE) || actor_member.role < target.role {
            return Err(Error::TeamRequest(
                "team editor is not authorized for this member transition",
            ));
        }
        let replacement_key = validate_replacement(target, destination_role, replacement)?;
        let loaded_removal_key;
        let removal_key = match retained_removal_key {
            Some(key) => key,
            None => {
                loaded_removal_key = self.load_team_removal_key(
                    host,
                    uid,
                    auth_seed,
                    certificate_chain,
                    &authenticated_team,
                    target,
                )?;
                &loaded_removal_key
            }
        };
        let commitment = team_removal_key_commitment(removal_key)?;
        if target.removal_key_commitment != Some(commitment) {
            return Err(Error::KeyBinding(
                "removal key does not match the authenticated member commitment",
            ));
        }
        let expected_seqno = authenticated_team
            .verified
            .chain_seqno()
            .checked_add(1)
            .ok_or(Error::TeamRequest("team sequence overflow"))?;
        let rotation_keys = validate_rotation_seeds(
            &authenticated_team,
            target,
            destination_role,
            replacement_key.map(|(_, key)| key.generation),
            supplied_rotations,
        )?;
        let receivers = resolve_remaining_receivers(
            &authenticated_team.verified,
            target,
            destination_role,
            replacement,
            &actor_user.verified,
            remaining_parties,
        )?;
        let (_, merkle) = self.advance_merkle_root(host)?;
        let root = TreeRoot {
            epoch: merkle.root().epoch,
            hash: foks_crypto::prefixed_hash_signable(
                MERKLE_ROOT_TYPE_ID,
                &merkle.root().encoded()?,
            )?,
        };
        let time = now_milliseconds()?;
        let next_tree_location = random_bytes()?;
        let crypto_rotations = rotation_keys
            .iter()
            .map(|rotation| TeamPtkRotation {
                role: rotation.role,
                generation: rotation.generation,
                seed: rotation.seed,
            })
            .collect::<Vec<_>>();
        let replacement_public = replacement_key.map(|(_, key)| SharedPublicMaterial {
            verify_key: key.verify_key.clone(),
            hepk: key.hepk.clone(),
        });
        let material = make_change_team_member_link(
            &ChangeTeamMemberInput {
                actor: uid,
                actor_source_role: actor_public.role,
                team,
                host: host.host_id(),
                sequence: expected_seqno,
                previous: authenticated_team.verified.chain_tail_hash(),
                root: &root,
                time,
                next_tree_location,
                member: &target.party,
                member_host: target.scoped_host.as_ref(),
                member_source_role: target.source_role,
                destination_role,
                member_generation: replacement_key.map(|(_, key)| key.generation),
                member_public: replacement_public.as_ref(),
            },
            &actor_puk.seed,
            &crypto_rotations,
        )?;
        let ptk_boxes = box_rotated_ptks(host, actor_puk, &rotation_keys, &receivers)?;
        let mut seed_chain = Vec::with_capacity(rotation_keys.len());
        for rotation in &rotation_keys {
            seed_chain.push(seal_puk_seed_chain_box(
                rotation.seed,
                &rotation.previous.seed,
                uid,
                host.host_id(),
                rotation.previous.generation,
                rotation.role,
                random_bytes()?,
            )?);
        }
        let removal = make_team_removal_proof(
            removal_key,
            TeamRemovalMacPayload {
                team: team.clone(),
                host: host.host_id().clone(),
                member: target.party.clone(),
                member_host: target
                    .scoped_host
                    .clone()
                    .unwrap_or_else(|| host.host_id().clone()),
                source_role: target.source_role,
                admin: uid.clone(),
                admin_host: host.host_id().clone(),
                root,
                time,
            },
        )?;
        let hepks = material
            .ptks
            .iter()
            .map(|ptk| ptk.hepk.clone())
            .collect::<Vec<_>>();
        let encoded_request = encode_remove_team_member_request(&RemoveTeamMemberArgument {
            link: &material.link,
            next_tree_location: material.next_tree_location,
            ptk_boxes: &ptk_boxes,
            seed_chain: &seed_chain,
            removals: &[removal],
            hepks: &hepks,
        })?;
        let binding = rotation_binding(
            TeamMemberSelector {
                party: &target.party,
                host: target.scoped_host.as_ref(),
                source_role: target.source_role,
            },
            commitment,
            destination_role,
            replacement_key.map(|(_, key)| (key.generation, key.verify_key.clone())),
            expected_seqno,
            &rotation_keys,
        )?;
        let operation_id = rotation_operation_id(uid, team, &binding)?;
        let created_at = now_microseconds()?;
        let operation = TeamMutationOperation {
            operation_id,
            kind: TeamMutationKind::PtkRotation,
            host_id: host.host_id().as_bytes().to_vec(),
            actor_id: uid.as_bytes().to_vec(),
            device_id: device_id.as_bytes().to_vec(),
            team_id: team.as_bytes().to_vec(),
            expected_seqno,
            request_hash: prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &encoded_request),
            state: TeamMutationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        hard_store.record_and_begin_team_mutation(&operation, now_microseconds()?)?;
        let post = || {
            let response = self.call_with_material(
                host,
                &host.user,
                &encoded_request,
                auth_seed,
                certificate_chain,
            )?;
            decode_team_edit_result(&response)?;
            Ok(())
        };
        let post_error = match post() {
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: STATUS_TX_RETRY_ERROR,
                ..
            })) => post().err(),
            Err(error) => Some(error),
            Ok(()) => None,
        };
        let is_definite_rejection = matches!(
            post_error.as_ref(),
            Some(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: STATUS_TEAM_RACE_ERROR,
                ..
            }))
        );
        if is_definite_rejection {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::Rejected,
                now_microseconds()?,
            )?;
        } else if post_error.is_none() {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::Submitted,
                now_microseconds()?,
            )?;
        } else if post_error.is_some() {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::SubmissionUnknown,
                now_microseconds()?,
            )?;
        }
        let authenticated = match self.wait_for_rotation(
            host,
            uid,
            auth_seed,
            certificate_chain,
            actor_user,
            &actor_puk.seed,
            team,
            &binding,
            supplied_rotations,
        ) {
            Ok(value) => value,
            Err(_) if post_error.is_some() => {
                if is_definite_rejection {
                    // STATUS_TEAM_RACE_ERROR is never committed; already Rejected.
                    return Err(post_error.expect("checked above"));
                }
                // Ambiguous failure: the rotation may have committed even though
                // the response was lost. Reconcile once before deciding, so we
                // neither falsely reject a committed rotation nor wedge the seqno.
                match self.authenticated_rotation_outcome(
                    host,
                    uid,
                    auth_seed,
                    certificate_chain,
                    actor_user,
                    &actor_puk.seed,
                    team,
                    &binding,
                    supplied_rotations,
                ) {
                    // The pending rotation is confirmed on-chain: finalize it.
                    Ok(RotationOutcome::Committed(authenticated)) => *authenticated,
                    // A different transition occupies the reserved sequence number:
                    // release it as Rejected.
                    Ok(RotationOutcome::Conflict) => {
                        hard_store.advance_team_mutation(
                            &operation_id,
                            TeamMutationState::Rejected,
                            now_microseconds()?,
                        )?;
                        return Err(post_error.expect("checked above"));
                    }
                    // Not yet observable (never-arrived or Merkle-lagged): release
                    // the seqno to avoid wedging a sole client, accepting a rare
                    // false-reject of a committed-but-lagged rotation.
                    Ok(RotationOutcome::Unresolved) | Err(_) => {
                        let _ = hard_store.advance_team_mutation(
                            &operation_id,
                            TeamMutationState::Rejected,
                            now_microseconds()?,
                        );
                        return Err(post_error.expect("checked above"));
                    }
                }
            }
            Err(error) => return Err(error),
        };
        finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        Ok(RotatedTeamPtks {
            operation_id,
            expected_seqno,
            authenticated,
        })
    }
}

struct ValidatedRotation<'a> {
    role: Role,
    generation: u64,
    seed: &'a SecretSeed,
    previous: &'a TeamPrivateKey,
    verify_key: EntityId,
}

#[cfg(test)]
mod tests;
