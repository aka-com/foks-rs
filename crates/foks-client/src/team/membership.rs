//! Named-team membership additions and crash reconciliation.

use std::time::Duration;

use foks_client_db::{HardStateStore, TeamMutationKind, TeamMutationOperation, TeamMutationState};
use foks_crypto::{
    derive_device_public, derive_shared_public, derive_subkey_id, make_add_local_team_member_link,
    make_add_remote_team_member_link, open_team_remote_member_view_token, prefixed_hash,
    seal_shared_key_boxes, seal_team_remote_member_view_token, seal_team_removal_key,
    AddLocalTeamMemberInput, AddRemoteTeamMemberInput, PukBoxRandomness, SharedKeyBoxInput,
    SharedPublicMaterial,
};
use foks_proto::{
    AddTeamMemberArgument, EntityId, FqParty, PermissionToken, RemoteTeamRsvp, Role, RoleType,
    SecretSeed, TeamRemoteMemberViewToken, TeamRemoteMemberViewTokenBoxPayload,
    TeamRemoteMemberViewTokenInner, TeamRemovalKeyMetadata, TreeRoot, ENTITY_NAMED_TEAM,
    ENTITY_PUK_VERIFY, MERKLE_ROOT_TYPE_ID,
};
use foks_rpc::{
    decode_team_edit_result, encode_add_team_member_request,
    encode_load_team_remote_view_tokens_request, STATUS_TEAM_RACE_ERROR, STATUS_TX_RETRY_ERROR,
};
use foks_snowpack::{encode, Value};
use foks_verify::{
    VerifiedSharedKey, VerifiedTeamMemberState, VerifiedTeamState, VerifiedUserState,
};

use super::{AuthenticatedTeamOutcome, TeamPrivateKey};
use crate::{
    current_owner_puk, now_microseconds, random_bytes, user_key_for_seed, AuthenticatedUserOutcome,
    DeviceCredential, Error, FoksClient, PinnedHost, ProtectedMutationStore, ProtectedStoreError,
    RemoteTeamOutcome, Result, UserPrivateKey, YubiCredential, TEAM_MUTATION_OPERATION_ID_TYPE_ID,
    TEAM_MUTATION_REQUEST_HASH_TYPE_ID,
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

/// Authenticated material for admitting a remote named/ad-hoc team to a
/// local named team. `remote_team` must have been loaded with the supplied
/// permission, so its admin PTK is independently verified before boxing.
pub struct AddRemoteTeamMemberRequest<'a> {
    pub remote_team: &'a RemoteTeamOutcome,
    pub destination_role: Role,
    pub removal_key: &'a SecretSeed,
}

pub struct AddedLocalTeamMember {
    pub operation_id: [u8; 16],
    pub expected_seqno: u64,
    pub authenticated: AuthenticatedTeamOutcome,
}

pub type AddedRemoteTeamMember = AddedLocalTeamMember;

#[derive(Debug)]
pub struct RemoteMemberViewPermission {
    pub member: FqParty,
    pub permission: PermissionToken,
}

struct AdditionBinding<'a> {
    target_id: &'a EntityId,
    target_host: Option<&'a EntityId>,
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

    /// Admits a previously verified remote team to a local named team and
    /// stores its remote-view bearer inside a PTK-authenticated opaque box.
    pub fn add_remote_team_to_named_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &AddRemoteTeamMemberRequest<'_>,
    ) -> Result<AddedRemoteTeamMember> {
        self.add_remote_team_to_named_team_for_saga(host, credential, team, request, None, None)
    }

    pub(crate) fn add_remote_team_to_named_team_for_saga(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        request: &AddRemoteTeamMemberRequest<'_>,
        saga_id: Option<&[u8; 16]>,
        protected_store: Option<&mut dyn ProtectedMutationStore>,
    ) -> Result<AddedRemoteTeamMember> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        self.add_remote_team_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            team,
            request,
            saga_id,
            protected_store,
        )
    }

    /// Retrieves and opens only the requested remote roster members' opaque
    /// bearer boxes. The server sees the requested fully-qualified parties,
    /// but never the enclosed permission tokens.
    pub fn load_remote_member_view_permissions(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        members: &[FqParty],
    ) -> Result<Vec<RemoteMemberViewPermission>> {
        let user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&user)?;
        let loaded = self.load_and_pin_team(
            host,
            credential,
            &user.verified,
            std::slice::from_ref(owner),
            team,
        )?;
        let mut requested = std::collections::BTreeSet::new();
        for member in members {
            if member.host == *host.host_id()
                || !requested.insert((
                    member.party.as_bytes().to_vec(),
                    member.host.as_bytes().to_vec(),
                ))
                || !loaded.verified.members().iter().any(|candidate| {
                    candidate.party == member.party
                        && candidate.scoped_host.as_ref() == Some(&member.host)
                })
            {
                return Err(Error::TeamBinding(
                    "remote token request is not a unique verified roster subset",
                ));
            }
        }
        let request = encode_load_team_remote_view_tokens_request(
            &foks_proto::FqTeam::new(team.clone(), host.host_id().clone())?,
            &loaded.view_token,
            members,
        )?;
        let response = self.call(host, &host.user, &request, Some(credential))?;
        let set = foks_proto::TeamRemoteViewTokenSet::decode(&response)?;
        if set.tokens.len() > members.len() {
            return Err(Error::TeamBinding(
                "server returned more remote token boxes than requested",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        set.tokens
            .into_iter()
            .map(|boxed| {
                let tuple = (
                    boxed.member.party.as_bytes().to_vec(),
                    boxed.member.host.as_bytes().to_vec(),
                );
                if !requested.contains(&tuple) || !seen.insert(tuple) {
                    return Err(Error::TeamBinding(
                        "server returned an unrequested or duplicate remote token box",
                    ));
                }
                let private = loaded
                    .ptks
                    .iter()
                    .find(|key| {
                        key.role == boxed.ptk_role && key.generation == boxed.ptk_generation
                    })
                    .ok_or(Error::KeyBinding(
                        "remote token box references an unavailable PTK",
                    ))?;
                // `load_and_pin_team` authenticates the current PTK against the
                // team chain, then opens every older generation through the
                // current parcel's authenticated seed chain. A remote-view box
                // deliberately remains readable after an unrelated PTK
                // rotation, so its historical generation need not be present
                // in `VerifiedTeamState`, which retains only current keys.
                let payload = open_team_remote_member_view_token(&private.seed, &boxed.secret_box)?;
                if payload.party != boxed.member {
                    return Err(Error::TeamBinding(
                        "decrypted remote token belongs to another party",
                    ));
                }
                Ok(RemoteMemberViewPermission {
                    member: boxed.member,
                    permission: payload.token,
                })
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn add_remote_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk: &UserPrivateKey,
        team: &EntityId,
        request: &AddRemoteTeamMemberRequest<'_>,
        saga_id: Option<&[u8; 16]>,
        protected_store: Option<&mut dyn ProtectedMutationStore>,
    ) -> Result<AddedRemoteTeamMember> {
        self.require_open_user_viewership(host, auth_seed, certificate_chain)?;
        team.clone().require_type(ENTITY_NAMED_TEAM)?;
        let remote_id = request.remote_team.verified.team();
        let remote_host = request.remote_team.verified.host();
        if remote_host == host.host_id()
            || remote_id == team
            || request.destination_role == Role::NONE
        {
            return Err(Error::TeamRequest(
                "remote team, host, or destination role is invalid",
            ));
        }
        let target = request
            .remote_team
            .verified
            .shared_key(Role::ADMIN)
            .ok_or(Error::KeyBinding("remote team has no current admin PTK"))?;
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
        if authenticated_team.verified.members().iter().any(|member| {
            member.party == *remote_id && member.scoped_host.as_ref() == Some(remote_host)
        }) {
            return Err(Error::TeamRequest("remote team is already a member"));
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
        let member_floor_public = authenticated_team
            .verified
            .shared_key(Role::member(0))
            .ok_or(Error::KeyBinding("team has no member-load-floor PTK"))?;
        let member_floor_private =
            current_team_private_key(&authenticated_team, member_floor_public)?;
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
        let material = make_add_remote_team_member_link(
            &AddRemoteTeamMemberInput {
                actor: uid,
                actor_source_role: actor_public.role,
                team,
                host: host.host_id(),
                sequence: expected_seqno,
                previous: authenticated_team.verified.chain_tail_hash(),
                root: &root,
                time: now_microseconds()?,
                next_tree_location: random_bytes()?,
                member: remote_id,
                member_host: remote_host,
                member_source_role: target.role,
                member_destination_role: request.destination_role,
                member_generation: target.generation,
                member_public: &target_public,
            },
            &actor_puk.seed,
            request.removal_key,
        )?;
        let ptk_boxes = box_visible_ptks_for_remote(
            host,
            actor_puk,
            remote_id,
            remote_host,
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
            member: remote_id.clone(),
            member_host: remote_host.clone(),
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
                "remote member removal box does not match signed commitment",
            ));
        }
        let member = FqParty::new(remote_id.clone(), remote_host.clone())?;
        let token_payload = TeamRemoteMemberViewTokenBoxPayload {
            token: request.remote_team.permission.clone(),
            party: member.clone(),
            time: now_microseconds()?,
        };
        let secret_box = seal_team_remote_member_view_token(
            &member_floor_private.seed,
            &token_payload,
            random_bytes()?,
        )?;
        let mut join_request = random_bytes()?;
        join_request[0] = 56;
        let remote_token = TeamRemoteMemberViewToken {
            team: team.clone(),
            inner: TeamRemoteMemberViewTokenInner {
                member,
                ptk_generation: member_floor_private.generation,
                secret_box,
                ptk_role: member_floor_private.role,
            },
            join_request: RemoteTeamRsvp::new(join_request)?,
        };
        let encoded_request = encode_add_team_member_request(&AddTeamMemberArgument {
            link: &material.link,
            next_tree_location: material.next_tree_location,
            ptk_boxes: &ptk_boxes,
            removal_keys: &[removal_box],
            hepks: std::slice::from_ref(&target.hepk),
            remote_member_view_tokens: &[remote_token],
            local_permissions_for: &[],
        })?;
        let binding = AdditionBinding {
            target_id: remote_id,
            target_host: Some(remote_host),
            target,
            destination_role: request.destination_role,
            removal_key_commitment: material.removal_key_commitment,
            expected_seqno,
        };
        self.submit_addition_with_material(
            host,
            uid,
            device_id,
            auth_seed,
            certificate_chain,
            actor_user,
            &actor_puk.seed,
            team,
            &binding,
            &encoded_request,
            saga_id,
            protected_store,
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

    pub(crate) fn resume_add_remote_team_to_named_team_for_saga(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        expected_seqno: u64,
        request: &AddRemoteTeamMemberRequest<'_>,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<AddedRemoteTeamMember> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        team.clone().require_type(ENTITY_NAMED_TEAM)?;
        let remote_id = request.remote_team.verified.team();
        let remote_host = request.remote_team.verified.host();
        if remote_host == host.host_id()
            || remote_id == team
            || request.destination_role == Role::NONE
        {
            return Err(Error::TeamRequest(
                "remote team, host, or destination role is invalid",
            ));
        }
        let target = request
            .remote_team
            .verified
            .shared_key(Role::ADMIN)
            .ok_or(Error::KeyBinding("remote team has no current admin PTK"))?;
        let binding = AdditionBinding {
            target_id: remote_id,
            target_host: Some(remote_host),
            target,
            destination_role: request.destination_role,
            removal_key_commitment: foks_crypto::team_removal_key_commitment(request.removal_key)?,
            expected_seqno,
        };
        let operation_id = addition_operation_id(&credential.uid, team, &binding)?;
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation = hard_store
            .team_mutation(&operation_id)?
            .ok_or(Error::TeamRequest("remote-team addition is not recorded"))?;
        validate_addition_operation(
            &operation,
            host,
            &credential.uid,
            &device_id,
            team,
            expected_seqno,
        )?;
        if matches!(
            operation.state,
            TeamMutationState::Rejected | TeamMutationState::Superseded
        ) {
            return Err(Error::OperationBinding("remote-team addition is terminal"));
        }
        if operation.state == TeamMutationState::Verified {
            let authenticated = self.wait_for_addition(
                host,
                &credential.uid,
                &credential.seed,
                &credential.certificate_chain,
                &authenticated_user,
                &owner.seed,
                team,
                &binding,
            )?;
            return Ok(AddedRemoteTeamMember {
                operation_id,
                expected_seqno,
                authenticated,
            });
        }

        // A non-prepared row may represent a request whose response was lost.
        // Reconcile the authenticated chain before replaying its exact bytes.
        if operation.state != TeamMutationState::Prepared {
            match self.wait_for_addition(
                host,
                &credential.uid,
                &credential.seed,
                &credential.certificate_chain,
                &authenticated_user,
                &owner.seed,
                team,
                &binding,
            ) {
                Ok(authenticated) => {
                    finish_team_mutation_journal(&mut hard_store, &operation_id)?;
                    return Ok(AddedRemoteTeamMember {
                        operation_id,
                        expected_seqno,
                        authenticated,
                    });
                }
                Err(error @ Error::OperationBinding(_)) => {
                    if self.authenticated_addition_conflicts(
                        host,
                        &credential.uid,
                        &credential.seed,
                        &credential.certificate_chain,
                        &authenticated_user,
                        &owner.seed,
                        team,
                        &binding,
                    )? {
                        hard_store.advance_team_mutation(
                            &operation_id,
                            TeamMutationState::Superseded,
                            now_microseconds()?,
                        )?;
                    }
                    return Err(error);
                }
                Err(Error::TransitionNotObserved(_)) => {}
                Err(error) => return Err(error),
            }
        }

        let material = protected_store
            .get(&remote_addition_material_key(&operation_id))
            .map_err(protected_material_error)?;
        if prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &material) != operation.request_hash {
            return Err(Error::OperationBinding(
                "protected remote-team request changed",
            ));
        }
        let first_submission = operation.state == TeamMutationState::Prepared;
        if first_submission {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::Submitting,
                now_microseconds()?,
            )?;
        }
        let post_error = self
            .call_with_material(
                host,
                &host.user,
                &material,
                &credential.seed,
                &credential.certificate_chain,
            )
            .and_then(|response| {
                decode_team_edit_result(&response)?;
                Ok(())
            })
            .err();
        let post_error = if first_submission {
            match post_error {
                None => {
                    hard_store.advance_team_mutation(
                        &operation_id,
                        TeamMutationState::Submitted,
                        now_microseconds()?,
                    )?;
                    None
                }
                Some(error) => {
                    hard_store.advance_team_mutation(
                        &operation_id,
                        TeamMutationState::SubmissionUnknown,
                        now_microseconds()?,
                    )?;
                    Some(error)
                }
            }
        } else {
            if post_error.is_none() {
                hard_store.advance_team_mutation(
                    &operation_id,
                    TeamMutationState::Submitted,
                    now_microseconds()?,
                )?;
            }
            post_error
        };

        let authenticated = match self.wait_for_addition(
            host,
            &credential.uid,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            team,
            &binding,
        ) {
            Ok(authenticated) => authenticated,
            Err(error @ Error::OperationBinding(_)) => {
                if self.authenticated_addition_conflicts(
                    host,
                    &credential.uid,
                    &credential.seed,
                    &credential.certificate_chain,
                    &authenticated_user,
                    &owner.seed,
                    team,
                    &binding,
                )? {
                    hard_store.advance_team_mutation(
                        &operation_id,
                        TeamMutationState::Superseded,
                        now_microseconds()?,
                    )?;
                }
                return Err(error);
            }
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        Ok(AddedRemoteTeamMember {
            operation_id,
            expected_seqno,
            authenticated,
        })
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
            remote_member_view_tokens: &[],
            local_permissions_for: std::slice::from_ref(request.target_user.uid()),
        })?;
        let binding = AdditionBinding {
            target_id: request.target_user.uid(),
            target_host: None,
            target,
            destination_role: request.destination_role,
            removal_key_commitment: material.removal_key_commitment,
            expected_seqno,
        };
        self.submit_addition_with_material(
            host,
            uid,
            device_id,
            auth_seed,
            certificate_chain,
            actor_user,
            &actor_puk.seed,
            team,
            &binding,
            &encoded_request,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_addition_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk_seed: &SecretSeed,
        team: &EntityId,
        binding: &AdditionBinding<'_>,
        encoded_request: &[u8],
        federation_saga_id: Option<&[u8; 16]>,
        protected_store: Option<&mut dyn ProtectedMutationStore>,
    ) -> Result<AddedLocalTeamMember> {
        let operation_id = addition_operation_id(uid, team, binding)?;
        let created_at = now_microseconds()?;
        let protected_request = if let Some(store) = protected_store {
            let key = remote_addition_material_key(&operation_id);
            match store.put_if_absent(&key, encoded_request) {
                Ok(()) | Err(ProtectedStoreError::Conflict) => {}
                Err(error) => return Err(protected_material_error(error)),
            }
            Some(store.get(&key).map_err(protected_material_error)?)
        } else {
            None
        };
        let submitted_request = protected_request
            .as_ref()
            .map_or(encoded_request, |request| request.as_slice());
        let operation = TeamMutationOperation {
            operation_id,
            kind: TeamMutationKind::MembershipChange,
            host_id: host.host_id().as_bytes().to_vec(),
            actor_id: uid.as_bytes().to_vec(),
            device_id: device_id.as_bytes().to_vec(),
            team_id: team.as_bytes().to_vec(),
            expected_seqno: binding.expected_seqno,
            request_hash: prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, submitted_request),
            state: TeamMutationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        if let Some(saga_id) = federation_saga_id {
            hard_store.prepare_federation_local_mutation(saga_id, &operation, created_at)?;
        } else {
            hard_store.record_and_begin_team_mutation(&operation, now_microseconds()?)?;
        }
        if federation_saga_id.is_some() {
            hard_store.advance_team_mutation(
                &operation_id,
                TeamMutationState::Submitting,
                now_microseconds()?,
            )?;
        }
        let post = || {
            let response = self.call_with_material(
                host,
                &host.user,
                submitted_request,
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
        // Definite server rejection (stale Merkle root or seqno) is never
        // committed — mark Rejected immediately so the seqno reservation
        // is released and a fresh root can be retried without wedge.
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
        let authenticated = match self.wait_for_addition(
            host,
            uid,
            auth_seed,
            certificate_chain,
            actor_user,
            actor_puk_seed,
            team,
            binding,
        ) {
            Ok(value) => value,
            Err(_) if post_error.is_some() => {
                // If the server definitely rejected and reconciliation shows
                // no conflicting transition, release the reservation. For
                // ambiguous SubmissionUnknown, the Rejected transition is
                // now allowed (lib.rs) so a sole client can be unblocked
                // without hard-state reset after confirming no commit.
                if is_definite_rejection {
                    // Already Rejected above; just surface the error.
                    return Err(post_error.expect("checked above"));
                }
                // For ambiguous errors where wait failed, attempt to mark
                // Rejected if no conflicting head was observed. The caller
                // retains the original error for diagnostics.
                let _ = hard_store.advance_team_mutation(
                    &operation_id,
                    TeamMutationState::Rejected,
                    now_microseconds()?,
                );
                return Err(post_error.expect("checked above"));
            }
            Err(error) => return Err(error),
        };
        finish_team_mutation_journal(&mut hard_store, &operation_id)?;
        Ok(AddedLocalTeamMember {
            operation_id,
            expected_seqno: binding.expected_seqno,
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
            target_host: None,
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

    #[allow(clippy::too_many_arguments)]
    fn authenticated_addition_conflicts(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor_user: &AuthenticatedUserOutcome,
        actor_puk_seed: &SecretSeed,
        team: &EntityId,
        binding: &AdditionBinding<'_>,
    ) -> Result<bool> {
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
            return Ok(false);
        }
        match validate_addition_transition(&authenticated.verified, binding) {
            Ok(()) => Ok(false),
            Err(Error::OperationBinding(_)) => Ok(true),
            Err(error) => Err(error),
        }
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

fn box_visible_ptks_for_remote(
    host: &PinnedHost,
    actor_puk: &UserPrivateKey,
    target_id: &EntityId,
    target_host: &EntityId,
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
            receiver_host: Some(target_host),
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
        binding
            .target_host
            .map_or(Value::Null, |host| Value::Binary(host.as_bytes().to_vec())),
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

pub(crate) fn remote_addition_material_key(operation_id: &[u8; 16]) -> Vec<u8> {
    let mut key = b"federation-remote-team-addition-v1:".to_vec();
    key.extend_from_slice(operation_id);
    key
}

fn protected_material_error(error: ProtectedStoreError) -> Error {
    Error::ProtectedMaterial(error.to_string())
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
        || member.scoped_host.as_ref() != binding.target_host
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
    match operation.state {
        TeamMutationState::Verified => {}
        TeamMutationState::Submitting
        | TeamMutationState::SubmissionUnknown
        | TeamMutationState::Submitted => store.advance_team_mutation(
            operation_id,
            TeamMutationState::Verified,
            now_microseconds()?,
        )?,
        TeamMutationState::Prepared => {
            return Err(Error::OperationBinding(
                "prepared team mutation was never submitted",
            ));
        }
        TeamMutationState::Rejected | TeamMutationState::Superseded => {
            return Err(Error::OperationBinding(
                "terminal team mutation cannot be verified",
            ));
        }
    }
    Ok(())
}
