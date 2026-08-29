//! Cross-host user federation workflows built on authenticated host pins.

use foks_client_db::{
    Acceptance, FederationSagaOperation, FederationSagaState, HardStateStore, TeamMutationState,
};
use foks_crypto::{
    federation_permission_token_hash, prefixed_hash, sign_shared_key_typed,
    team_removal_key_commitment,
};
use foks_proto::{
    EntityId, FqParty, PermissionToken, RemoteViewPermissionPayload, Role, SecretSeed, TeamChain,
    ENTITY_PTK_VERIFY, ENTITY_USER, REMOTE_VIEW_PERMISSION_PAYLOAD_TYPE_ID,
};
use foks_rpc::{
    encode_grant_remote_view_permission_for_team_request,
    encode_grant_remote_view_permission_for_user_request, encode_load_remote_team_chain_request,
    encode_load_remote_user_chain_request, encode_registration_select_vhost_request,
};
use foks_snowpack::{encode, Value};
use foks_verify::{
    team_chain_root_epochs, verify_team_chain, verify_team_chain_increment, verify_user_chain,
    verify_user_chain_increment, VerifiedTeamState, VerifiedUserState,
};

use crate::auth::user_chain_cursor;
use crate::{
    current_owner_puk, now_microseconds, AddRemoteTeamMemberRequest, AddedRemoteTeamMember,
    DeviceCredential, Error, FoksClient, PinnedHost, ProtectedMutationStore, ProtectedStoreError,
    Result,
};

const FEDERATION_SAGA_OPERATION_ID_TYPE_ID: u64 = 0x14db_10fd_97c7_08ac;

#[derive(Debug)]
pub struct RemoteUserOutcome {
    pub merkle_acceptance: Acceptance,
    pub acceptance: Acceptance,
    pub verified: VerifiedUserState,
}

#[derive(Debug)]
pub struct RemoteTeamOutcome {
    pub merkle_acceptance: Acceptance,
    pub acceptance: Acceptance,
    pub verified: VerifiedTeamState,
    pub(crate) permission: PermissionToken,
}

/// Both authenticated sides of one remote-team admission. The coordinator
/// journals only public bindings; credentials, the permission, and the
/// removal key stay in caller-owned protected memory.
pub struct FederatedTeamAdmissionRequest<'a> {
    pub remote_host: &'a PinnedHost,
    pub remote_credential: &'a DeviceCredential,
    pub remote_team: &'a EntityId,
    pub local_host: &'a PinnedHost,
    pub local_credential: &'a DeviceCredential,
    pub local_team: &'a EntityId,
    pub destination_role: Role,
    pub removal_key: &'a SecretSeed,
}

pub struct FederatedTeamAdmissionOutcome {
    pub operation_id: [u8; 16],
    pub remote: RemoteTeamOutcome,
    pub added: AddedRemoteTeamMember,
}

/// Durable identities needed to refresh an already-admitted remote team's
/// view capability without re-entering the admission saga.
pub struct FederatedTeamRefreshRequest<'a> {
    pub remote_host: &'a PinnedHost,
    pub remote_credential: &'a DeviceCredential,
    pub remote_team: &'a EntityId,
    pub local_host: &'a PinnedHost,
    pub local_credential: &'a DeviceCredential,
    pub local_team: &'a EntityId,
}

impl FoksClient {
    /// Renews the existing remote-view bearer and proves that the local team
    /// still stores that same capability. This never creates or resumes a
    /// federation admission saga and never edits the local team chain.
    pub fn refresh_federated_team_capability(
        &self,
        request: &FederatedTeamRefreshRequest<'_>,
    ) -> Result<RemoteTeamOutcome> {
        if request.remote_host.host_id() == request.local_host.host_id()
            || request.remote_team == request.local_team
        {
            return Err(Error::TeamRequest(
                "federation refresh hosts or parties are invalid",
            ));
        }
        request
            .local_team
            .clone()
            .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
        if !matches!(
            request.remote_team.entity_type(),
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
        ) {
            return Err(Error::TeamRequest("remote federation party is not a team"));
        }
        let viewer = FqParty::new(
            request.local_team.clone(),
            request.local_host.host_id().clone(),
        )?;
        let permission = self.grant_remote_team_view(
            request.remote_host,
            request.remote_credential,
            request.remote_team,
            viewer,
        )?;
        let remote =
            self.load_remote_team_and_pin(request.remote_host, request.remote_team, &permission)?;
        let member = FqParty::new(
            request.remote_team.clone(),
            request.remote_host.host_id().clone(),
        )?;
        let recovered = self.load_remote_member_view_permissions(
            request.local_host,
            request.local_credential,
            request.local_team,
            std::slice::from_ref(&member),
        )?;
        let [recovered] = recovered.as_slice() else {
            return Err(Error::OperationBinding(
                "local team did not return the admitted remote permission",
            ));
        };
        if recovered.member != member || recovered.permission != permission {
            return Err(Error::OperationBinding(
                "refreshed permission differs from the admitted capability",
            ));
        }
        Ok(remote)
    }

    /// Runs or resumes the durable cross-host admission saga. A retry first
    /// reobtains the idempotent live permission, then reconciles the local
    /// mutation journal without blindly replaying a possibly committed edit.
    pub fn admit_remote_team_to_named_team(
        &self,
        request: &FederatedTeamAdmissionRequest<'_>,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<FederatedTeamAdmissionOutcome> {
        validate_admission_request(request)?;
        let viewer = FqParty::new(
            request.local_team.clone(),
            request.local_host.host_id().clone(),
        )?;
        let permission = self.grant_remote_team_view(
            request.remote_host,
            request.remote_credential,
            request.remote_team,
            viewer,
        )?;
        let removal_commitment = team_removal_key_commitment(request.removal_key)?;
        let operation_id = federation_saga_operation_id(request, &removal_commitment)?;
        let now = now_microseconds()?;
        let (destination_role_type, destination_visibility) = role_parts(request.destination_role);
        let mut store = HardStateStore::open(&request.local_host.database_path)?;
        store.record_federation_saga(&FederationSagaOperation {
            operation_id,
            local_host_id: request.local_host.host_id().as_bytes().to_vec(),
            remote_host_id: request.remote_host.host_id().as_bytes().to_vec(),
            actor_id: request.local_credential.uid.as_bytes().to_vec(),
            local_team_id: request.local_team.as_bytes().to_vec(),
            remote_party_id: request.remote_team.as_bytes().to_vec(),
            permission_hash: federation_permission_token_hash(&permission),
            destination_role_type,
            destination_visibility,
            removal_key_commitment: removal_commitment,
            state: FederationSagaState::PermissionGranted,
            expected_local_seqno: None,
            local_mutation_id: None,
            created_at: now,
            updated_at: now,
        })?;
        let mut saga = store
            .federation_saga(&operation_id)?
            .ok_or(Error::OperationBinding("federation saga disappeared"))?;
        if saga.state == FederationSagaState::Rejected {
            return Err(Error::OperationBinding(
                "federation saga was definitively rejected",
            ));
        }

        let remote =
            self.load_remote_team_and_pin(request.remote_host, request.remote_team, &permission)?;
        if saga.state == FederationSagaState::PermissionGranted {
            store.advance_federation_saga(
                &operation_id,
                FederationSagaState::RemoteVerified,
                now_microseconds()?,
            )?;
            saga = store
                .federation_saga(&operation_id)?
                .ok_or(Error::OperationBinding("federation saga disappeared"))?;
        }
        let addition = AddRemoteTeamMemberRequest {
            remote_team: &remote,
            destination_role: request.destination_role,
            removal_key: request.removal_key,
        };
        let added = match saga.state {
            FederationSagaState::RemoteVerified => self.add_remote_team_to_named_team_for_saga(
                request.local_host,
                request.local_credential,
                request.local_team,
                &addition,
                Some(&operation_id),
                Some(protected_store),
            )?,
            FederationSagaState::LocalPrepared
            | FederationSagaState::LocalVerified
            | FederationSagaState::Completed => {
                let sequence = saga.expected_local_seqno.ok_or(Error::OperationBinding(
                    "federation saga has no local sequence checkpoint",
                ))?;
                let mutation_id = saga.local_mutation_id.ok_or(Error::OperationBinding(
                    "federation saga has no local mutation checkpoint",
                ))?;
                match store.team_mutation(&mutation_id)? {
                    Some(local)
                        if matches!(
                            local.state,
                            TeamMutationState::Rejected | TeamMutationState::Superseded
                        ) =>
                    {
                        store.advance_federation_saga(
                            &operation_id,
                            FederationSagaState::Rejected,
                            now_microseconds()?,
                        )?;
                        return Err(Error::OperationBinding(
                            "local team mutation was rejected or superseded",
                        ));
                    }
                    Some(_) => self.resume_add_remote_team_to_named_team_for_saga(
                        request.local_host,
                        request.local_credential,
                        request.local_team,
                        sequence,
                        &addition,
                        protected_store,
                    )?,
                    None if saga.state == FederationSagaState::LocalPrepared => self
                        .add_remote_team_to_named_team_for_saga(
                            request.local_host,
                            request.local_credential,
                            request.local_team,
                            &addition,
                            Some(&operation_id),
                            Some(protected_store),
                        )?,
                    None => {
                        return Err(Error::OperationBinding(
                            "verified federation saga lost its local mutation journal",
                        ));
                    }
                }
            }
            FederationSagaState::PermissionGranted | FederationSagaState::Rejected => {
                return Err(Error::OperationBinding(
                    "federation saga state changed unexpectedly",
                ));
            }
        };
        saga = store
            .federation_saga(&operation_id)?
            .ok_or(Error::OperationBinding("federation saga disappeared"))?;
        if saga.state == FederationSagaState::LocalPrepared {
            store.advance_federation_saga(
                &operation_id,
                FederationSagaState::LocalVerified,
                now_microseconds()?,
            )?;
        }

        let member = FqParty::new(
            request.remote_team.clone(),
            request.remote_host.host_id().clone(),
        )?;
        let recovered = self.load_remote_member_view_permissions(
            request.local_host,
            request.local_credential,
            request.local_team,
            std::slice::from_ref(&member),
        )?;
        let [recovered] = recovered.as_slice() else {
            return Err(Error::OperationBinding(
                "local team did not return the admitted remote permission",
            ));
        };
        if recovered.member != member || recovered.permission != permission {
            return Err(Error::OperationBinding(
                "recovered remote permission does not match the granted capability",
            ));
        }
        store.advance_federation_saga(
            &operation_id,
            FederationSagaState::Completed,
            now_microseconds()?,
        )?;
        match protected_store.remove(&crate::team::remote_addition_material_key(
            &added.operation_id,
        )) {
            Ok(()) | Err(ProtectedStoreError::Missing) => {}
            Err(error) => return Err(Error::ProtectedMaterial(error.to_string())),
        }
        Ok(FederatedTeamAdmissionOutcome {
            operation_id,
            remote,
            added,
        })
    }

    /// Grants a remote fully-qualified party a bearer token for this user's
    /// public chain. Repeating the same grant returns the same live token.
    pub fn grant_remote_user_view(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        viewer: FqParty,
    ) -> Result<PermissionToken> {
        credential.uid.clone().require_type(ENTITY_USER)?;
        let payload =
            RemoteViewPermissionPayload::new(credential.uid.clone(), viewer, now_microseconds()?)?;
        let request = encode_grant_remote_view_permission_for_user_request(&payload)?;
        let response = self.call(host, &host.user, &request, Some(credential))?;
        PermissionToken::decode(&response).map_err(Into::into)
    }

    /// Loads and verifies a remote user's chain through the public
    /// registration service using an explicitly granted bearer token.
    /// `host` must be the viewee host; a chain bound to any other host is
    /// rejected before it is pinned.
    pub fn load_remote_user_and_pin(
        &self,
        host: &PinnedHost,
        uid: &foks_proto::EntityId,
        token: &PermissionToken,
    ) -> Result<RemoteUserOutcome> {
        uid.clone().require_type(ENTITY_USER)?;
        let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
        let prior = match self.pinned_user(host, uid) {
            Ok(prior) => prior,
            Err(Error::Verify(
                foks_verify::Error::PersistedMerkleEvidence
                | foks_verify::Error::UserChainContinuity,
            )) => None,
            Err(error) => return Err(error),
        };
        let (start, name) = user_chain_cursor(prior.as_ref())?;
        let request = encode_load_remote_user_chain_request(uid, start, name, token)?;
        let chain_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &request,
        )?;
        let authenticated_roots =
            self.authenticate_user_chain_roots(host, &merkle, &chain_bytes)?;
        let verified = match prior.as_ref() {
            Some(prior) => verify_user_chain_increment(
                &chain_bytes,
                prior,
                uid,
                host.host_id(),
                &authenticated_roots,
                &merkle.root().hostchain,
            )?,
            None => verify_user_chain(
                &chain_bytes,
                uid,
                host.host_id(),
                &authenticated_roots,
                &merkle.root().hostchain,
            )?,
        };
        if verified.host() != host.host_id() {
            return Err(Error::UserBinding(
                "remote user chain host does not match the pinned host",
            ));
        }
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_user(&verified.hard_state_snapshot()?)?;
        Ok(RemoteUserOutcome {
            merkle_acceptance,
            acceptance,
            verified,
        })
    }

    /// Grants a remote party access to one locally authoritative team's
    /// public chain. The grant is signed by the current admin PTK and issued
    /// over the authenticated user service.
    pub fn grant_remote_team_view(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &foks_proto::EntityId,
        viewer: FqParty,
    ) -> Result<PermissionToken> {
        let user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&user)?;
        let loaded = self.load_and_pin_team(
            host,
            credential,
            &user.verified,
            std::slice::from_ref(owner),
            team,
        )?;
        let public = loaded
            .verified
            .shared_key(Role::ADMIN)
            .ok_or(Error::KeyBinding("team has no current admin PTK"))?;
        let private = loaded
            .ptks
            .iter()
            .find(|candidate| {
                candidate.role == public.role && candidate.generation == public.generation
            })
            .ok_or(Error::KeyBinding("current admin PTK is unavailable"))?;
        if foks_crypto::derive_shared_public(&private.seed, ENTITY_PTK_VERIFY)?.verify_key
            != public.verify_key
        {
            return Err(Error::KeyBinding(
                "current admin PTK does not match team state",
            ));
        }
        let payload = RemoteViewPermissionPayload::new(team.clone(), viewer, now_microseconds()?)?;
        let signature = sign_shared_key_typed(
            &private.seed,
            REMOTE_VIEW_PERMISSION_PAYLOAD_TYPE_ID,
            &payload.encoded()?,
        )?;
        let request = encode_grant_remote_view_permission_for_team_request(
            &payload,
            &signature,
            private.generation,
            private.role,
        )?;
        PermissionToken::decode(&self.call(host, &host.user, &request, Some(credential))?)
            .map_err(Into::into)
    }

    /// Loads a remote team through a permission token, verifies its Merkle
    /// evidence, and persists only authenticated public hard state. Remote
    /// loads never accept PTK parcels. `host` must be the viewee host.
    pub fn load_remote_team_and_pin(
        &self,
        host: &PinnedHost,
        team: &foks_proto::EntityId,
        token: &PermissionToken,
    ) -> Result<RemoteTeamOutcome> {
        if !matches!(
            team.entity_type(),
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
        ) {
            return Err(Error::TeamBinding("remote party is not a team"));
        }
        let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
        let prior = match self.pinned_team(host, team) {
            Ok(prior) => prior,
            Err(Error::Verify(
                foks_verify::Error::PersistedMerkleEvidence
                | foks_verify::Error::TeamChainContinuity,
            )) => None,
            Err(error) => return Err(error),
        };
        let (start, name) = match prior.as_ref() {
            Some(prior) => (
                prior
                    .chain_seqno()
                    .checked_add(1)
                    .ok_or(Error::TeamBinding("team chain sequence overflow"))?,
                Some((
                    prior.team_name(),
                    prior
                        .team_name_sequence()
                        .checked_add(1)
                        .ok_or(Error::TeamBinding("team name sequence overflow"))?,
                )),
            ),
            None => (1, None),
        };
        let chain_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_load_remote_team_chain_request(team, host.host_id(), token, start, name)?,
        )?;
        let disclosed = TeamChain::decode(&chain_bytes)?;
        if !disclosed.boxes.is_empty() {
            return Err(Error::TeamBinding(
                "remote team response unexpectedly disclosed PTK parcels",
            ));
        }
        let targets = team_chain_root_epochs(&chain_bytes)?
            .into_iter()
            .filter(|epoch| !merkle.authenticated_roots().contains_epoch(*epoch))
            .collect();
        let authenticated_roots =
            self.authenticate_chain_roots(host, &merkle, targets, Error::TeamBinding)?;
        let verified = match prior.as_ref() {
            Some(prior) => verify_team_chain_increment(
                &chain_bytes,
                prior,
                team,
                host.host_id(),
                &authenticated_roots,
                &merkle.root().hostchain,
            )?,
            None => verify_team_chain(
                &chain_bytes,
                team,
                host.host_id(),
                &authenticated_roots,
                &merkle.root().hostchain,
            )?,
        };
        if verified.host() != host.host_id() {
            return Err(Error::TeamBinding(
                "remote team chain host does not match the pinned host",
            ));
        }
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_team(&verified.hard_state_snapshot()?)?;
        Ok(RemoteTeamOutcome {
            merkle_acceptance,
            acceptance,
            verified,
            permission: token.clone(),
        })
    }
}

fn validate_admission_request(request: &FederatedTeamAdmissionRequest<'_>) -> Result<()> {
    if request.remote_host.host_id() == request.local_host.host_id()
        || request.destination_role == Role::NONE
        || request.remote_team == request.local_team
    {
        return Err(Error::TeamRequest(
            "federation admission hosts, parties, or role are invalid",
        ));
    }
    request
        .local_team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    if !matches!(
        request.remote_team.entity_type(),
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
    ) {
        return Err(Error::TeamRequest("remote federation party is not a team"));
    }
    Ok(())
}

fn federation_saga_operation_id(
    request: &FederatedTeamAdmissionRequest<'_>,
    removal_commitment: &[u8; 32],
) -> Result<[u8; 16]> {
    let binding = encode(&Value::Array(vec![
        Value::Binary(request.local_host.host_id().as_bytes().to_vec()),
        Value::Binary(request.remote_host.host_id().as_bytes().to_vec()),
        Value::Binary(request.local_credential.uid.as_bytes().to_vec()),
        Value::Binary(request.local_team.as_bytes().to_vec()),
        Value::Binary(request.remote_team.as_bytes().to_vec()),
        request.destination_role.to_value(),
        Value::Binary(removal_commitment.to_vec()),
    ]))?;
    let hash = prefixed_hash(FEDERATION_SAGA_OPERATION_ID_TYPE_ID, &binding);
    hash[..16]
        .try_into()
        .map_err(|_| Error::OperationBinding("federation saga ID has the wrong length"))
}

fn role_parts(role: Role) -> (u64, i64) {
    (
        role.protocol_value(),
        i64::from(role.visibility().unwrap_or(0)),
    )
}
