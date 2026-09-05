//! Crash reconciliation and authenticated transition observation.

use std::time::Duration;

use foks_client_db::{HardStateStore, TeamMutationState};
use foks_crypto::{derive_shared_public, team_removal_key_commitment};
use foks_proto::{EntityId, Role, SecretSeed, ENTITY_NAMED_TEAM, ENTITY_PTK_VERIFY};

use super::{
    finish_team_mutation_journal, rotation_operation_id, validate_rotation_change,
    validate_rotation_operation, validate_rotation_transition, AuthenticatedTeamOutcome,
    ChangeTeamMemberRequest, FoksClient, RemoveLocalTeamMemberRequest, RotatedTeamPtks,
    RotationBinding, RotationOutcome, TeamPtkRotationSeed,
};
use crate::{
    AuthenticatedUserOutcome, Error, PinnedHost, ProtectedMutationStore, Result, UserPrivateKey,
};

impl FoksClient {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resume_change_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk_seed: &SecretSeed,
        team: &EntityId,
        expected_seqno: u64,
        expected_operation_id: &[u8; 16],
        request: &ChangeTeamMemberRequest<'_>,
        retained_removal_key: Option<&SecretSeed>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let recorded = HardStateStore::open(&host.database_path)?
            .team_mutation(expected_operation_id)?
            .ok_or(Error::TeamRequest("team transition is not recorded"))?;
        validate_rotation_operation(&recorded, host, uid, team, expected_seqno)?;
        let authenticated = self.load_and_pin_team_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            &actor_user.verified,
            actor_puk_seed,
            team,
        )?;
        if authenticated.verified.chain_seqno() < expected_seqno {
            let operation_id = super::prepared_rotation_operation_id(
                uid,
                team,
                &authenticated,
                request.target,
                request.destination_role,
                request.replacement,
                request.rotations,
            )?;
            if operation_id != *expected_operation_id {
                return Err(Error::OperationBinding(
                    "resumed team transition differs from the recorded operation",
                ));
            }
            let actor_puk = actor_user
                .puks
                .iter()
                .find(|private| private.seed == *actor_puk_seed)
                .ok_or(Error::KeyBinding(
                    "journaled team editor PUK is unavailable",
                ))?;
            let replayed = self.change_with_material(
                host,
                uid,
                device_id,
                auth_seed,
                certificate_chain,
                actor_user,
                actor_puk,
                team,
                request.target,
                request.destination_role,
                request.replacement,
                retained_removal_key,
                request.rotations,
                request.remaining_parties,
                protected_store,
            )?;
            if replayed.operation_id != *expected_operation_id {
                return Err(Error::OperationBinding(
                    "replayed team transition differs from the recorded operation",
                ));
            }
            return Ok(replayed);
        }
        let change = authenticated.verified.group_change_at(expected_seqno)?;
        let [member] = change.changes.as_slice() else {
            return Err(Error::OperationBinding(
                "journaled sequence contains another roster transition",
            ));
        };
        let replacement = request
            .replacement
            .and_then(|party| party.shared_key(request.target.source_role))
            .map(|key| (key.generation, key.verify_key.clone()));
        let introduced = change
            .shared_keys
            .iter()
            .zip(request.rotations)
            .map(|(public, rotation)| {
                let verify = derive_shared_public(rotation.seed, ENTITY_PTK_VERIFY)?.verify_key;
                if public.role != rotation.role || public.verify_key != verify {
                    return Err(Error::OperationBinding(
                        "caller-retained PTK does not match the observed transition",
                    ));
                }
                Ok((public.role, public.generation, verify))
            })
            .collect::<Result<Vec<_>>>()?;
        if introduced.len() != request.rotations.len() {
            return Err(Error::OperationBinding(
                "observed PTK schedule differs from the resumed transition",
            ));
        }
        let binding = RotationBinding {
            target: request.target.party.clone(),
            target_host: request.target.host.cloned(),
            target_source_role: request.target.source_role,
            removal_key_commitment: [0; 32],
            destination_role: request.destination_role,
            replacement,
            expected_seqno,
            introduced,
        };
        validate_rotation_change(&change, &binding)?;
        if member.party != *request.target.party {
            return Err(Error::OperationBinding(
                "observed member differs from the resumed target",
            ));
        }
        if !request.rotations.iter().all(|rotation| {
            authenticated
                .ptks
                .iter()
                .any(|ptk| ptk.role == rotation.role && ptk.seed == *rotation.seed)
        }) {
            return Err(Error::OperationBinding(
                "caller-retained PTKs do not match the authenticated team state",
            ));
        }
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation = hard_store
            .team_mutation(expected_operation_id)?
            .ok_or(Error::TeamRequest("team transition is not recorded"))?;
        validate_rotation_operation(&operation, host, uid, team, expected_seqno)?;
        let material_key = super::team_rotation_material_key(expected_operation_id);
        if operation.state != TeamMutationState::Verified {
            let exact_request = protected_store
                .get(&material_key)
                .map_err(super::protected_material_error)?;
            if foks_crypto::prefixed_hash(crate::TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &exact_request)
                != operation.request_hash
            {
                return Err(Error::OperationBinding(
                    "protected team transition changed before reconciliation",
                ));
            }
            let protected = super::decode_protected_team_edit_request(&exact_request)?;
            if protected.team_bearer_token.is_some()
                || protected.link.decode_team_group_change()? != change
            {
                return Err(Error::OperationBinding(
                    "authenticated transition differs from the exact recorded team request",
                ));
            }
            finish_team_mutation_journal(&mut hard_store, &operation.operation_id)?;
        }
        super::remove_team_rekey_material(protected_store, &material_key)?;
        Ok(RotatedTeamPtks {
            operation_id: operation.operation_id,
            expected_seqno,
            authenticated,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn resume_removal_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk: &UserPrivateKey,
        team: &EntityId,
        expected_seqno: u64,
        expected_operation_id: &[u8; 16],
        request: &RemoveLocalTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        team.clone().require_type(ENTITY_NAMED_TEAM)?;
        let recorded = HardStateStore::open(&host.database_path)?
            .team_mutation(expected_operation_id)?
            .ok_or(Error::TeamRequest("PTK rotation is not recorded"))?;
        validate_rotation_operation(&recorded, host, uid, team, expected_seqno)?;
        let observed = self.load_and_pin_team_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            &actor_user.verified,
            &actor_puk.seed,
            team,
        )?;
        if observed.verified.chain_seqno() < expected_seqno {
            let operation_id =
                self.remove_local_user_and_rotate_ptks_operation_id(uid, team, &observed, request)?;
            if operation_id != *expected_operation_id {
                return Err(Error::OperationBinding(
                    "resumed team removal differs from the recorded operation",
                ));
            }
            let remaining = request
                .remaining_users
                .iter()
                .map(|user| super::VerifiedMemberParty::User(user))
                .collect::<Vec<_>>();
            let replayed = self.change_with_material(
                host,
                uid,
                device_id,
                auth_seed,
                certificate_chain,
                actor_user,
                actor_puk,
                team,
                super::TeamMemberSelector {
                    party: request.target_user,
                    host: None,
                    source_role: Role::OWNER,
                },
                Role::NONE,
                None,
                Some(request.removal_key),
                request.rotations,
                &remaining,
                protected_store,
            )?;
            if replayed.operation_id != *expected_operation_id {
                return Err(Error::OperationBinding(
                    "replayed team removal differs from the recorded operation",
                ));
            }
            return Ok(replayed);
        }
        let change = observed.verified.group_change_at(expected_seqno)?;
        let [removed] = change.changes.as_slice() else {
            return Err(Error::OperationBinding(
                "journaled PTK rotation sequence contains another roster transition",
            ));
        };
        if removed.party != *request.target_user
            || removed.role != Role::NONE
            || removed.keys.is_some()
            || change.shared_keys.len() != request.rotations.len()
        {
            return Err(Error::OperationBinding(
                "observed transition does not describe the requested removal",
            ));
        }
        let introduced = change
            .shared_keys
            .iter()
            .zip(request.rotations)
            .map(|(public, rotation)| {
                let verify = derive_shared_public(rotation.seed, ENTITY_PTK_VERIFY)?.verify_key;
                if public.role != rotation.role || public.verify_key != verify {
                    return Err(Error::OperationBinding(
                        "caller-retained PTK does not match the observed rotation",
                    ));
                }
                Ok((public.role, public.generation, verify))
            })
            .collect::<Result<Vec<_>>>()?;
        let binding = RotationBinding {
            target: request.target_user.clone(),
            target_host: None,
            target_source_role: removed.source_role,
            removal_key_commitment: team_removal_key_commitment(request.removal_key)?,
            destination_role: Role::NONE,
            replacement: None,
            expected_seqno,
            introduced,
        };
        let operation_id = rotation_operation_id(uid, team, &binding)?;
        if operation_id != *expected_operation_id {
            return Err(Error::OperationBinding(
                "resumed team removal differs from the recorded operation",
            ));
        }
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation = hard_store
            .team_mutation(&operation_id)?
            .ok_or(Error::TeamRequest("PTK rotation is not recorded"))?;
        validate_rotation_operation(&operation, host, uid, team, expected_seqno)?;
        validate_rotation_transition(&observed.verified, &binding)?;
        if !request.rotations.iter().all(|rotation| {
            observed
                .ptks
                .iter()
                .any(|ptk| ptk.role == rotation.role && ptk.seed == *rotation.seed)
        }) {
            return Err(Error::OperationBinding(
                "caller-retained PTKs do not match the authenticated team state",
            ));
        }
        let material_key = super::team_rotation_material_key(&operation_id);
        if operation.state != TeamMutationState::Verified {
            let exact_request = protected_store
                .get(&material_key)
                .map_err(super::protected_material_error)?;
            if foks_crypto::prefixed_hash(crate::TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &exact_request)
                != operation.request_hash
            {
                return Err(Error::OperationBinding(
                    "protected team removal changed before reconciliation",
                ));
            }
            let protected = super::decode_protected_team_edit_request(&exact_request)?;
            if protected.team_bearer_token.is_some()
                || protected.link.decode_team_group_change()? != change
            {
                return Err(Error::OperationBinding(
                    "authenticated removal differs from the exact recorded team request",
                ));
            }
            finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        }
        super::remove_team_rekey_material(protected_store, &material_key)?;
        Ok(RotatedTeamPtks {
            operation_id,
            expected_seqno,
            authenticated: observed,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn wait_for_rotation(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk_seed: &SecretSeed,
        team: &EntityId,
        binding: &RotationBinding,
        rotations: &[TeamPtkRotationSeed<'_>],
    ) -> Result<AuthenticatedTeamOutcome> {
        let mut last_error = None;
        for attempt in 0..40 {
            match self.load_and_pin_team_with_material(
                host,
                uid,
                auth_seed,
                certificate_chain,
                &actor_user.verified,
                actor_puk_seed,
                team,
            ) {
                Ok(authenticated)
                    if authenticated.verified.chain_seqno() >= binding.expected_seqno =>
                {
                    validate_rotation_transition(&authenticated.verified, binding)?;
                    if rotations.iter().all(|rotation| {
                        authenticated
                            .ptks
                            .iter()
                            .any(|ptk| ptk.role == rotation.role && ptk.seed == *rotation.seed)
                    }) {
                        return Ok(authenticated);
                    }
                    last_error = Some(Error::TransitionNotObserved(
                        "rotated PTKs do not match caller-retained secrets",
                    ));
                }
                Ok(_) => {
                    last_error = Some(Error::TransitionNotObserved(
                        "team chain has not reached the prepared PTK rotation",
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

    /// One authoritative reconciliation of a pending PTK rotation against the
    /// current authenticated team chain, distinguishing the committed rotation associated
    /// with the pending operation, a
    /// conflicting transition at the seqno, or a not-yet-observable position.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn authenticated_rotation_outcome(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk_seed: &SecretSeed,
        team: &EntityId,
        binding: &RotationBinding,
        rotations: &[TeamPtkRotationSeed<'_>],
    ) -> Result<RotationOutcome> {
        let authenticated = self.load_and_pin_team_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            &actor_user.verified,
            actor_puk_seed,
            team,
        )?;
        if authenticated.verified.chain_seqno() < binding.expected_seqno {
            return Ok(RotationOutcome::Unresolved);
        }
        // A committed rotation at the pending operation's sequence number carries its generated PTK
        // verify key, which validate_rotation_transition checks against the
        // binding — so an Ok result proves the on-chain rotation is ours.
        match validate_rotation_transition(&authenticated.verified, binding) {
            Ok(())
                if rotations.iter().all(|rotation| {
                    authenticated
                        .ptks
                        .iter()
                        .any(|ptk| ptk.role == rotation.role && ptk.seed == *rotation.seed)
                }) =>
            {
                Ok(RotationOutcome::Committed(Box::new(authenticated)))
            }
            Ok(()) => Ok(RotationOutcome::Unresolved),
            Err(Error::OperationBinding(_)) => Ok(RotationOutcome::Conflict),
            Err(error) => Err(error),
        }
    }
}
