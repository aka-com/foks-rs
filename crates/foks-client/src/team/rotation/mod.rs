//! Named-team membership changes with mandatory PTK rotation.

mod reconcile;
mod support;
mod types;

use support::*;
pub use types::*;

use foks_client_db::{HardStateStore, TeamMutationKind, TeamMutationOperation, TeamMutationState};
use foks_crypto::{
    derive_shared_public, derive_subkey_id, make_change_team_member_link,
    make_change_team_members_link, make_team_removal_proof, prefixed_hash, seal_puk_seed_chain_box,
    team_removal_key_commitment, ChangeTeamMemberEntryInput, ChangeTeamMemberInput,
    ChangeTeamMembersInput, SharedPublicMaterial, TeamPtkRotation,
};
use foks_proto::{
    EntityId, RemoveTeamMemberArgument, Role, SecretSeed, TeamRemovalMacPayload, TreeRoot,
    ENTITY_NAMED_TEAM, MERKLE_ROOT_TYPE_ID,
};
use foks_rpc::{decode_team_edit_result, encode_remove_team_member_request, STATUS_TX_RETRY_ERROR};
use foks_snowpack::{encode, Value};
use foks_verify::{VerifiedSharedKey, VerifiedTeamMemberState, VerifiedUserState};

use super::membership::{authorized_actor_member, finish_team_mutation_journal};
use super::{AuthenticatedTeamOutcome, TeamPrivateKey};
use crate::{
    now_microseconds, now_milliseconds, random_bytes, user_key_history_for_seed,
    AuthenticatedUserOutcome, DeviceCredential, Error, FoksClient, PinnedHost,
    ProtectedMutationStore, ProtectedStoreError, Result, UserPrivateKey, YubiCredential,
    TEAM_MUTATION_OPERATION_ID_TYPE_ID, TEAM_MUTATION_REQUEST_HASH_TYPE_ID,
};

enum RefreshActorSecrets<'a> {
    User(&'a AuthenticatedUserOutcome),
    Team(&'a AuthenticatedTeamOutcome),
}

struct RefreshActor<'a> {
    party: &'a EntityId,
    source: VerifiedSharedKey,
    signing_seed: &'a SecretSeed,
    recipient: VerifiedMemberParty<'a>,
    root: TreeRoot,
    key_type: u8,
    secrets: RefreshActorSecrets<'a>,
}

impl RefreshActor<'_> {
    fn replacement_seed(&self, replacement: &VerifiedSharedKey) -> Result<Option<&SecretSeed>> {
        match self.secrets {
            RefreshActorSecrets::User(user) => Ok(user.puks.iter().find_map(|private| {
                (private.role == replacement.role
                    && private.generation == replacement.generation
                    && user_key_history_for_seed(&user.verified, &private.seed)
                        .is_ok_and(|public| public.verify_key == replacement.verify_key))
                .then_some(&private.seed)
            })),
            RefreshActorSecrets::Team(team) => {
                let history = super::verified_team_private_history(team)?;
                Ok(team.ptks.iter().find_map(|private| {
                    super::team_key_for_seed(&history, private)
                        .is_ok_and(|public| {
                            public.role == replacement.role
                                && public.generation == replacement.generation
                                && public.verify_key == replacement.verify_key
                        })
                        .then_some(&private.seed)
                }))
            }
        }
    }
}

fn local_team_refresh_actor<'a>(
    actor: &'a AuthenticatedTeamOutcome,
    recipient: &'a VerifiedTeamRecipient,
    target: &AuthenticatedTeamOutcome,
) -> Result<RefreshActor<'a>> {
    if actor.verified.team() != recipient.verified().team()
        || actor.verified.host() != recipient.verified().host()
        || actor.verified.chain_tail_hash() != recipient.verified().chain_tail_hash()
        || actor.verified.tree_root() != recipient.verified().tree_root()
        || actor.verified.host() != target.verified.host()
    {
        return Err(Error::TeamBinding(
            "local team actor witness does not match its current authenticated state",
        ));
    }
    let history = super::verified_team_private_history(actor)?;
    let mut candidates = Vec::new();
    for member in target.verified.members().iter().filter(|member| {
        member.party == *actor.verified.team()
            && member.scoped_host.is_none()
            && matches!(
                member.role.kind(),
                foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
            )
    }) {
        for private in &actor.ptks {
            if let Ok(public) = super::team_key_for_seed(&history, private) {
                if public.role == member.source_role
                    && public.generation == member.generation
                    && public.verify_key == member.verify_key
                    && foks_crypto::hepk_fingerprint(&public.hepk)? == member.hepk_fingerprint
                {
                    candidates.push((public.clone(), private, member.role));
                }
            }
        }
    }
    let (source, private, _) = candidates
        .into_iter()
        .max_by_key(|(source, _, destination)| (*destination, source.role))
        .ok_or(Error::TeamBinding(
            "no authenticated historical PTK matches the team administrator roster",
        ))?;
    Ok(RefreshActor {
        party: actor.verified.team(),
        source,
        signing_seed: &private.seed,
        recipient: VerifiedMemberParty::Team(recipient),
        root: actor.verified.tree_root(),
        key_type: foks_proto::ENTITY_PTK_VERIFY,
        secrets: RefreshActorSecrets::Team(actor),
    })
}

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

struct RefreshChange {
    party: EntityId,
    host: Option<EntityId>,
    source_role: Role,
    destination_role: Role,
    generation: u64,
    verify_key: EntityId,
    hepk_fingerprint: [u8; 32],
}

struct RefreshBinding {
    changes: Vec<RefreshChange>,
    expected_seqno: u64,
    introduced: Vec<(Role, u64, EntityId, [u8; 32])>,
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

fn team_actor_puk<'a>(
    user: &'a AuthenticatedUserOutcome,
    team: &AuthenticatedTeamOutcome,
) -> Result<&'a UserPrivateKey> {
    let matching = user.puks.iter().filter_map(|private| {
        let Ok(public) = user_key_history_for_seed(&user.verified, &private.seed) else {
            return None;
        };
        let member = team.verified.members().iter().find(|member| {
            member.party == *user.verified.uid()
                && member.scoped_host.is_none()
                && member.source_role == public.role
                && member.generation == public.generation
                && member.verify_key.as_bytes()[1..] == public.verify_key.as_bytes()[1..]
                && matches!(
                    member.role.kind(),
                    foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                )
        })?;
        (private.role == public.role && private.generation == public.generation).then_some((
            private,
            member.role,
            public.role,
        ))
    });
    let signer = matching
        .max_by_key(|(_, destination, source)| (*destination, *source))
        .map(|(private, _, _)| private)
        .ok_or(Error::TeamBinding(
            "no authenticated historical PUK matches the team administrator roster",
        ))?;
    Ok(signer)
}

fn require_federated_removal_target(selector: TeamMemberSelector<'_>) -> Result<()> {
    let host = selector.host.ok_or(Error::TeamRequest(
        "retained-key expulsion requires the exact remote host",
    ))?;
    host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if !matches!(
        selector.party.entity_type(),
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
    ) || selector.source_role == Role::NONE
    {
        return Err(Error::TeamRequest(
            "retained-key expulsion requires an exact remote team selector",
        ));
    }
    Ok(())
}

fn prepared_rotation_operation_id(
    actor: &EntityId,
    team: &EntityId,
    authenticated_team: &AuthenticatedTeamOutcome,
    selector: TeamMemberSelector<'_>,
    destination_role: Role,
    replacement: Option<VerifiedMemberParty<'_>>,
    supplied_rotations: &[TeamPtkRotationSeed<'_>],
) -> Result<[u8; 16]> {
    team.clone().require_type(ENTITY_NAMED_TEAM)?;
    if authenticated_team.verified.team() != team {
        return Err(Error::OperationBinding(
            "team mutation identity does not match the authenticated team",
        ));
    }
    let target = unique_target(&authenticated_team.verified, selector)?;
    let replacement_key = validate_replacement(target, destination_role, replacement)?;
    let expected_seqno = authenticated_team
        .verified
        .chain_seqno()
        .checked_add(1)
        .ok_or(Error::TeamRequest("team sequence overflow"))?;
    let rotations = validate_rotation_seeds(
        authenticated_team,
        target,
        destination_role,
        replacement_key.map(|(_, key)| key.generation),
        supplied_rotations,
    )?;
    let commitment = target
        .removal_key_commitment
        .ok_or(Error::TeamBinding("team member has no removal commitment"))?;
    let binding = rotation_binding(
        selector,
        commitment,
        destination_role,
        replacement_key.map(|(_, key)| (key.generation, key.verify_key.clone())),
        expected_seqno,
        &rotations,
    )?;
    rotation_operation_id(actor, team, &binding)
}

fn prepared_refresh_operation_id(
    actor: &EntityId,
    team: &EntityId,
    authenticated_team: &AuthenticatedTeamOutcome,
    request: &RefreshTeamMemberKeysRequest<'_>,
) -> Result<[u8; 16]> {
    if authenticated_team.verified.team() != team
        || authenticated_team.verified.chain_seqno().checked_add(1) != Some(request.expected_seqno)
    {
        return Err(Error::OperationBinding(
            "team member-key refresh operation identity does not match the authenticated team head",
        ));
    }
    let rotations =
        validate_refresh_rotation_seeds(authenticated_team, request.changes, request.rotations)?;
    let binding = refresh_binding_from_request(request, &rotations)?;
    refresh_operation_id(actor, team, &binding)
}

impl FoksClient {
    /// Returns the exact ordered PTK roles required for a member demotion,
    /// removal, or source-key generation advance. Promotion is rejected: it
    /// needs a distinct key-distribution transaction and is not safe to model
    /// as this rotation primitive.
    pub fn team_member_rotation_roles(
        &self,
        authenticated_team: &AuthenticatedTeamOutcome,
        selector: TeamMemberSelector<'_>,
        destination_role: Role,
        replacement: Option<VerifiedMemberParty<'_>>,
    ) -> Result<Vec<Role>> {
        let target = unique_target(&authenticated_team.verified, selector)?;
        let replacement_generation = validate_replacement(target, destination_role, replacement)?
            .map(|(_, key)| key.generation);
        Ok(required_rotation_roles(
            authenticated_team
                .verified
                .shared_keys()
                .iter()
                .map(|key| key.role),
            target.role,
            target.generation,
            destination_role,
            replacement_generation,
        ))
    }

    /// Derives the stable journal identity for one caller-durable team member-key refresh plan
    /// before any protected request or public journal row is written.
    pub fn refresh_team_member_keys_operation_id(
        &self,
        credential: &DeviceCredential,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
    ) -> Result<[u8; 16]> {
        prepared_refresh_operation_id(&credential.uid, team, authenticated_team, request)
    }

    /// Hardware-backed identity derivation for a caller-durable team member-key refresh plan.
    /// Credential kind and signing device are deliberately absent from the
    /// identity: the authenticated user, team transition, and retained PTKs
    /// bind the same operation across software and Yubi recovery paths.
    pub fn refresh_team_member_keys_operation_id_yubi(
        &self,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
    ) -> Result<[u8; 16]> {
        prepared_refresh_operation_id(&credential.uid, team, authenticated_team, request)
    }

    /// Stable team member-key refresh identity when a local member team, rather than the
    /// transport user, signs the target-team transition.
    pub fn refresh_team_member_keys_operation_id_for_actor(
        &self,
        actor: &EntityId,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
    ) -> Result<[u8; 16]> {
        prepared_refresh_operation_id(actor, team, authenticated_team, request)
    }

    /// Derives the journal identity for a generalized member edit before the
    /// request is submitted. Callers retaining rotation seeds should persist
    /// this ID beside them so an ambiguous result can be resumed exactly.
    pub fn change_team_member_and_rotate_ptks_operation_id(
        &self,
        actor: &EntityId,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &ChangeTeamMemberRequest<'_>,
    ) -> Result<[u8; 16]> {
        prepared_rotation_operation_id(
            actor,
            team,
            authenticated_team,
            request.target,
            request.destination_role,
            request.replacement,
            request.rotations,
        )
    }

    /// Preflight identity for the local-user removal convenience API.
    pub fn remove_local_user_and_rotate_ptks_operation_id(
        &self,
        actor: &EntityId,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RemoveLocalTeamMemberRequest<'_>,
    ) -> Result<[u8; 16]> {
        let target = unique_target(
            &authenticated_team.verified,
            TeamMemberSelector {
                party: request.target_user,
                host: None,
                source_role: Role::OWNER,
            },
        )?;
        if target.removal_key_commitment != Some(team_removal_key_commitment(request.removal_key)?)
        {
            return Err(Error::KeyBinding(
                "removal key does not match the authenticated member commitment",
            ));
        }
        prepared_rotation_operation_id(
            actor,
            team,
            authenticated_team,
            TeamMemberSelector {
                party: request.target_user,
                host: None,
                source_role: Role::OWNER,
            },
            Role::NONE,
            None,
            request.rotations,
        )
    }

    /// Preflight identity for removal-key retrieval followed by team edit.
    pub fn remove_team_member_and_rotate_ptks_operation_id(
        &self,
        actor: &EntityId,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RemoveTeamMemberRequest<'_>,
    ) -> Result<[u8; 16]> {
        prepared_rotation_operation_id(
            actor,
            team,
            authenticated_team,
            TeamMemberSelector {
                party: request.target,
                host: None,
                source_role: Role::OWNER,
            },
            Role::NONE,
            None,
            request.rotations,
        )
    }

    /// Preflight identity for an exact fully-qualified federated-team
    /// expulsion using the admission-time retained removal key.
    pub fn retained_team_member_removal_operation_id(
        &self,
        actor: &EntityId,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RetainedTeamMemberRemovalRequest<'_>,
    ) -> Result<[u8; 16]> {
        require_federated_removal_target(request.target)?;
        let target = unique_target(&authenticated_team.verified, request.target)?;
        if target.removal_key_commitment != Some(team_removal_key_commitment(request.removal_key)?)
        {
            return Err(Error::KeyBinding(
                "retained removal key does not match the exact federated member commitment",
            ));
        }
        prepared_rotation_operation_id(
            actor,
            team,
            authenticated_team,
            request.target,
            Role::NONE,
            None,
            request.rotations,
        )
    }

    /// Removes one local user from a named team and rotates every PTK the
    /// removed member could read, following FOKS v0.1.9's exact gameplan.
    pub fn remove_local_user_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &RemoveLocalTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let device_id = credential.public_material()?.id;
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
            actor_puk,
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
            protected_store,
        )
    }

    /// Hardware-backed variant of [`Self::remove_local_user_and_rotate_ptks`].
    pub fn remove_local_user_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        request: &RemoveLocalTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team_yubi(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
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
            actor_puk,
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
            protected_store,
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
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let device_id = credential.public_material()?.id;
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
            actor_puk,
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
            protected_store,
        )
    }

    /// Hardware-backed variant of [`Self::remove_team_member_and_rotate_ptks`].
    pub fn remove_team_member_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        request: &RemoveTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team_yubi(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
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
            actor_puk,
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
            protected_store,
        )
    }

    /// Expels one exact federated team with its retained admission key. All
    /// parties except that fully-qualified row receive every affected rotated
    /// PTK; the old admission remains usable until the resulting chain head is
    /// authenticated by `change_with_material`.
    pub fn remove_retained_team_member_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &RetainedTeamMemberRemovalRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        require_federated_removal_target(request.target)?;
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let owner = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let device_id = credential.public_material()?.id;
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
            Role::NONE,
            None,
            Some(request.removal_key),
            request.rotations,
            request.remaining_parties,
            protected_store,
        )
    }

    pub fn change_team_member_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &ChangeTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let owner = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let device_id = credential.public_material()?.id;
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
            protected_store,
        )
    }

    pub fn change_team_member_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        request: &ChangeTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team_yubi(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let owner = team_actor_puk(&authenticated_user, &authenticated_team)?;
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
            protected_store,
        )
    }

    /// Atomically advances multiple stale roster PUK generations and rotates
    /// the union of exposed PTKs. This is the team member-key refresh primitive: no retired key
    /// receives any PTK introduced by the transition.
    pub fn refresh_team_member_keys_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let actor_source =
            user_key_history_for_seed(&authenticated_user.verified, &actor_puk.seed)?.clone();
        let device_id = credential.public_material()?.id;
        self.refresh_team_member_keys_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_team,
            RefreshActor {
                party: &credential.uid,
                source: actor_source,
                signing_seed: &actor_puk.seed,
                recipient: VerifiedMemberParty::User(&authenticated_user.verified),
                root: authenticated_user.verified.tree_root(),
                key_type: foks_proto::ENTITY_PUK_VERIFY,
                secrets: RefreshActorSecrets::User(&authenticated_user),
            },
            team,
            request,
            protected_store,
        )
    }

    /// Hardware-backed variant of
    /// [`Self::refresh_team_member_keys_and_rotate_ptks`]. The parent Yubi
    /// remains the enrolled device identity while its delegated software
    /// subkey is used only for mTLS, exactly as in other Yubi team edits.
    pub fn refresh_team_member_keys_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team_yubi(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let actor_source =
            user_key_history_for_seed(&authenticated_user.verified, &actor_puk.seed)?.clone();
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
        self.refresh_team_member_keys_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_team,
            RefreshActor {
                party: &credential.uid,
                source: actor_source,
                signing_seed: &actor_puk.seed,
                recipient: VerifiedMemberParty::User(&authenticated_user.verified),
                root: authenticated_user.verified.tree_root(),
                key_type: foks_proto::ENTITY_PUK_VERIFY,
                secrets: RefreshActorSecrets::User(&authenticated_user),
            },
            team,
            request,
            protected_store,
        )
    }

    /// Runs team member-key refresh with a local member team as the cryptographic actor while an
    /// ordinary software device supplies only the authenticated transport.
    /// The target must have been loaded through that member team's view
    /// capability and `actor_recipient` must recursively prove the member
    /// team's complete roster is current and nonstale.
    #[allow(clippy::too_many_arguments)]
    pub fn refresh_team_member_keys_and_rotate_ptks_as_local_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        transport_user: &AuthenticatedUserOutcome,
        actor_team: &AuthenticatedTeamOutcome,
        actor_recipient: &VerifiedTeamRecipient,
        target_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        if transport_user.verified.uid() != &credential.uid
            || transport_user.verified.host() != host.host_id()
        {
            return Err(Error::UserBinding(
                "nested team member-key refresh transport does not match the authenticated user",
            ));
        }
        let actor = local_team_refresh_actor(actor_team, actor_recipient, target_team)?;
        let device_id = credential.public_material()?.id;
        self.refresh_team_member_keys_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            target_team,
            actor,
            target_team.verified.team(),
            request,
            protected_store,
        )
    }

    /// Hardware-backed transport variant of
    /// [`Self::refresh_team_member_keys_and_rotate_ptks_as_local_team`].
    #[allow(clippy::too_many_arguments)]
    pub fn refresh_team_member_keys_and_rotate_ptks_as_local_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        transport_user: &AuthenticatedUserOutcome,
        actor_team: &AuthenticatedTeamOutcome,
        actor_recipient: &VerifiedTeamRecipient,
        target_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        if transport_user.verified.uid() != &credential.uid
            || transport_user.verified.host() != host.host_id()
        {
            return Err(Error::UserBinding(
                "nested team member-key refresh transport does not match the authenticated user",
            ));
        }
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if !transport_user.verified.devices().iter().any(|device| {
            device.id == *credential.parent.entity_id()
                && device.hepk == *credential.parent.hepk()
                && device.subkey.as_ref() == Some(&subkey)
        }) {
            return Err(Error::CredentialBinding(
                "nested team member-key refresh Yubi transport is not enrolled",
            ));
        }
        let actor = local_team_refresh_actor(actor_team, actor_recipient, target_team)?;
        self.refresh_team_member_keys_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            target_team,
            actor,
            target_team.verified.team(),
            request,
            protected_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn refresh_team_member_keys_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        authenticated_team: &AuthenticatedTeamOutcome,
        actor: RefreshActor<'_>,
        team: &EntityId,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let actor_public = &actor.source;
        let actor_member =
            authorized_actor_member(&authenticated_team.verified, actor.party, actor_public)?;
        if request.changes.is_empty()
            || authenticated_team.verified.chain_seqno().checked_add(1)
                != Some(request.expected_seqno)
        {
            return Err(Error::TeamRequest(
                "member-key refresh sequence is stale or has no changes",
            ));
        }

        let mut replacement_publics = Vec::with_capacity(request.changes.len());
        let mut self_replacement = None;
        for change in request.changes {
            let target = unique_target(&authenticated_team.verified, change.target)?;
            if actor_member.role < target.role {
                return Err(Error::TeamRequest(
                    "team editor cannot refresh a higher-role member",
                ));
            }
            let (_, replacement) =
                validate_replacement(target, change.destination_role, change.replacement)?.ok_or(
                    Error::TeamRequest("member-key refresh lacks replacement keys"),
                )?;
            if change.destination_role != target.role
                || replacement.generation <= target.generation
                || replacement.generation != change.replacement_generation
                || replacement.verify_key != *change.replacement_verify_key
                || foks_crypto::hepk_fingerprint(&replacement.hepk)?
                    != change.replacement_hepk_fingerprint
            {
                return Err(Error::TeamRequest(
                    "member-key refresh does not advance its generation",
                ));
            }
            if target.party == *actor.party
                && target.scoped_host.is_none()
                && target.source_role == actor_public.role
            {
                let private = actor
                    .replacement_seed(replacement)?
                    .ok_or(Error::KeyBinding(
                        "current replacement actor key is unavailable for self-refresh",
                    ))?;
                if self_replacement.replace(private).is_some() {
                    return Err(Error::TeamRequest(
                        "member-key refresh contains duplicate actor changes",
                    ));
                }
            }
            replacement_publics.push(SharedPublicMaterial {
                verify_key: change.replacement_verify_key.clone(),
                hepk: replacement.hepk.clone(),
            });
        }
        let rotation_keys = validate_refresh_rotation_seeds(
            authenticated_team,
            request.changes,
            request.rotations,
        )?;
        let rotated_roles = rotation_keys
            .iter()
            .map(|rotation| rotation.role)
            .collect::<Vec<_>>();
        let receivers = resolve_refresh_receivers(
            &authenticated_team.verified,
            request.changes,
            actor.recipient,
            request.remaining_parties,
            &rotated_roles,
        )?;
        let sender_seed = self_replacement.unwrap_or(actor.signing_seed);
        let actor_is_refreshed = self_replacement.is_some();
        let new_key_on_rotate = self_replacement
            .map(|_| derive_shared_public(sender_seed, actor.key_type))
            .transpose()?
            .map(|public| public.verify_key);

        let (_, merkle) = self.advance_merkle_root(host)?;
        let root = TreeRoot {
            epoch: merkle.root().epoch,
            hash: foks_crypto::prefixed_hash_signable(
                MERKLE_ROOT_TYPE_ID,
                &merkle.root().encoded()?,
            )?,
        };
        if actor.root != root
            || authenticated_team.verified.tree_root() != root
            || request
                .changes
                .iter()
                .filter_map(|change| change.replacement)
                .any(|party| !party.matches_authoritative_root(host.host_id(), &root))
            || request
                .remaining_parties
                .iter()
                .any(|party| !party.matches_authoritative_root(host.host_id(), &root))
        {
            return Err(Error::TeamRequest(
                "team member-key refresh snapshot does not match the latest authenticated Merkle root",
            ));
        }
        // The actor is deliberately omitted from `remaining_parties`: it is
        // supplied separately so its signing credentials can be selected. A
        // nested actor can nevertheless contain remote descendants, whose
        // authenticated heads must be rechecked just like every receiver
        // before we emit boxes or a signature.
        actor.recipient.ensure_current_heads(self)?;
        for party in request
            .changes
            .iter()
            .filter_map(|change| change.replacement)
            .chain(request.remaining_parties.iter().copied())
        {
            party.ensure_current_heads(self)?;
        }
        let next_tree_location = random_bytes()?;
        let member_inputs = request
            .changes
            .iter()
            .zip(&replacement_publics)
            .map(|(change, public)| ChangeTeamMemberEntryInput {
                member: change.target.party,
                member_host: change.target.host,
                member_source_role: change.target.source_role,
                destination_role: change.destination_role,
                member_generation: Some(change.replacement_generation),
                member_public: Some(public),
                member_index_range: change
                    .replacement
                    .and_then(VerifiedMemberParty::index_range),
            })
            .collect::<Vec<_>>();
        let crypto_rotations = rotation_keys
            .iter()
            .map(|rotation| TeamPtkRotation {
                role: rotation.role,
                generation: rotation.generation,
                seed: rotation.seed,
            })
            .collect::<Vec<_>>();
        let link_output = make_change_team_members_link(
            &ChangeTeamMembersInput {
                actor: actor.party,
                actor_source_role: actor_public.role,
                team,
                host: host.host_id(),
                sequence: request.expected_seqno,
                previous: authenticated_team.verified.chain_tail_hash(),
                root: &root,
                time: now_milliseconds()?,
                next_tree_location,
                members: &member_inputs,
            },
            actor.signing_seed,
            &crypto_rotations,
        )?;
        let ptk_boxes =
            box_rotated_ptks(host, actor.party, sender_seed, &rotation_keys, &receivers)?;
        let seed_chain = rotation_keys
            .iter()
            .map(|rotation| {
                seal_puk_seed_chain_box(
                    rotation.seed,
                    &rotation.previous.seed,
                    actor.party,
                    host.host_id(),
                    rotation.previous.generation,
                    rotation.role,
                    random_bytes()?,
                )
                .map_err(Error::from)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut hepk_fingerprints = std::collections::BTreeSet::new();
        let mut hepks = Vec::new();
        for hepk in replacement_publics
            .iter()
            .map(|public| &public.hepk)
            .chain(link_output.ptks.iter().map(|public| &public.hepk))
        {
            let fingerprint = foks_crypto::hepk_fingerprint(hepk)?;
            if hepk_fingerprints.insert(fingerprint) {
                hepks.push(hepk.clone());
            }
        }
        let encoded_request = encode_remove_team_member_request(&RemoveTeamMemberArgument {
            link: &link_output.link,
            next_tree_location: link_output.next_tree_location,
            ptk_boxes: &ptk_boxes,
            seed_chain: &seed_chain,
            removals: &[],
            hepks: &hepks,
            new_key_on_rotate: new_key_on_rotate.as_ref(),
            team_bearer_token: None,
        })?;
        let binding = refresh_binding_from_request(request, &rotation_keys)?;
        let operation_id = refresh_operation_id(actor.party, team, &binding)?;
        let request_key = team_member_key_refresh_request_key(&operation_id);
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        if hard_store.team_mutation(&operation_id)?.is_some() {
            // An existing journal may bind an older recipient manifest even
            // though the public team member-key refresh identity is unchanged. Only the explicit
            // replay/resume APIs revalidate that exact protected frame against
            // the current authenticated roster.
            return Err(Error::OperationBinding(
                "recorded team member-key refresh must be resumed through its exact replay path",
            ));
        }
        // A protected-only frame cannot have reached submission because
        // beginning submission records the public journal atomically. Rebuild
        // it against the freshly authenticated Merkle head.
        remove_team_request(protected_store, &request_key)?;
        match protected_store.put_if_absent(&request_key, &encoded_request) {
            Ok(()) => {}
            Err(ProtectedStoreError::Conflict) => {
                return Err(Error::OperationBinding(
                    "a concurrent team member-key refresh plan changed the protected request",
                ));
            }
            Err(error) => return Err(protected_store_error(error)),
        }
        let protected_request = protected_store
            .get(&request_key)
            .map_err(protected_store_error)?;
        if protected_request.as_slice() != encoded_request {
            return Err(Error::OperationBinding(
                "protected team member-key refresh request changed before journaling",
            ));
        }
        let decoded = decode_protected_team_edit_request(&protected_request)?;
        if decoded.team_bearer_token.is_some()
            || decoded.link.decode_team_group_change()?.team != *team
        {
            return Err(Error::OperationBinding(
                "protected team member-key refresh request contains reusable authority or another team",
            ));
        }
        // Bearer activation is a preflight: it cannot submit the edit. Do it
        // before entering an active journal state so a synchronous authority
        // failure leaves only safely replaceable protected request.
        // Once Submitting is durable, every exit must conservatively assume
        // that the exact edit could have reached the server.
        let bearer = match &actor.secrets {
            RefreshActorSecrets::Team(_) => Some(self.activate_team_admin_bearer(
                host,
                uid,
                auth_seed,
                certificate_chain,
                authenticated_team,
            )?),
            RefreshActorSecrets::User(_) => None,
        };
        let created_at = now_microseconds()?;
        let operation = TeamMutationOperation {
            operation_id,
            kind: TeamMutationKind::PtkRotation,
            host_id: host.host_id().as_bytes().to_vec(),
            actor_id: actor.party.as_bytes().to_vec(),
            device_id: device_id.as_bytes().to_vec(),
            team_id: team.as_bytes().to_vec(),
            expected_seqno: request.expected_seqno,
            request_hash: prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &protected_request),
            state: TeamMutationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        hard_store.record_and_begin_team_mutation(&operation, now_microseconds()?)?;
        let submission_request =
            frame_protected_team_edit_with_bearer(&protected_request, bearer.as_ref())?;
        let post = || {
            let response = self.call_with_material(
                host,
                &host.user,
                &submission_request,
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
        // RPC statuses are not authenticated evidence. A compromised server
        // can commit this exact request and lie with TEAM_RACE, so retain the
        // request and classify every post error as ambiguous until a fresh,
        // latest-root team chain proves our transition or a conflict.
        let mutation_state = if post_error.is_none() {
            TeamMutationState::Submitted
        } else {
            TeamMutationState::SubmissionUnknown
        };
        hard_store.advance_team_mutation(&operation_id, mutation_state, now_microseconds()?)?;

        let mut polling_token = authenticated_team.view_token;
        let mut replacement_token_active = !actor_is_refreshed;

        let mut last_error = None;
        for attempt in 0..40 {
            // A self-refresh invalidates the old generation's view capability.
            // Retry activation sparsely until the roster transaction exposes
            // the replacement PUK, then reuse that one token for all polls.
            if let RefreshActorSecrets::User(user) = actor.secrets {
                if !replacement_token_active && attempt % 4 == 0 {
                    if let Ok(token) = self.activate_team_view_with_material(
                        host,
                        uid,
                        auth_seed,
                        certificate_chain,
                        &user.verified,
                        sender_seed,
                        team,
                    ) {
                        polling_token = token;
                        replacement_token_active = true;
                    }
                }
            }
            let observed = match actor.secrets {
                RefreshActorSecrets::User(user) => self.load_and_pin_team_with_view_token(
                    host,
                    uid,
                    auth_seed,
                    certificate_chain,
                    &user.verified,
                    sender_seed,
                    team,
                    &polling_token,
                ),
                RefreshActorSecrets::Team(actor_team) => self
                    .load_and_pin_team_as_local_team_with_material(
                        host,
                        uid,
                        auth_seed,
                        certificate_chain,
                        actor_team,
                        team,
                    ),
            };
            match observed {
                Ok(observed) if observed.verified.chain_seqno() >= request.expected_seqno => {
                    if let Err(error) = validate_refresh_transition(&observed.verified, &binding) {
                        hard_store.advance_team_mutation(
                            &operation_id,
                            TeamMutationState::Rejected,
                            now_microseconds()?,
                        )?;
                        remove_team_request(protected_store, &request_key)?;
                        return Err(error);
                    }
                    if request.rotations.iter().all(|rotation| {
                        observed
                            .ptks
                            .iter()
                            .any(|ptk| ptk.role == rotation.role && ptk.seed == *rotation.seed)
                    }) {
                        finish_team_mutation_journal(&mut hard_store, &operation_id)?;
                        remove_team_request(protected_store, &request_key)?;
                        return Ok(RotatedTeamPtks {
                            operation_id,
                            expected_seqno: request.expected_seqno,
                            authenticated: observed,
                        });
                    }
                    last_error = Some(Error::TransitionNotObserved(
                        "refreshed PTKs do not match caller-retained secrets",
                    ));
                }
                Ok(_) => {
                    last_error = Some(Error::TransitionNotObserved(
                        "team chain has not reached the prepared team member-key refresh transition",
                    ));
                }
                Err(error) => last_error = Some(error),
            }
            if attempt != 39 {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        Err(post_error.unwrap_or_else(|| {
            last_error.unwrap_or(Error::TransitionNotObserved(
                "team member-key refresh transition was not observed",
            ))
        }))
    }

    /// Finalizes a caller-durable team member-key refresh intent after its exact transition is
    /// visible in the authenticated team chain.
    pub fn resume_refresh_team_member_keys_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        recovery: TeamMutationRecovery<'_>,
        request: &RefreshTeamMemberKeysRequest<'_>,
        expected_actor: &EntityId,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        if recovery.expected_seqno != request.expected_seqno {
            return Err(Error::OperationBinding(
                "team member-key refresh recovery sequence does not match the request",
            ));
        }
        let user = self.authenticate_and_pin(host, credential)?;
        let observed =
            self.load_and_pin_team(host, credential, &user.verified, &user.puks, recovery.team)?;
        self.resume_refresh_team_member_keys_after_load(
            host,
            &credential.uid,
            recovery.team,
            request,
            recovery.expected_operation_id,
            expected_actor,
            observed,
            protected_store,
        )
    }

    /// Hardware-backed reconciliation for a caller-durable team member-key refresh intent. Like
    /// the software path, this never regenerates or reposts key material; it
    /// accepts only the exact authenticated transition bound to the journal.
    pub fn resume_refresh_team_member_keys_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        recovery: TeamMutationRecovery<'_>,
        request: &RefreshTeamMemberKeysRequest<'_>,
        expected_actor: &EntityId,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        if recovery.expected_seqno != request.expected_seqno {
            return Err(Error::OperationBinding(
                "team member-key refresh recovery sequence does not match the request",
            ));
        }
        let user = self.authenticate_yubi_and_pin(host, credential)?;
        let observed = self.load_and_pin_team_yubi(
            host,
            credential,
            &user.verified,
            &user.puks,
            recovery.team,
        )?;
        self.resume_refresh_team_member_keys_after_load(
            host,
            &credential.uid,
            recovery.team,
            request,
            recovery.expected_operation_id,
            expected_actor,
            observed,
            protected_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resume_refresh_team_member_keys_and_rotate_ptks_as_local_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        transport_user: &VerifiedUserState,
        actor_team: &AuthenticatedTeamOutcome,
        team: &EntityId,
        request: &RefreshTeamMemberKeysRequest<'_>,
        expected_operation_id: &[u8; 16],
        expected_actor: &EntityId,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let observed = self.load_and_pin_team_as_local_team(
            host,
            credential,
            transport_user,
            actor_team,
            team,
        )?;
        self.resume_refresh_team_member_keys_after_load(
            host,
            actor_team.verified.team(),
            team,
            request,
            expected_operation_id,
            expected_actor,
            observed,
            protected_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resume_refresh_team_member_keys_and_rotate_ptks_as_local_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        transport_user: &VerifiedUserState,
        actor_team: &AuthenticatedTeamOutcome,
        team: &EntityId,
        request: &RefreshTeamMemberKeysRequest<'_>,
        expected_operation_id: &[u8; 16],
        expected_actor: &EntityId,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let observed = self.load_and_pin_team_as_local_team_yubi(
            host,
            credential,
            transport_user,
            actor_team,
            team,
        )?;
        self.resume_refresh_team_member_keys_after_load(
            host,
            actor_team.verified.team(),
            team,
            request,
            expected_operation_id,
            expected_actor,
            observed,
            protected_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn resume_refresh_team_member_keys_after_load(
        &self,
        host: &PinnedHost,
        observer: &EntityId,
        team: &EntityId,
        request: &RefreshTeamMemberKeysRequest<'_>,
        expected_operation_id: &[u8; 16],
        expected_actor: &EntityId,
        observed: AuthenticatedTeamOutcome,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        if observed.verified.chain_seqno() < request.expected_seqno {
            return Err(Error::TransitionNotObserved(
                "team chain has not reached the caller-durable team member-key refresh transition",
            ));
        }
        let mut changes = Vec::with_capacity(request.changes.len());
        for request_change in request.changes {
            changes.push(RefreshChange {
                party: request_change.target.party.clone(),
                host: request_change.target.host.cloned(),
                source_role: request_change.target.source_role,
                destination_role: request_change.destination_role,
                generation: request_change.replacement_generation,
                verify_key: request_change.replacement_verify_key.clone(),
                hepk_fingerprint: request_change.replacement_hepk_fingerprint,
            });
        }
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation =
            hard_store
                .team_mutation(expected_operation_id)?
                .ok_or(Error::TeamRequest(
                    "team member-key refresh transition is not recorded",
                ))?;
        if operation.kind != TeamMutationKind::PtkRotation
            || operation.host_id != host.host_id().as_bytes()
            || operation.actor_id != expected_actor.as_bytes()
            || operation.team_id != team.as_bytes()
            || operation.expected_seqno != request.expected_seqno
            || operation.operation_id != *expected_operation_id
        {
            return Err(Error::OperationBinding(
                "team member-key refresh journal does not match supplied identities",
            ));
        }
        let request_key = team_member_key_refresh_request_key(&operation.operation_id);
        if operation.state != TeamMutationState::Verified {
            let exact_request = protected_store
                .get(&request_key)
                .map_err(protected_store_error)?;
            if prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &exact_request)
                != operation.request_hash
            {
                return Err(Error::OperationBinding(
                    "protected team member-key refresh request changed before reconciliation",
                ));
            }
            let protected = decode_protected_team_edit_request(&exact_request)?;
            if protected.team_bearer_token.is_some()
                || protected.link.decode_team_group_change()?
                    != observed.verified.group_change_at(request.expected_seqno)?
            {
                hard_store.advance_team_mutation(
                    &operation.operation_id,
                    TeamMutationState::Rejected,
                    now_microseconds()?,
                )?;
                remove_team_request(protected_store, &request_key)?;
                return Err(Error::OperationBinding(
                    "authenticated team transition differs from the exact recorded team member-key refresh request",
                ));
            }
        }
        let binding_result = (|| {
            let change = observed.verified.group_change_at(request.expected_seqno)?;
            if change.shared_keys.len() != request.rotations.len() {
                return Err(Error::OperationBinding(
                    "observed team member-key refresh PTK schedule differs from caller-durable seeds",
                ));
            }
            let introduced = change
                .shared_keys
                .iter()
                .zip(request.rotations)
                .map(|(public, rotation)| {
                    let verify =
                        derive_shared_public(rotation.seed, foks_proto::ENTITY_PTK_VERIFY)?
                            .verify_key;
                    if public.role != rotation.role || public.verify_key != verify {
                        return Err(Error::OperationBinding(
                            "caller-durable PTK does not match the observed team member-key refresh transition",
                        ));
                    }
                    let hepk =
                        derive_shared_public(rotation.seed, foks_proto::ENTITY_PTK_VERIFY)?.hepk;
                    Ok((
                        public.role,
                        public.generation,
                        verify,
                        foks_crypto::hepk_fingerprint(&hepk)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let binding = RefreshBinding {
                changes,
                expected_seqno: request.expected_seqno,
                introduced,
            };
            validate_refresh_transition(&observed.verified, &binding)?;
            Ok(binding)
        })();
        let binding = binding_result?;
        let operation_id = refresh_operation_id(expected_actor, team, &binding)?;
        if operation_id != *expected_operation_id {
            return Err(Error::OperationBinding(
                "team member-key refresh journal identity differs from caller-durable intent",
            ));
        }
        let current_role = observed
            .verified
            .members()
            .iter()
            .filter(|member| member.party == *observer && member.scoped_host.is_none())
            .map(|member| member.role)
            .max();
        if observer != expected_actor
            && current_role.is_none_or(|role| {
                !matches!(
                    role.kind(),
                    foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                )
            })
        {
            return Err(Error::TeamBinding(
                "team member-key refresh observer is not a direct team administrator",
            ));
        }
        if !request
            .rotations
            .iter()
            .filter(|rotation| current_role.is_some_and(|role| rotation.role <= role))
            .all(|rotation| {
                observed
                    .ptks
                    .iter()
                    .any(|ptk| ptk.role == rotation.role && ptk.seed == *rotation.seed)
            })
        {
            return Err(Error::OperationBinding(
                "caller-durable PTKs do not match authenticated team member-key refresh state",
            ));
        }
        if operation.state != TeamMutationState::Verified {
            finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        }
        remove_team_request(
            protected_store,
            &team_member_key_refresh_request_key(&operation.operation_id),
        )?;
        Ok(RotatedTeamPtks {
            operation_id,
            expected_seqno: request.expected_seqno,
            authenticated: observed,
        })
    }

    /// Replays the exact canonical team member-key refresh request durably recorded with its
    /// public journal row. This closes the crash window after sequence
    /// reservation but before the first socket write without rebuilding boxes
    /// or signatures with different randomness.
    pub fn replay_recorded_team_rekey(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        recovery: TeamMutationRecovery<'_>,
        authenticated_team: &AuthenticatedTeamOutcome,
        current_parties: &[VerifiedMemberParty<'_>],
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        self.replay_recorded_team_rekey_with_material(
            host,
            &credential.uid,
            &credential.uid,
            None,
            &credential.seed,
            &credential.certificate_chain,
            recovery.team,
            recovery.expected_seqno,
            recovery.expected_operation_id,
            authenticated_team,
            current_parties,
            protected_store,
        )
    }

    /// Hardware-backed exact replay of a recorded team member-key refresh frame. The frame's
    /// original signatures and boxes remain authoritative; the Yubi delegated
    /// subkey supplies only the authenticated transport used for replay.
    pub fn replay_recorded_team_rekey_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        recovery: TeamMutationRecovery<'_>,
        authenticated_team: &AuthenticatedTeamOutcome,
        current_parties: &[VerifiedMemberParty<'_>],
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        self.replay_recorded_team_rekey_with_material(
            host,
            &credential.uid,
            &credential.uid,
            None,
            &credential.subkey_seed,
            &credential.certificate_chain,
            recovery.team,
            recovery.expected_seqno,
            recovery.expected_operation_id,
            authenticated_team,
            current_parties,
            protected_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replay_recorded_team_rekey_as_local_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        actor_team: &AuthenticatedTeamOutcome,
        team: &EntityId,
        expected_seqno: u64,
        expected_operation_id: &[u8; 16],
        authenticated_team: &AuthenticatedTeamOutcome,
        current_parties: &[VerifiedMemberParty<'_>],
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        self.replay_recorded_team_rekey_with_material(
            host,
            &credential.uid,
            actor_team.verified.team(),
            Some(actor_team),
            &credential.seed,
            &credential.certificate_chain,
            team,
            expected_seqno,
            expected_operation_id,
            authenticated_team,
            current_parties,
            protected_store,
        )
    }

    /// Hardware-backed exact replay for a team member-key refresh transition signed by a local
    /// member team's PTK. The retained frame supplies the team signature; the
    /// unlocked Yubi credential is used only for the authenticated transport.
    #[allow(clippy::too_many_arguments)]
    pub fn replay_recorded_team_rekey_as_local_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        actor_team: &AuthenticatedTeamOutcome,
        team: &EntityId,
        expected_seqno: u64,
        expected_operation_id: &[u8; 16],
        authenticated_team: &AuthenticatedTeamOutcome,
        current_parties: &[VerifiedMemberParty<'_>],
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        self.replay_recorded_team_rekey_with_material(
            host,
            &credential.uid,
            actor_team.verified.team(),
            Some(actor_team),
            &credential.subkey_seed,
            &credential.certificate_chain,
            team,
            expected_seqno,
            expected_operation_id,
            authenticated_team,
            current_parties,
            protected_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_recorded_team_rekey_with_material(
        &self,
        host: &PinnedHost,
        transport_uid: &EntityId,
        actor: &EntityId,
        actor_team: Option<&AuthenticatedTeamOutcome>,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        team: &EntityId,
        expected_seqno: u64,
        expected_operation_id: &[u8; 16],
        authenticated_team: &AuthenticatedTeamOutcome,
        current_parties: &[VerifiedMemberParty<'_>],
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation =
            hard_store
                .team_mutation(expected_operation_id)?
                .ok_or(Error::TeamRequest(
                    "team member-key refresh transition is not recorded",
                ))?;
        if operation.kind != TeamMutationKind::PtkRotation
            || operation.host_id != host.host_id().as_bytes()
            || operation.actor_id != actor.as_bytes()
            || operation.team_id != team.as_bytes()
            || operation.expected_seqno != expected_seqno
            || operation.operation_id != *expected_operation_id
            || !matches!(
                operation.state,
                TeamMutationState::Submitting
                    | TeamMutationState::SubmissionUnknown
                    | TeamMutationState::Submitted
            )
        {
            return Err(Error::OperationBinding(
                "recorded team member-key refresh replay request is missing or changed",
            ));
        }
        let request_key = team_member_key_refresh_request_key(&operation.operation_id);
        let request = protected_store
            .get(&request_key)
            .map_err(protected_store_error)?;
        if prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &request) != operation.request_hash {
            return Err(Error::OperationBinding(
                "protected team member-key refresh replay request changed",
            ));
        }
        let decoded = decode_protected_team_edit_request(&request)?;
        if decoded.team_bearer_token.is_some()
            || decoded.link.decode_team_group_change()?.team != *team
        {
            return Err(Error::OperationBinding(
                "recorded team member-key refresh replay contains reusable authority or another team",
            ));
        }
        let (_, latest) = self.advance_merkle_root(host)?;
        let latest_root = TreeRoot {
            epoch: latest.root().epoch,
            hash: latest
                .authenticated_roots()
                .root_hash(latest.root().epoch)
                .ok_or(Error::OperationBinding(
                    "latest Merkle root is not authenticated",
                ))?,
        };
        if authenticated_team.verified.team() != team
            || authenticated_team.verified.tree_root() != latest_root
        {
            return Err(Error::OperationBinding(
                "team member-key refresh replay team projection is not at the authenticated Merkle head",
            ));
        }
        for party in current_parties {
            party.ensure_current_heads(self)?;
        }
        for boxed in &decoded.ptk_boxes.boxes {
            if !authenticated_team.verified.members().iter().any(|member| {
                member.party == boxed.target.entity
                    && member.scoped_host == boxed.target.host
                    && member.source_role == boxed.target.role
            }) {
                return Err(Error::OperationBinding(
                    "team member-key refresh replay contains a recipient outside the current team roster",
                ));
            }
            let party = current_parties
                .iter()
                .copied()
                .find(|party| {
                    party.party() == &boxed.target.entity
                        && (boxed
                            .target
                            .host
                            .as_ref()
                            .is_some_and(|scope| party.host() == scope)
                            || (boxed.target.host.is_none() && party.host() == host.host_id()))
                })
                .ok_or(Error::OperationBinding(
                    "team member-key refresh replay recipient is not authenticated",
                ))?;
            if !party.matches_authoritative_root(host.host_id(), &latest_root)
                || party.has_stale_shared_key(boxed.target.role)
                || party
                    .shared_key(boxed.target.role)
                    .is_none_or(|key| key.generation != boxed.target.generation)
            {
                return Err(Error::OperationBinding(
                    "team member-key refresh replay recipient key is stale or not at the Merkle head",
                ));
            }
        }
        let bearer = match actor_team {
            Some(authority)
                if authority.verified.team() == actor
                    && matches!(
                        actor.entity_type(),
                        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
                    ) =>
            {
                Some(self.activate_team_admin_bearer(
                    host,
                    transport_uid,
                    auth_seed,
                    certificate_chain,
                    authenticated_team,
                )?)
            }
            None if actor.entity_type() == foks_proto::ENTITY_USER => None,
            _ => {
                return Err(Error::TeamBinding(
                    "nested team member-key refresh replay lacks the actor team's current bearer authority",
                ))
            }
        };
        let submission_request = frame_protected_team_edit_with_bearer(&request, bearer.as_ref())?;
        let posted = self.call_with_material(
            host,
            &host.user,
            &submission_request,
            auth_seed,
            certificate_chain,
        );
        let decoded_response = posted.and_then(|response| {
            decode_team_edit_result(&response)?;
            Ok(())
        });
        match decoded_response {
            Ok(_) => {
                if operation.state != TeamMutationState::Submitted {
                    hard_store.advance_team_mutation(
                        &operation.operation_id,
                        TeamMutationState::Submitted,
                        now_microseconds()?,
                    )?;
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Releases an unsubmitted or already-terminal team member-key refresh journal reservation.
    /// Active submission states require an authenticated conflict witness and
    /// are rejected only by the reconciliation path that verifies that
    /// witness; caller-supplied identifiers alone are never sufficient.
    pub fn reject_recorded_team_rekey(
        &self,
        host: &PinnedHost,
        team: &EntityId,
        expected_seqno: u64,
        expected_operation_id: &[u8; 16],
        expected_actor: &EntityId,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation =
            hard_store
                .team_mutation(expected_operation_id)?
                .ok_or(Error::TeamRequest(
                    "team member-key refresh transition is not recorded",
                ))?;
        if operation.kind != TeamMutationKind::PtkRotation
            || operation.host_id != host.host_id().as_bytes()
            || operation.actor_id != expected_actor.as_bytes()
            || operation.team_id != team.as_bytes()
            || operation.expected_seqno != expected_seqno
            || operation.operation_id != *expected_operation_id
        {
            return Err(Error::OperationBinding(
                "recorded team member-key refresh rejection does not match supplied identities",
            ));
        }
        if !matches!(
            operation.state,
            TeamMutationState::Prepared
                | TeamMutationState::Rejected
                | TeamMutationState::Superseded
        ) {
            return Err(Error::OperationBinding(
                "active team member-key refresh submission requires an authenticated conflict witness",
            ));
        }
        if !matches!(
            operation.state,
            TeamMutationState::Rejected | TeamMutationState::Superseded
        ) {
            hard_store.advance_team_mutation(
                &operation.operation_id,
                TeamMutationState::Rejected,
                now_microseconds()?,
            )?;
        }
        remove_team_request(
            protected_store,
            &team_member_key_refresh_request_key(&operation.operation_id),
        )
    }

    /// Removes protected team member-key refresh request only when no public journal row proves
    /// that submission could have begun. Callers use this to replace a stale
    /// pre-journal plan after re-authenticating all roster parties.
    pub fn discard_unrecorded_team_rekey(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        self.discard_unrecorded_team_rekey_for_actor(
            host,
            &credential.uid,
            team,
            authenticated_team,
            request,
            protected_store,
        )
    }

    /// Hardware-backed identity wrapper for discarding a protected-only team member-key refresh
    /// plan. No hardware operation is needed because a missing journal proves
    /// submission never began; the Yubi credential contributes only its UID.
    pub fn discard_unrecorded_team_rekey_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        self.discard_unrecorded_team_rekey_for_actor(
            host,
            &credential.uid,
            team,
            authenticated_team,
            request,
            protected_store,
        )
    }

    pub fn discard_unrecorded_team_rekey_for_local_team_actor(
        &self,
        host: &PinnedHost,
        actor: &EntityId,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        self.discard_unrecorded_team_rekey_for_actor(
            host,
            actor,
            team,
            authenticated_team,
            request,
            protected_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn discard_unrecorded_team_rekey_for_actor(
        &self,
        host: &PinnedHost,
        actor: &EntityId,
        team: &EntityId,
        authenticated_team: &AuthenticatedTeamOutcome,
        request: &RefreshTeamMemberKeysRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        if authenticated_team.verified.chain_seqno().checked_add(1) != Some(request.expected_seqno)
        {
            return Err(Error::OperationBinding(
                "unrecorded team member-key refresh is not adjacent to the authenticated team head",
            ));
        }
        let binding = RefreshBinding {
            changes: request
                .changes
                .iter()
                .map(|change| RefreshChange {
                    party: change.target.party.clone(),
                    host: change.target.host.cloned(),
                    source_role: change.target.source_role,
                    destination_role: change.destination_role,
                    generation: change.replacement_generation,
                    verify_key: change.replacement_verify_key.clone(),
                    hepk_fingerprint: change.replacement_hepk_fingerprint,
                })
                .collect(),
            expected_seqno: request.expected_seqno,
            introduced: request
                .rotations
                .iter()
                .map(|rotation| {
                    let current = authenticated_team
                        .verified
                        .shared_key(rotation.role)
                        .ok_or(Error::OperationBinding(
                            "unrecorded team member-key refresh role is absent from the team head",
                        ))?;
                    let public =
                        derive_shared_public(rotation.seed, foks_proto::ENTITY_PTK_VERIFY)?;
                    Ok((
                        rotation.role,
                        current
                            .generation
                            .checked_add(1)
                            .ok_or(Error::TeamRequest("team PTK generation overflow"))?,
                        public.verify_key,
                        foks_crypto::hepk_fingerprint(&public.hepk)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        };
        let operation_id = refresh_operation_id(actor, team, &binding)?;
        self.discard_unjournaled_team_rekey_request(host, &operation_id, protected_store)
    }

    /// Removes one protected pre-journal team member-key refresh frame only when no journal row
    /// exists for that exact operation identity. A different operation at the
    /// same sequence must never be mistaken for this caller-durable plan.
    pub fn discard_unjournaled_team_rekey_request(
        &self,
        host: &PinnedHost,
        operation_id: &[u8; 16],
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        if HardStateStore::open(&host.database_path)?
            .team_mutation(operation_id)?
            .is_some()
        {
            return Err(Error::OperationBinding(
                "journaled team member-key refresh request cannot be discarded as unrecorded",
            ));
        }
        remove_team_request(
            protected_store,
            &team_member_key_refresh_request_key(operation_id),
        )
    }

    /// Removes residual protected request after the public journal proves
    /// that this exact team member-key refresh transition was already authenticated. No current
    /// roster authority is needed because this path cannot submit or accept
    /// another transition.
    #[allow(clippy::too_many_arguments)]
    pub fn cleanup_verified_team_rekey_request(
        &self,
        host: &PinnedHost,
        team: &EntityId,
        expected_seqno: u64,
        operation_id: &[u8; 16],
        expected_actor: &EntityId,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        let operation = HardStateStore::open(&host.database_path)?
            .team_mutation(operation_id)?
            .ok_or(Error::TeamRequest(
                "verified team member-key refresh transition is not recorded",
            ))?;
        if operation.kind != TeamMutationKind::PtkRotation
            || operation.host_id != host.host_id().as_bytes()
            || operation.actor_id != expected_actor.as_bytes()
            || operation.team_id != team.as_bytes()
            || operation.expected_seqno != expected_seqno
            || operation.operation_id != *operation_id
            || operation.state != TeamMutationState::Verified
        {
            return Err(Error::OperationBinding(
                "verified team member-key refresh cleanup does not match supplied identities",
            ));
        }
        remove_team_request(
            protected_store,
            &team_member_key_refresh_request_key(&operation.operation_id),
        )
    }

    /// Reconciles a journaled generalized member transition without needing
    /// the removal key or reposting the signed edit.
    pub fn resume_change_team_member_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        recovery: TeamMutationRecovery<'_>,
        request: &ChangeTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            recovery.team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let device_id = credential.public_material()?.id;
        self.resume_change_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            &actor_puk.seed,
            recovery.team,
            recovery.expected_seqno,
            recovery.expected_operation_id,
            request,
            None,
            protected_store,
        )
    }

    /// Reconciles or exactly replays a crash-interrupted retained-key
    /// federated expulsion.
    pub fn resume_retained_team_member_removal(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        recovery: TeamMutationRecovery<'_>,
        request: &RetainedTeamMemberRemovalRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        require_federated_removal_target(request.target)?;
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            recovery.team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let device_id = credential.public_material()?.id;
        let change = ChangeTeamMemberRequest {
            target: request.target,
            destination_role: Role::NONE,
            replacement: None,
            rotations: request.rotations,
            remaining_parties: request.remaining_parties,
        };
        self.resume_change_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            &actor_puk.seed,
            recovery.team,
            recovery.expected_seqno,
            recovery.expected_operation_id,
            &change,
            Some(request.removal_key),
            protected_store,
        )
    }

    /// Completes a member edit already visible in the authenticated team
    /// chain using only its journal identity and caller-retained PTK seeds.
    /// This recovery path never reposts bytes and therefore remains safe when
    /// a target user rotates again after the committed transition.
    pub fn finish_recorded_team_member_change(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        recovery: TeamMutationRecovery<'_>,
        removal_key_commitment: [u8; 32],
        rotations: &[TeamPtkRotationSeed<'_>],
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            recovery.team,
        )?;
        if authenticated.verified.chain_seqno() < recovery.expected_seqno {
            return Err(Error::TransitionNotObserved(
                "team chain has not reached the recorded member edit",
            ));
        }
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation = hard_store
            .team_mutation(recovery.expected_operation_id)?
            .ok_or(Error::TeamRequest("team transition is not recorded"))?;
        validate_rotation_operation(
            &operation,
            host,
            &credential.uid,
            recovery.team,
            recovery.expected_seqno,
        )?;
        if matches!(
            operation.state,
            TeamMutationState::Rejected | TeamMutationState::Superseded
        ) {
            return Err(Error::OperationBinding("team transition is terminal"));
        }
        let observed_change = authenticated
            .verified
            .group_change_at(recovery.expected_seqno)?;
        let [observed_member] = observed_change.changes.as_slice() else {
            return Err(Error::OperationBinding(
                "recorded team transition differs from the authenticated chain",
            ));
        };
        if observed_change.team != *recovery.team
            || rotations.len() != observed_change.shared_keys.len()
        {
            return Err(Error::OperationBinding(
                "recorded team transition differs from the authenticated chain",
            ));
        }
        let introduced = rotations
            .iter()
            .zip(&observed_change.shared_keys)
            .map(|(rotation, observed)| {
                let public = derive_shared_public(rotation.seed, foks_proto::ENTITY_PTK_VERIFY)?;
                if rotation.role != observed.role || public.verify_key != observed.verify_key {
                    return Err(Error::OperationBinding(
                        "caller-retained PTKs do not match the committed transition",
                    ));
                }
                Ok((rotation.role, observed.generation, public.verify_key))
            })
            .collect::<Result<Vec<_>>>()?;
        let binding = RotationBinding {
            target: observed_member.party.clone(),
            target_host: observed_member.scoped_host.clone(),
            target_source_role: observed_member.source_role,
            removal_key_commitment,
            destination_role: observed_member.role,
            replacement: observed_member
                .keys
                .as_ref()
                .map(|keys| (keys.generation, keys.verify_key.clone())),
            expected_seqno: recovery.expected_seqno,
            introduced,
        };
        if rotation_operation_id(&credential.uid, recovery.team, &binding)?
            != *recovery.expected_operation_id
        {
            return Err(Error::OperationBinding(
                "authenticated team transition has another operation identity",
            ));
        }
        validate_rotation_change(&observed_change, &binding)?;
        for (rotation, public) in rotations.iter().zip(&observed_change.shared_keys) {
            let verify = derive_shared_public(rotation.seed, foks_proto::ENTITY_PTK_VERIFY)?;
            if rotation.role != public.role
                || verify.verify_key != public.verify_key
                || !authenticated
                    .ptks
                    .iter()
                    .any(|ptk| ptk.role == rotation.role && ptk.seed == *rotation.seed)
            {
                return Err(Error::OperationBinding(
                    "caller-retained PTKs do not match the committed transition",
                ));
            }
        }
        finish_team_mutation_journal(&mut hard_store, recovery.expected_operation_id)?;
        remove_team_request(
            protected_store,
            &team_member_change_request_key(recovery.expected_operation_id),
        )?;
        Ok(RotatedTeamPtks {
            operation_id: *recovery.expected_operation_id,
            expected_seqno: recovery.expected_seqno,
            authenticated,
        })
    }

    /// Marks a recorded member edit terminal after an authenticated, different
    /// transition occupied its reserved sequence. This is intentionally
    /// separate from reconciliation: callers invoke it only after observing
    /// an operation-binding failure at or beyond `expected_seqno`.
    pub fn supersede_recorded_team_member_change(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        expected_seqno: u64,
        expected_operation_id: &[u8; 16],
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<()> {
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation = hard_store
            .team_mutation(expected_operation_id)?
            .ok_or(Error::TeamRequest("team transition is not recorded"))?;
        validate_rotation_operation(&operation, host, &credential.uid, team, expected_seqno)?;
        match operation.state {
            TeamMutationState::Verified => {
                return Err(Error::OperationBinding(
                    "verified team transition cannot be superseded",
                ));
            }
            TeamMutationState::Rejected | TeamMutationState::Superseded => {}
            _ => hard_store.advance_team_mutation(
                expected_operation_id,
                TeamMutationState::Superseded,
                now_microseconds()?,
            )?,
        }
        remove_team_request(
            protected_store,
            &team_member_change_request_key(expected_operation_id),
        )
    }

    pub fn resume_change_team_member_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        recovery: TeamMutationRecovery<'_>,
        request: &ChangeTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team_yubi(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            recovery.team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        self.resume_change_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            &actor_puk.seed,
            recovery.team,
            recovery.expected_seqno,
            recovery.expected_operation_id,
            request,
            None,
            protected_store,
        )
    }

    /// Reconciles an already journaled removal without reposting it.
    pub fn resume_remove_local_user_and_rotate_ptks(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        recovery: TeamMutationRecovery<'_>,
        request: &RemoveLocalTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            recovery.team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        let device_id = credential.public_material()?.id;
        self.resume_removal_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            actor_puk,
            recovery.team,
            recovery.expected_seqno,
            recovery.expected_operation_id,
            request,
            protected_store,
        )
    }

    /// Hardware-backed reconciliation variant. It never reposts the edit.
    pub fn resume_remove_local_user_and_rotate_ptks_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        recovery: TeamMutationRecovery<'_>,
        request: &RemoveLocalTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<RotatedTeamPtks> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let authenticated_team = self.load_and_pin_team_yubi(
            host,
            credential,
            &authenticated_user.verified,
            &authenticated_user.puks,
            recovery.team,
        )?;
        let actor_puk = team_actor_puk(&authenticated_user, &authenticated_team)?;
        self.resume_removal_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            actor_puk,
            recovery.team,
            recovery.expected_seqno,
            recovery.expected_operation_id,
            request,
            protected_store,
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
        protected_store: &mut dyn ProtectedMutationStore,
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
        let actor_public = user_key_history_for_seed(&actor_user.verified, &actor_puk.seed)?;
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
        let observer_puk = if self_target {
            let (_, replacement_public) = replacement_key.ok_or(Error::TeamRequest(
                "self team-member refresh requires replacement PUK keys",
            ))?;
            actor_user
                .puks
                .iter()
                .find(|private| {
                    private.role == replacement_public.role
                        && private.generation == replacement_public.generation
                        && user_key_history_for_seed(&actor_user.verified, &private.seed)
                            .is_ok_and(|public| public.verify_key == replacement_public.verify_key)
                })
                .ok_or(Error::KeyBinding(
                    "current replacement PUK is unavailable for team reconciliation",
                ))?
        } else {
            actor_puk
        };
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
        let rotated_roles = rotation_keys
            .iter()
            .map(|rotation| rotation.role)
            .collect::<Vec<_>>();
        let receivers = resolve_remaining_receivers(
            &authenticated_team.verified,
            target,
            destination_role,
            replacement,
            &actor_user.verified,
            remaining_parties,
            &rotated_roles,
        )?;
        let (_, merkle) = self.advance_merkle_root(host)?;
        let root = TreeRoot {
            epoch: merkle.root().epoch,
            hash: foks_crypto::prefixed_hash_signable(
                MERKLE_ROOT_TYPE_ID,
                &merkle.root().encoded()?,
            )?,
        };
        if actor_user.verified.tree_root() != root
            || authenticated_team.verified.tree_root() != root
            || replacement
                .is_some_and(|party| !party.matches_authoritative_root(host.host_id(), &root))
            || remaining_parties
                .iter()
                .any(|party| !party.matches_authoritative_root(host.host_id(), &root))
        {
            return Err(Error::TeamRequest(
                "team mutation snapshot does not match the latest authenticated Merkle root",
            ));
        }
        if let Some(replacement) = replacement {
            replacement.ensure_current_heads(self)?;
        }
        for party in remaining_parties {
            party.ensure_current_heads(self)?;
        }
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
        let link_output = make_change_team_member_link(
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
                member_index_range: if destination_role == Role::NONE {
                    None
                } else {
                    replacement.and_then(VerifiedMemberParty::index_range)
                },
            },
            &actor_puk.seed,
            &crypto_rotations,
        )?;
        let box_sender = if self_target { observer_puk } else { actor_puk };
        let new_key_on_rotate = if self_target {
            Some(derive_shared_public(&box_sender.seed, foks_proto::ENTITY_PUK_VERIFY)?.verify_key)
        } else {
            None
        };
        let ptk_boxes = box_rotated_ptks(host, uid, &box_sender.seed, &rotation_keys, &receivers)?;
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
        let removals = if destination_role < target.role {
            vec![make_team_removal_proof(
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
                    root: root.clone(),
                    time,
                },
            )?]
        } else {
            Vec::new()
        };
        let mut seen_hepks = std::collections::BTreeSet::new();
        let mut hepks = Vec::new();
        for hepk in replacement_public
            .iter()
            .map(|public| &public.hepk)
            .chain(link_output.ptks.iter().map(|ptk| &ptk.hepk))
        {
            let fingerprint = foks_crypto::hepk_fingerprint(hepk)?;
            if seen_hepks.insert(fingerprint) {
                hepks.push(hepk.clone());
            }
        }
        let encoded_request = encode_remove_team_member_request(&RemoveTeamMemberArgument {
            link: &link_output.link,
            next_tree_location: link_output.next_tree_location,
            ptk_boxes: &ptk_boxes,
            seed_chain: &seed_chain,
            removals: &removals,
            hepks: &hepks,
            new_key_on_rotate: new_key_on_rotate.as_ref(),
            team_bearer_token: None,
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
        let request_key = team_member_change_request_key(&operation_id);
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let recorded = hard_store.team_mutation(&operation_id)?;
        if recorded.is_none() {
            // A protected-only record proves submission never began. Replace
            // it with the freshly authenticated head-bound frame.
            remove_team_request(protected_store, &request_key)?;
        }
        match protected_store.put_if_absent(&request_key, &encoded_request) {
            Ok(()) | Err(ProtectedStoreError::Conflict) => {}
            Err(error) => return Err(protected_store_error(error)),
        }
        let protected_request = protected_store
            .get(&request_key)
            .map_err(protected_store_error)?;
        let decoded = decode_protected_team_edit_request(&protected_request)?;
        let protected_change = decoded.link.decode_team_group_change()?;
        validate_rotation_change(&protected_change, &binding)?;
        if decoded.team_bearer_token.is_some()
            || protected_change.team != *team
            || (recorded.is_none() && protected_change.root != root)
        {
            return Err(Error::OperationBinding(
                "protected team mutation is not bound to the authenticated head",
            ));
        }
        if recorded.is_some() {
            let targets = (!merkle
                .authenticated_roots()
                .contains_epoch(protected_change.root.epoch))
            .then_some(protected_change.root.epoch)
            .into_iter()
            .collect();
            let roots =
                self.authenticate_chain_roots(host, &merkle, targets, Error::TeamBinding)?;
            if roots.root_hash(protected_change.root.epoch) != Some(protected_change.root.hash) {
                return Err(Error::OperationBinding(
                    "journaled team mutation cites an unauthenticated Merkle root",
                ));
            }
        }
        let expected_boxes = rotation_keys
            .iter()
            .flat_map(|rotation| {
                receivers
                    .iter()
                    .filter(move |receiver| rotation.role <= receiver.member.role)
                    .map(move |receiver| {
                        (
                            rotation.role,
                            rotation.generation,
                            receiver.member.party.as_bytes().to_vec(),
                            receiver
                                .member
                                .scoped_host
                                .as_ref()
                                .map(|host| host.as_bytes().to_vec()),
                            receiver.member.source_role,
                            receiver.member.generation,
                        )
                    })
            })
            .collect::<std::collections::BTreeSet<_>>();
        let actual_boxes = decoded
            .ptk_boxes
            .boxes
            .iter()
            .map(|boxed| {
                (
                    boxed.role,
                    boxed.generation,
                    boxed.target.entity.as_bytes().to_vec(),
                    boxed
                        .target
                        .host
                        .as_ref()
                        .map(|host| host.as_bytes().to_vec()),
                    boxed.target.role,
                    boxed.target.generation,
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        if actual_boxes != expected_boxes || decoded.ptk_boxes.boxes.len() != expected_boxes.len() {
            return Err(Error::OperationBinding(
                "protected team mutation recipient manifest changed",
            ));
        }
        let created_at = now_microseconds()?;
        let operation = TeamMutationOperation {
            operation_id,
            kind: TeamMutationKind::PtkRotation,
            host_id: host.host_id().as_bytes().to_vec(),
            actor_id: uid.as_bytes().to_vec(),
            device_id: device_id.as_bytes().to_vec(),
            team_id: team.as_bytes().to_vec(),
            expected_seqno,
            request_hash: prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &protected_request),
            state: TeamMutationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        let replay_state = if let Some(recorded) = recorded {
            validate_rotation_operation(&recorded, host, uid, team, expected_seqno)?;
            if recorded.operation_id != operation.operation_id
                || recorded.request_hash != operation.request_hash
                || !matches!(
                    recorded.state,
                    TeamMutationState::Submitting
                        | TeamMutationState::SubmissionUnknown
                        | TeamMutationState::Submitted
                )
            {
                return Err(Error::OperationBinding(
                    "journaled team mutation cannot be replayed",
                ));
            }
            Some(recorded.state)
        } else {
            hard_store.record_and_begin_team_mutation(&operation, now_microseconds()?)?;
            None
        };
        let post = || {
            let response = self.call_with_material(
                host,
                &host.user,
                &protected_request,
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
        let observed_state = if post_error.is_none() {
            TeamMutationState::Submitted
        } else {
            TeamMutationState::SubmissionUnknown
        };
        if replay_state.is_none_or(|state| {
            state == TeamMutationState::Submitting
                || (state == TeamMutationState::SubmissionUnknown
                    && observed_state == TeamMutationState::Submitted)
        }) {
            hard_store.advance_team_mutation(&operation_id, observed_state, now_microseconds()?)?;
        }
        let authenticated = match self.wait_for_rotation(
            host,
            uid,
            auth_seed,
            certificate_chain,
            actor_user,
            &observer_puk.seed,
            team,
            &binding,
            supplied_rotations,
        ) {
            Ok(value) => value,
            Err(wait_error) => {
                // Both success and error replies are unauthenticated. Reconcile
                // every failed observation against the latest-root chain before
                // deciding whether this sequence committed or conflicted.
                match self.authenticated_rotation_outcome(
                    host,
                    uid,
                    auth_seed,
                    certificate_chain,
                    actor_user,
                    &observer_puk.seed,
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
                        remove_team_request(protected_store, &request_key)?;
                        return Err(post_error.unwrap_or(wait_error));
                    }
                    // Not yet observable (never-arrived or Merkle-lagged): retain
                    // the ambiguous journal. Status replies are not authenticated
                    // evidence that can safely release caller-held key link_output.
                    Ok(RotationOutcome::Unresolved) | Err(_) => {
                        return Err(post_error.unwrap_or(wait_error));
                    }
                }
            }
        };
        finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        remove_team_request(protected_store, &request_key)?;
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

fn refresh_operation_id(
    actor: &EntityId,
    team: &EntityId,
    binding: &RefreshBinding,
) -> Result<[u8; 16]> {
    let identity = encode(&Value::Array(vec![
        Value::Binary(actor.as_bytes().to_vec()),
        Value::Binary(team.as_bytes().to_vec()),
        Value::Unsigned(binding.expected_seqno),
        Value::Array(
            binding
                .changes
                .iter()
                .map(|change| {
                    Value::Array(vec![
                        Value::Binary(change.party.as_bytes().to_vec()),
                        change
                            .host
                            .as_ref()
                            .map_or(Value::Null, |host| Value::Binary(host.as_bytes().to_vec())),
                        change.source_role.to_value(),
                        change.destination_role.to_value(),
                        Value::Unsigned(change.generation),
                        Value::Binary(change.verify_key.as_bytes().to_vec()),
                        Value::Binary(change.hepk_fingerprint.to_vec()),
                    ])
                })
                .collect(),
        ),
        Value::Array(
            binding
                .introduced
                .iter()
                .map(|(role, generation, verify, hepk)| {
                    Value::Array(vec![
                        role.to_value(),
                        Value::Unsigned(*generation),
                        Value::Binary(verify.as_bytes().to_vec()),
                        Value::Binary(hepk.to_vec()),
                    ])
                })
                .collect(),
        ),
    ]))?;
    let hash = prefixed_hash(TEAM_MUTATION_OPERATION_ID_TYPE_ID, &identity);
    Ok(hash[..16].try_into().expect("hash prefix has fixed length"))
}

fn refresh_binding_from_request(
    request: &RefreshTeamMemberKeysRequest<'_>,
    rotations: &[ValidatedRotation<'_>],
) -> Result<RefreshBinding> {
    Ok(RefreshBinding {
        changes: request
            .changes
            .iter()
            .map(|change| RefreshChange {
                party: change.target.party.clone(),
                host: change.target.host.cloned(),
                source_role: change.target.source_role,
                destination_role: change.destination_role,
                generation: change.replacement_generation,
                verify_key: change.replacement_verify_key.clone(),
                hepk_fingerprint: change.replacement_hepk_fingerprint,
            })
            .collect(),
        expected_seqno: request.expected_seqno,
        introduced: rotations
            .iter()
            .map(|rotation| {
                Ok((
                    rotation.role,
                    rotation.generation,
                    rotation.verify_key.clone(),
                    foks_crypto::hepk_fingerprint(
                        &derive_shared_public(rotation.seed, foks_proto::ENTITY_PTK_VERIFY)?.hepk,
                    )?,
                ))
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

fn team_member_key_refresh_request_key(operation_id: &[u8; 16]) -> Vec<u8> {
    crate::ProtectedRecordKey::TeamRekey(operation_id).encoded()
}

fn team_member_change_request_key(operation_id: &[u8; 16]) -> Vec<u8> {
    crate::ProtectedRecordKey::TeamRotation(operation_id).encoded()
}

pub(super) fn decode_protected_team_edit_request(
    request: &[u8],
) -> Result<foks_proto::DecodedTeamEditArgument> {
    let mut framed = std::io::Cursor::new(request);
    let call =
        foks_rpc::read_call(&mut framed, foks_rpc::DEFAULT_MAX_FRAME_LENGTH).map_err(|_| {
            Error::OperationBinding("protected team member-key refresh request is malformed")
        })?;
    if usize::try_from(framed.position()).ok() != Some(request.len())
        || call.protocol_id() != foks_rpc::TEAM_ADMIN_PROTOCOL_ID
        || call.method_position() != foks_rpc::TEAM_EDIT_METHOD_POSITION
    {
        return Err(Error::OperationBinding(
            "protected team member-key refresh request targets another route",
        ));
    }
    Ok(foks_proto::DecodedTeamEditArgument::decode(
        call.argument(),
    )?)
}

fn frame_protected_team_edit_with_bearer(
    request: &[u8],
    bearer: Option<&foks_proto::TeamBearerToken>,
) -> Result<Vec<u8>> {
    let decoded = decode_protected_team_edit_request(request)?;
    if decoded.team_bearer_token.is_some()
        || !decoded.removal_keys.is_empty()
        || !decoded.remote_member_view_tokens.is_empty()
        || !decoded.local_permissions_for.is_empty()
    {
        return Err(Error::OperationBinding(
            "protected team member-key refresh request contains reusable or unrelated authority",
        ));
    }
    Ok(encode_remove_team_member_request(
        &RemoveTeamMemberArgument {
            link: &decoded.link,
            next_tree_location: decoded.next_tree_location,
            ptk_boxes: &decoded.ptk_boxes,
            seed_chain: &decoded.seed_chain,
            removals: &decoded.removals,
            hepks: &decoded.hepks,
            new_key_on_rotate: decoded.new_key_on_rotate.as_ref(),
            team_bearer_token: bearer,
        },
    )?)
}

fn protected_store_error(error: ProtectedStoreError) -> Error {
    Error::ProtectedStore(error.to_string())
}

fn remove_team_request(store: &mut dyn ProtectedMutationStore, key: &[u8]) -> Result<()> {
    match store.remove(key) {
        Ok(()) | Err(ProtectedStoreError::Missing) => Ok(()),
        Err(error) => Err(protected_store_error(error)),
    }
}

fn validate_refresh_transition(
    team: &foks_verify::VerifiedTeamState,
    binding: &RefreshBinding,
) -> Result<()> {
    let change = team.group_change_at(binding.expected_seqno)?;
    if change.changes.len() != binding.changes.len()
        || change
            .changes
            .iter()
            .zip(&binding.changes)
            .any(|(member, expected)| {
                member.party != expected.party
                    || member.scoped_host != expected.host
                    || member.source_role != expected.source_role
                    || member.role != expected.destination_role
                    || member.keys.as_ref().is_none_or(|keys| {
                        keys.generation != expected.generation
                            || keys.verify_key != expected.verify_key
                            || keys.hepk_fingerprint != expected.hepk_fingerprint
                    })
            })
        || change.shared_keys.len() != binding.introduced.len()
        || change
            .shared_keys
            .iter()
            .zip(&binding.introduced)
            .any(|(key, expected)| {
                key.role != expected.0
                    || key.generation != expected.1
                    || key.verify_key != expected.2
                    || key.hepk_fingerprint != expected.3
            })
    {
        return Err(Error::OperationBinding(
            "observed team transition does not match the prepared team member-key refresh batch",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
