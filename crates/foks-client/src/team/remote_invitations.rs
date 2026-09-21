//! Cross-host invitation preparation and authenticated request expansion.
use crate::*;
use foks_proto::{
    FqParty, PostGenericLinkArgument, RemoteJoinPayload, RemoteJoinRequest, RemoteJoinVisible,
    TeamInvite, TeamRsvp,
};

#[derive(Clone)]
pub struct PreparedRemoteInvitation {
    pub(crate) team: EntityId,
    pub(crate) invite: TeamInvite,
    pub(crate) request: RemoteJoinRequest,
    pub(crate) home_link: PostGenericLinkArgument,
}
pub struct ExpandedRemoteInvitation {
    pub payload: RemoteJoinPayload,
    pub user: crate::RemoteUserOutcome,
}
impl FoksClient {
    pub fn prepare_remote_user_invitation(
        &self,
        home: &PinnedHost,
        destination: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        invite: &TeamInvite,
    ) -> Result<PreparedRemoteInvitation> {
        if home.host_id() == destination.host_id() {
            return Err(Error::TeamBinding(
                "remote invitation requires different hosts",
            ));
        }
        let preview = self.preview_team_invitation(destination, invite)?;
        let user = self.authenticate_credential_and_pin(home, credential)?;
        let (seed, certs) = credential.transport();
        let viewer = FqParty::new(
            preview.advertised.team.team.clone(),
            destination.host_id().clone(),
        )?;
        let grant = foks_proto::RemoteViewPermissionPayload::new(
            credential.uid().clone(),
            viewer,
            now_milliseconds()?,
        )?;
        let response = self.call_with_material(
            home,
            &home.user,
            &foks_rpc::encode_grant_remote_view_permission_for_user_request(&grant)?,
            seed,
            certs,
        )?;
        let permission = foks_proto::PermissionToken::decode(&response)?;
        let payload = RemoteJoinPayload {
            joiner: FqParty::new(credential.uid().clone(), home.host_id().clone())?,
            permission,
            time: now_milliseconds()?,
            source_role: Role::OWNER,
            visible: RemoteJoinVisible { index_range: None },
        };
        let owner = current_owner_puk(&user)?;
        let request = foks_crypto::seal_remote_join_request(
            &owner.seed,
            &preview.advertised,
            &payload,
            &super::membership::random_box_randomness()?,
        )?;
        let home_link = self.prepare_requested_user_membership(
            home,
            credential,
            &user.verified,
            &preview.advertised.team,
        )?;
        Ok(PreparedRemoteInvitation {
            team: preview.advertised.team.team,
            invite: invite.clone(),
            request,
            home_link,
        })
    }
    /// Public guest send. Caller must persist the exact request before this call.
    pub fn submit_remote_invitation(
        &self,
        destination: &PinnedHost,
        prepared: &PreparedRemoteInvitation,
    ) -> Result<TeamRsvp> {
        if destination.host_id() != &prepared.invite.host {
            return Err(Error::TeamBinding("remote invitation destination"));
        }
        let reply = self.call(
            destination,
            &destination.registration,
            &foks_rpc::encode_accept_invite_remote_request(&prepared.invite, &prepared.request)?,
            None,
        )?;
        let receipt = TeamRsvp::decode(&reply)?;
        if !receipt.is_remote() {
            return Err(Error::TeamBinding("remote RSVP kind"));
        }
        Ok(receipt)
    }
    pub fn load_remote_invitation_request(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        receipt: &TeamRsvp,
    ) -> Result<RemoteJoinRequest> {
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let loaded = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            team,
        )?;
        let (seed, certs) = credential.transport();
        let token =
            self.activate_team_admin_bearer(host, credential.uid(), seed, certs, &loaded)?;
        let bytes = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_load_remote_join_request(&token, receipt)?,
            seed,
            certs,
        )?;
        Ok(RemoteJoinRequest::decode(&bytes)?)
    }
    pub fn open_remote_invitation(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        request: &RemoteJoinRequest,
    ) -> Result<RemoteJoinPayload> {
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let loaded = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            team,
        )?;
        self.open_remote_invitation_with_team(host, team, &loaded, request)
    }

    /// [`Self::open_remote_invitation`] against a destination team the caller
    /// has already authenticated with the same credential, so an approval
    /// opens its request against one destination load rather than its own.
    /// The opening itself is local: only the destination's retained admin
    /// PTKs can unseal the request.
    pub fn open_remote_invitation_with_team(
        &self,
        host: &PinnedHost,
        team: &EntityId,
        loaded: &crate::AuthenticatedTeamOutcome,
        request: &RemoteJoinRequest,
    ) -> Result<RemoteJoinPayload> {
        super::invitations::require_invitation_destination(host, team, loaded)?;
        super::verified_team_private_history(loaded)?;
        for key in loaded.ptks.iter().filter(|k| k.role == Role::ADMIN) {
            let receiver = foks_crypto::SharedKeyDecapsulator::new(&key.seed, team.clone())?;
            if foks_crypto::hepk_fingerprint(foks_crypto::HybridSecretDecapsulator::hepk(
                &receiver,
            ))? == request.hepk_fingerprint
            {
                return Ok(foks_crypto::open_remote_join_request(request, &receiver)?);
            }
        }
        Err(Error::KeyBinding(
            "historical invitation admin key unavailable",
        ))
    }
    /// Source is already pinned by the caller; decrypted host strings are never dialled.
    pub fn verify_remote_invitation_user(
        &self,
        source: &PinnedHost,
        payload: RemoteJoinPayload,
    ) -> Result<ExpandedRemoteInvitation> {
        if &payload.joiner.host != source.host_id()
            || payload.joiner.party.entity_type() != foks_proto::ENTITY_USER
            || payload.source_role != Role::OWNER
            || payload.visible.index_range.is_some()
        {
            return Err(Error::TeamBinding("remote invitation user binding"));
        }
        let user =
            self.load_remote_user_and_pin(source, &payload.joiner.party, &payload.permission)?;
        Ok(ExpandedRemoteInvitation { payload, user })
    }
}

impl FoksClient {
    /// Resumes the invitation saga without holding a connection across user approval.
    /// Phase 1 is an uncertain home link, 2 verified home intent, 3 uncertain guest send.
    pub fn remote_invitation_progress(
        &self,
        home: &PinnedHost,
        destination: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        id: [u8; 16],
        attempt: bool,
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<super::InvitationProgress> {
        let mut op = self.invitation_operation(home, credential.uid(), id)?;
        if op.state.is_terminal() {
            crate::mutation::remove_terminal_material(protected, &op.material_ref)?;
            return Ok(super::InvitationProgress {
                operation: op,
                receipt: None,
                invite: None,
            });
        }
        let intent = super::InvitationIntent::decode(
            &MutationCoordinator::new(&home.database_path, protected).load_bound_material(&op)?,
        )?;
        let super::InvitationIntent::RemoteAcceptance(p) = intent else {
            return Err(Error::OperationBinding(
                "operation is not a remote acceptance",
            ));
        };
        if destination.host_id() != &p.invite.host {
            return Err(Error::OperationBinding("remote acceptance destination"));
        }
        let ack_key = crate::ProtectedRecordKey::InvitationAck(&id).encoded();
        match protected.get(&ack_key) {
            Ok(ack) => {
                let receipt = TeamRsvp::decode(&ack)?;
                MutationCoordinator::new(&home.database_path, protected).remote_verified(&id)?;
                return Ok(super::InvitationProgress {
                    operation: self.invitation_operation(home, credential.uid(), id)?,
                    receipt: Some(receipt),
                    invite: None,
                });
            }
            Err(ProtectedStoreError::Missing) => {}
            Err(e) => return Err(Error::ProtectedMaterial(e.to_string())),
        }
        let mut phase = HardStateStore::open(&home.database_path)?
            .invitation_delivery_phase(&id)?
            .unwrap_or(0);
        if op.state == MutationState::Prepared && attempt {
            self.authenticate_credential_and_pin(home, credential)?;
            // Persist phase before Submitting: either crash ordering remains conservative.
            if phase == 0 {
                HardStateStore::open(&home.database_path)?
                    .advance_invitation_delivery(&id, 0, 1)?;
            } else if phase != 1 {
                return Err(Error::OperationBinding("unsubmitted invitation phase"));
            }
            MutationCoordinator::new(&home.database_path, protected).begin_submission(&id)?;
            phase = 1;
            let (seed, certs) = credential.transport();
            let entity = p.home_link.link.decode_generic()?.entity;
            let request = if entity == *credential.uid() {
                foks_rpc::encode_post_generic_link_request(&p.home_link)?
            } else {
                let user = self.authenticate_credential_and_pin(home, credential)?;
                let source = self.load_and_pin_team_with_credential(
                    home,
                    credential,
                    &user.verified,
                    &user.puks,
                    &entity,
                )?;
                let token =
                    self.activate_team_admin_bearer(home, credential.uid(), seed, certs, &source)?;
                foks_rpc::encode_post_team_membership_link_request(&token, &p.home_link)?
            };
            let send = self.call_void_with_material(home, &home.user, &request, seed, certs);
            // A verified exact home link resolves its lost reply. It never establishes
            // guest delivery and cannot cause an automatic second home submission.
            if let Err(e) = send {
                MutationCoordinator::new(&home.database_path, protected).submission_unknown(&id)?;
                if !self.invitation_home_link_present(home, credential, &p.home_link)? {
                    return Err(e);
                }
            }
        }
        if op.state == MutationState::Prepared && !attempt {
            return Ok(super::InvitationProgress {
                operation: op,
                receipt: None,
                invite: None,
            });
        }
        if phase == 3 && op.state == MutationState::Submitting {
            MutationCoordinator::new(&home.database_path, protected).submission_unknown(&id)?;
        }
        if phase == 1 && self.invitation_home_link_present(home, credential, &p.home_link)? {
            HardStateStore::open(&home.database_path)?.advance_invitation_delivery(&id, 1, 2)?;
            phase = 2;
        }
        let mut receipt = None;
        if phase == 2 && attempt {
            HardStateStore::open(&home.database_path)?.advance_invitation_delivery(&id, 2, 3)?;
            match self.submit_remote_invitation(destination, &p) {
                Ok(r) => {
                    protected
                        .put_if_absent(&ack_key, &zeroize::Zeroizing::new(r.encoded()?))
                        .map_err(|e| Error::ProtectedMaterial(e.to_string()))?;
                    MutationCoordinator::new(&home.database_path, protected)
                        .remote_verified(&id)?;
                    receipt = Some(r);
                }
                Err(e) => {
                    MutationCoordinator::new(&home.database_path, protected)
                        .submission_unknown(&id)?;
                    return Err(e);
                }
            }
        }
        op = self.invitation_operation(home, credential.uid(), id)?;
        if receipt.is_none() && op.state == MutationState::Submitting {
            MutationCoordinator::new(&home.database_path, protected).submission_unknown(&id)?;
            op = self.invitation_operation(home, credential.uid(), id)?;
        }
        Ok(super::InvitationProgress {
            operation: op,
            receipt,
            invite: None,
        })
    }
    fn invitation_home_link_present(
        &self,
        home: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        link: &PostGenericLinkArgument,
    ) -> Result<bool> {
        // The fleet may publish a root between owner and subchain reads.
        // Retry both read-only proofs together; never retry the home send.
        self.retry_chain_load(home, |home| {
            self.invitation_home_link_at_root(home, credential, link)
        })
    }
    fn invitation_home_link_at_root(
        &self,
        home: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        link: &PostGenericLinkArgument,
    ) -> Result<bool> {
        let user = self.authenticate_credential_and_pin(home, credential)?;
        let entity = link.link.decode_generic()?.entity;
        if entity != *credential.uid() {
            let source = self.load_and_pin_team_with_credential(
                home,
                credential,
                &user.verified,
                &user.puks,
                &entity,
            )?;
            let verified = self.invitation_team_membership_chain(home, credential, &source)?;
            let hash =
                foks_crypto::prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &link.link.encoded()?);
            return Ok(verified.link_hashes.contains(&hash));
        }
        let (seed, certs) = credential.transport();
        let verified = self.authenticated_membership_chain(
            home,
            credential.uid(),
            seed,
            certs,
            &user.verified,
        )?;
        let hash =
            foks_crypto::prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &link.link.encoded()?);
        Ok(verified.link_hashes.contains(&hash))
    }
}
