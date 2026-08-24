//! Named-team membership additions and crash reconciliation.

use std::time::Duration;

use foks_client_db::{HardStateStore, TeamMutationKind, TeamMutationOperation, TeamMutationState};
use foks_crypto::{
    derive_device_public, derive_shared_public, derive_subkey_id, make_add_local_team_member_link,
    prefixed_hash, seal_shared_key_boxes, seal_team_removal_key, AddLocalTeamMemberInput,
    PukBoxRandomness, SharedKeyBoxInput, SharedPublicMaterial,
};
use foks_proto::{
    AddTeamMemberArgument, EntityId, Role, RoleType, SecretSeed, TeamRemovalKeyMetadata, TreeRoot,
    ENTITY_NAMED_TEAM, ENTITY_PUK_VERIFY, MERKLE_ROOT_TYPE_ID,
};
use foks_rpc::{decode_team_edit_result, encode_add_team_member_request, STATUS_TX_RETRY_ERROR};
use foks_snowpack::{encode, Value};
use foks_verify::{
    VerifiedSharedKey, VerifiedTeamMemberState, VerifiedTeamState, VerifiedUserState,
};

use super::{AuthenticatedTeamOutcome, TeamPrivateKey};
use crate::{
    current_owner_puk, now_microseconds, random_bytes, user_key_for_seed, AuthenticatedUserOutcome,
    DeviceCredential, Error, FoksClient, PinnedHost, Result, UserPrivateKey, YubiCredential,
    TEAM_MUTATION_OPERATION_ID_TYPE_ID, TEAM_MUTATION_REQUEST_HASH_TYPE_ID,
};

/// Caller-durable material for adding one local user to a named team.
///
/// The removal key must be persisted in the application's encrypted secret
/// store before submission. It is needed for later removal/rotation and for
/// crash reconciliation, but is never written to the public hard-state DB.
pub struct AddLocalTeamMemberRequest<'a> {
    pub target_user: &'a VerifiedUserState,
    pub destination_role: Role,
    pub removal_key: &'a SecretSeed,
}

pub struct AddedLocalTeamMember {
    pub operation_id: [u8; 16],
    pub expected_seqno: u64,
    pub authenticated: AuthenticatedTeamOutcome,
}

struct AdditionBinding<'a> {
    target_id: &'a EntityId,
    target: &'a VerifiedSharedKey,
    destination_role: Role,
    removal_key_commitment: [u8; 32],
    expected_seqno: u64,
}

impl FoksClient {
    /// Adds one current local user PUK to a named team using FOKS v0.1.9's
    /// open-viewership edit path. Pure additions reuse existing PTKs and box
    /// only the roles visible at `destination_role` to the new member.
    pub fn add_local_user_to_named_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &AddLocalTeamMemberRequest<'_>,
    ) -> Result<AddedLocalTeamMember> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        self.add_local_user_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            request,
        )
    }

    /// Hardware-backed variant of [`Self::add_local_user_to_named_team`].
    pub fn add_local_user_to_named_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        request: &AddLocalTeamMemberRequest<'_>,
    ) -> Result<AddedLocalTeamMember> {
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
        self.add_local_user_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            request,
        )
    }

    /// Reconciles a journaled addition without replaying its signed mutation.
    pub fn resume_add_local_user_to_named_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        expected_seqno: u64,
        request: &AddLocalTeamMemberRequest<'_>,
    ) -> Result<AddedLocalTeamMember> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        self.resume_addition_with_material(
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
    pub fn resume_add_local_user_to_named_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        expected_seqno: u64,
        request: &AddLocalTeamMemberRequest<'_>,
    ) -> Result<AddedLocalTeamMember> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_addition_with_material(
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
    fn add_local_user_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk: &UserPrivateKey,
        team: &EntityId,
        request: &AddLocalTeamMemberRequest<'_>,
    ) -> Result<AddedLocalTeamMember> {
        self.require_open_user_viewership(host, auth_seed, certificate_chain)?;
        validate_target_user(host, uid, team, request)?;
        let target = current_owner_public(request.target_user)?;
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
        if authenticated_team
            .verified
            .members()
            .iter()
            .any(|member| member.party == *request.target_user.uid())
        {
            return Err(Error::TeamRequest("target user is already a team member"));
        }
        if request.destination_role > actor_member.role {
            return Err(Error::TeamRequest(
                "team editor cannot grant a role above its own",
            ));
        }
        if authenticated_team
            .verified
            .shared_key(request.destination_role)
            .is_none()
        {
            return Err(Error::TeamRequest(
                "destination role has no existing PTK; addition would require rotation",
            ));
        }
        let expected_seqno = authenticated_team
            .verified
            .chain_seqno()
            .checked_add(1)
            .ok_or(Error::TeamRequest("team sequence overflow"))?;
        let (_, merkle) = self.advance_merkle_root(host)?;
        let root = TreeRoot {
            epoch: merkle.root().epoch,
            hash: prefixed_hash(MERKLE_ROOT_TYPE_ID, &merkle.root().encoded()?),
        };
        let target_public = SharedPublicMaterial {
            verify_key: target.verify_key.clone(),
            hepk: target.hepk.clone(),
        };
        let material = make_add_local_team_member_link(
            &AddLocalTeamMemberInput {
                actor: uid,
                actor_source_role: actor_public.role,
                team,
                host: host.host_id(),
                sequence: expected_seqno,
                previous: authenticated_team.verified.chain_tail_hash(),
                root: &root,
                time: now_microseconds()?,
                next_tree_location: random_bytes()?,
                member: request.target_user.uid(),
                member_source_role: target.role,
                member_destination_role: request.destination_role,
                member_generation: target.generation,
                member_public: &target_public,
            },
            &actor_puk.seed,
            request.removal_key,
        )?;
        let ptk_boxes = box_visible_ptks(
            host,
            actor_puk,
            request.target_user.uid(),
            target,
            request.destination_role,
            &authenticated_team,
        )?;
        let admin_public = authenticated_team
            .verified
            .shared_key(Role::ADMIN)
            .ok_or(Error::KeyBinding("named team has no current admin PTK"))?;
        let admin_private = current_team_private_key(&authenticated_team, admin_public)?;
        let actor_hepk = derive_shared_public(&actor_puk.seed, ENTITY_PUK_VERIFY)?.hepk;
        let removal_metadata = TeamRemovalKeyMetadata {
            team: team.clone(),
            host: host.host_id().clone(),
            member: request.target_user.uid().clone(),
            member_host: host.host_id().clone(),
            source_role: target.role,
            destination_role: request.destination_role,
            team_sequence: expected_seqno,
        };
        let removal_box = seal_team_removal_key(
            &actor_puk.seed,
            &actor_hepk,
            &admin_public.hepk,
            Role::ADMIN,
            admin_private.generation,
            &target.hepk,
            target.role,
            target.generation,
            request.removal_key,
            removal_metadata,
            [random_box_randomness()?, random_box_randomness()?],
        )?;
        if removal_box.commitment != material.removal_key_commitment {
            return Err(Error::KeyBinding(
                "member removal box does not match the signed commitment",
            ));
        }
        let encoded_request = encode_add_team_member_request(&AddTeamMemberArgument {
            link: &material.link,
            next_tree_location: material.next_tree_location,
            ptk_boxes: &ptk_boxes,
            removal_keys: &[removal_box],
            hepks: std::slice::from_ref(&target.hepk),
            local_permissions_for: std::slice::from_ref(request.target_user.uid()),
        })?;
        let binding = AdditionBinding {
            target_id: request.target_user.uid(),
            target,
            destination_role: request.destination_role,
            removal_key_commitment: material.removal_key_commitment,
            expected_seqno,
        };
        let operation_id = addition_operation_id(uid, team, &binding)?;
        let created_at = now_microseconds()?;
        let operation = TeamMutationOperation {
            operation_id,
            kind: TeamMutationKind::MembershipChange,
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
        hard_store.record_team_mutation(&operation)?;
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
        if post_error.is_none() {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::Submitted,
                now_microseconds()?,
            )?;
        }
        if matches!(
            post_error,
            Some(Error::Rpc(foks_rpc::Error::RemoteStatus { .. }))
        ) {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::Rejected,
                now_microseconds()?,
            )?;
            return Err(post_error.expect("matched above"));
        }
        let authenticated = match self.wait_for_addition(
            host,
            uid,
            auth_seed,
            certificate_chain,
            actor_user,
            &actor_puk.seed,
            team,
            &binding,
        ) {
            Ok(value) => value,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        Ok(AddedLocalTeamMember {
            operation_id,
            expected_seqno,
            authenticated,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn resume_addition_with_material(
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
        request: &AddLocalTeamMemberRequest<'_>,
    ) -> Result<AddedLocalTeamMember> {
        validate_target_user(host, uid, team, request)?;
        let target = current_owner_public(request.target_user)?;
        let removal_key_commitment = foks_crypto::team_removal_key_commitment(request.removal_key)?;
        let binding = AdditionBinding {
            target_id: request.target_user.uid(),
            target,
            destination_role: request.destination_role,
            removal_key_commitment,
            expected_seqno,
        };
        let operation_id = addition_operation_id(uid, team, &binding)?;
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation = hard_store
            .team_mutation(&operation_id)?
            .ok_or(Error::TeamRequest("team-member addition is not recorded"))?;
        validate_addition_operation(&operation, host, uid, device_id, team, expected_seqno)?;
        let authenticated = self.wait_for_addition(
            host,
            uid,
            auth_seed,
            certificate_chain,
            actor_user,
            &actor_puk.seed,
            team,
            &binding,
        )?;
        finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        Ok(AddedLocalTeamMember {
            operation_id,
            expected_seqno,
            authenticated,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_for_addition(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk_seed: &SecretSeed,
        team: &EntityId,
        binding: &AdditionBinding<'_>,
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
                    validate_addition_transition(&authenticated.verified, binding)?;
                    return Ok(authenticated);
                }
                Ok(_) => {
                    last_error = Some(Error::TransitionNotObserved(
                        "team chain has not reached the prepared addition",
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

fn validate_target_user(
    host: &PinnedHost,
    actor: &EntityId,
    team: &EntityId,
    request: &AddLocalTeamMemberRequest<'_>,
) -> Result<()> {
    team.clone().require_type(ENTITY_NAMED_TEAM)?;
    if request.target_user.host() != host.host_id()
        || request.target_user.uid() == actor
        || request.destination_role == Role::NONE
    {
        return Err(Error::TeamRequest(
            "target user, host, or destination role is invalid",
        ));
    }
    Ok(())
}

fn current_owner_public(user: &VerifiedUserState) -> Result<&VerifiedSharedKey> {
    user.shared_key(Role::OWNER)
        .ok_or(Error::KeyBinding("target user has no current owner PUK"))
}

pub(super) fn authorized_actor_member<'a>(
    team: &'a VerifiedTeamState,
    actor: &EntityId,
    actor_puk: &VerifiedSharedKey,
) -> Result<&'a VerifiedTeamMemberState> {
    let mut matches = team.members().iter().filter(|member| {
        member.party == *actor
            && member.source_role == actor_puk.role
            && member.generation == actor_puk.generation
            && member.verify_key.as_bytes()[1..] == actor_puk.verify_key.as_bytes()[1..]
    });
    let member = matches
        .next()
        .ok_or(Error::TeamBinding("actor is not a current team member"))?;
    if matches.next().is_some()
        || !matches!(member.role.kind(), RoleType::Admin | RoleType::Owner)
        || member.scoped_host.is_some()
    {
        return Err(Error::TeamBinding(
            "team editor is not one unscoped admin or owner",
        ));
    }
    Ok(member)
}

pub(super) fn current_team_private_key<'a>(
    team: &'a AuthenticatedTeamOutcome,
    public: &VerifiedSharedKey,
) -> Result<&'a TeamPrivateKey> {
    let mut matches = team
        .ptks
        .iter()
        .filter(|private| private.role == public.role && private.generation == public.generation);
    let private = matches
        .next()
        .ok_or(Error::KeyBinding("current team PTK is unavailable"))?;
    if matches.next().is_some()
        || derive_shared_public(&private.seed, foks_proto::ENTITY_PTK_VERIFY)?.verify_key
            != public.verify_key
    {
        return Err(Error::KeyBinding(
            "current team PTK does not match verified public state",
        ));
    }
    Ok(private)
}

fn box_visible_ptks(
    host: &PinnedHost,
    actor_puk: &UserPrivateKey,
    target_id: &EntityId,
    target: &VerifiedSharedKey,
    destination_role: Role,
    team: &AuthenticatedTeamOutcome,
) -> Result<foks_proto::SharedKeyBoxSet> {
    let visible = team
        .verified
        .shared_keys()
        .iter()
        .filter(|key| key.role <= destination_role)
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return Err(Error::KeyBinding("destination role exposes no team PTKs"));
    }
    let private = visible
        .iter()
        .map(|public| current_team_private_key(team, public))
        .collect::<Result<Vec<_>>>()?;
    let inputs = private
        .iter()
        .map(|key| SharedKeyBoxInput {
            seed: &key.seed,
            generation: key.generation,
            role: key.role,
            receiver_id: target_id,
            receiver_host: None,
            receiver_hepk: &target.hepk,
            receiver_role: target.role,
            receiver_generation: target.generation,
        })
        .collect::<Vec<_>>();
    let randomness = (0..inputs.len())
        .map(|_| random_box_randomness())
        .collect::<Result<Vec<_>>>()?;
    let sender = derive_shared_public(&actor_puk.seed, ENTITY_PUK_VERIFY)?;
    Ok(seal_shared_key_boxes(
        host.host_id(),
        &actor_puk.seed,
        &sender.hepk,
        random_bytes()?,
        &inputs,
        &randomness,
    )?)
}

pub(super) fn random_box_randomness() -> Result<PukBoxRandomness> {
    Ok(PukBoxRandomness {
        kem_message: random_bytes()?,
        nonce: random_bytes()?,
    })
}

fn addition_operation_id(
    actor: &EntityId,
    team: &EntityId,
    binding: &AdditionBinding<'_>,
) -> Result<[u8; 16]> {
    let identity = encode(&Value::Array(vec![
        Value::Binary(actor.as_bytes().to_vec()),
        Value::Binary(team.as_bytes().to_vec()),
        Value::Binary(binding.target_id.as_bytes().to_vec()),
        Value::Binary(binding.target.verify_key.as_bytes().to_vec()),
        Value::Unsigned(binding.target.generation),
        binding.target.role.to_value(),
        binding.destination_role.to_value(),
        Value::Unsigned(binding.expected_seqno),
        Value::Binary(binding.removal_key_commitment.to_vec()),
    ]))?;
    let hash = prefixed_hash(TEAM_MUTATION_OPERATION_ID_TYPE_ID, &identity);
    Ok(hash[..16].try_into().expect("hash prefix has fixed length"))
}

fn validate_addition_transition(
    team: &VerifiedTeamState,
    binding: &AdditionBinding<'_>,
) -> Result<()> {
    let change = team.group_change_at(binding.expected_seqno)?;
    let [member] = change.changes.as_slice() else {
        return Err(Error::OperationBinding(
            "prepared addition sequence contains another roster transition",
        ));
    };
    let keys = member.keys.as_ref().ok_or(Error::OperationBinding(
        "prepared addition sequence removes rather than adds a member",
    ))?;
    if !change.shared_keys.is_empty()
        || member.party != *binding.target_id
        || member.source_role != binding.target.role
        || member.role != binding.destination_role
        || keys.generation != binding.target.generation
        || keys.verify_key != binding.target.verify_key
        || keys.removal_key_commitment != Some(binding.removal_key_commitment)
    {
        return Err(Error::OperationBinding(
            "observed team transition does not match the prepared addition",
        ));
    }
    Ok(())
}

fn validate_addition_operation(
    operation: &TeamMutationOperation,
    host: &PinnedHost,
    actor: &EntityId,
    device_id: &EntityId,
    team: &EntityId,
    expected_seqno: u64,
) -> Result<()> {
    if operation.kind != TeamMutationKind::MembershipChange
        || operation.host_id != host.host_id().as_bytes()
        || operation.actor_id != actor.as_bytes()
        || operation.device_id != device_id.as_bytes()
        || operation.team_id != team.as_bytes()
        || operation.expected_seqno != expected_seqno
    {
        return Err(Error::OperationBinding(
            "team-member addition journal does not match supplied identities",
        ));
    }
    Ok(())
}

pub(super) fn finish_team_mutation_journal(
    store: &mut HardStateStore,
    operation_id: &[u8; 16],
) -> Result<()> {
    let operation = store
        .team_mutation(operation_id)?
        .ok_or(Error::OperationBinding(
            "team mutation disappeared during reconciliation",
        ))?;
    if operation.state == TeamMutationState::Prepared {
        store.advance_team_mutation(
            operation_id,
            TeamMutationState::Submitted,
            now_microseconds()?,
        )?;
    }
    if operation.state != TeamMutationState::Verified {
        store.advance_team_mutation(
            operation_id,
            TeamMutationState::Verified,
            now_microseconds()?,
        )?;
    }
    Ok(())
}
