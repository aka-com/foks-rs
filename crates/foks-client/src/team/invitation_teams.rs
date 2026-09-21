//! Joining teams retain their own source role and membership-chain signer.
use crate::*;
use foks_proto::{
    FqTeam, PostGenericLinkArgument, RemoteJoinPayload, RemoteJoinVisible, TeamInvite,
};

impl FoksClient {
    pub fn prepare_local_team_invitation(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        source: &EntityId,
        source_role: Role,
        invite: &TeamInvite,
    ) -> Result<foks_proto::LocalInviteAcceptance> {
        let preview = self.preview_team_invitation(host, invite)?;
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let loaded = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            source,
        )?;
        Self::check_invitation_source(&loaded, source_role, &preview)?;
        let link = self.prepare_requested_team_membership(
            host,
            credential,
            &loaded,
            source_role,
            &preview.advertised.team,
        )?;
        let (seed, certs) = credential.transport();
        let token =
            self.activate_team_admin_bearer(host, credential.uid(), seed, certs, &loaded)?;
        Ok(foks_proto::LocalInviteAcceptance {
            invite: invite.clone(),
            source_role,
            source_token: Some(token),
            membership_link: Some(link),
        })
    }
    pub fn prepare_remote_team_invitation(
        &self,
        home: &PinnedHost,
        destination: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        source: &EntityId,
        source_role: Role,
        invite: &TeamInvite,
    ) -> Result<super::PreparedRemoteInvitation> {
        if home.host_id() == destination.host_id() {
            return Err(Error::TeamBinding(
                "remote invitation requires different hosts",
            ));
        }
        let preview = self.preview_team_invitation(destination, invite)?;
        let user = self.authenticate_credential_and_pin(home, credential)?;
        let loaded = self.load_and_pin_team_with_credential(
            home,
            credential,
            &user.verified,
            &user.puks,
            source,
        )?;
        Self::check_invitation_source(&loaded, source_role, &preview)?;
        let viewer = foks_proto::FqParty::new(
            preview.advertised.team.team.clone(),
            destination.host_id().clone(),
        )?;
        let permission =
            self.grant_remote_team_view_with_credential(home, credential, source, viewer)?;
        let payload = RemoteJoinPayload {
            joiner: foks_proto::FqParty::new(source.clone(), home.host_id().clone())?,
            permission,
            time: now_milliseconds()?,
            source_role,
            visible: RemoteJoinVisible {
                index_range: Some(loaded.verified.index_range().clone()),
            },
        };
        let key = loaded
            .ptks
            .iter()
            .filter(|k| k.role == source_role)
            .max_by_key(|k| k.generation)
            .ok_or(Error::TeamBinding(
                "invitation source private key unavailable",
            ))?;
        let request = foks_crypto::seal_remote_join_request(
            &key.seed,
            &preview.advertised,
            &payload,
            &super::membership::random_box_randomness()?,
        )?;
        let home_link = self.prepare_requested_team_membership(
            home,
            credential,
            &loaded,
            source_role,
            &preview.advertised.team,
        )?;
        Ok(super::PreparedRemoteInvitation {
            team: preview.advertised.team.team,
            invite: invite.clone(),
            request,
            home_link,
        })
    }
    fn check_invitation_source(
        loaded: &AuthenticatedTeamOutcome,
        role: Role,
        preview: &super::InvitationPreview,
    ) -> Result<()> {
        if role == Role::NONE
            || loaded.verified.shared_key(role).is_none()
            || (loaded.verified.team() == &preview.advertised.team.team
                && loaded.verified.host() == &preview.advertised.team.host)
            || !foks_verify::rational_range_strictly_before(
                loaded.verified.index_range(),
                &preview.unsigned_index_range,
            )?
        {
            return Err(Error::TeamBinding(
                "joining team source role or index range",
            ));
        }
        // Preview range is advisory; destination rechecks its authenticated range
        // and the admitting client verifies both full chains before editing.
        Ok(())
    }
    pub(super) fn prepare_requested_team_membership(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &AuthenticatedTeamOutcome,
        source_role: Role,
        destination: &FqTeam,
    ) -> Result<PostGenericLinkArgument> {
        let (seed, certs) = credential.transport();
        // Prove source Admin/Owner authority independently of selected source role.
        self.activate_team_admin_bearer(host, credential.uid(), seed, certs, team)?;
        let verified = self.invitation_team_membership_chain(host, credential, team)?;
        let signer = team
            .ptks
            .iter()
            .filter(|k| k.role == Role::ADMIN)
            .max_by_key(|k| k.generation)
            .ok_or(Error::TeamBinding("source team admin key unavailable"))?;
        let public = team
            .verified
            .shared_key(Role::ADMIN)
            .ok_or(Error::TeamBinding("source admin key"))?;
        let next = crate::random_bytes::<32>()?;
        let unsigned = foks_proto::UnsignedUserLink::requested_membership(
            &foks_proto::RequestedMembershipLinkPublic {
                joiner: team.verified.team(),
                host: host.host_id(),
                signer: &public.verify_key,
                sequence: verified.tail.sequence,
                previous: verified.tail.previous,
                root: &team.verified.tree_root(),
                time: now_milliseconds()?,
                next_location_commitment: foks_crypto::prefixed_hash_signable(
                    foks_proto::TREE_LOCATION_TYPE_ID,
                    &foks_snowpack::encode(&foks_snowpack::Value::Binary(next.to_vec()))?,
                )?,
                team: destination,
                source_role,
            },
        )?;
        let signature = foks_crypto::sign_shared_key_typed(
            &signer.seed,
            foks_proto::LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&[])?,
        )?;
        Ok(PostGenericLinkArgument {
            link: unsigned.finish(vec![signature])?,
            next_tree_location: next,
        })
    }
    pub(super) fn invitation_team_membership_chain(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &AuthenticatedTeamOutcome,
    ) -> Result<super::VerifiedUserGenericChain> {
        let (seed, certs) = credential.transport();
        let response = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_load_team_membership_chain_request(
                team.verified.team(),
                host.host_id(),
                &team.view_token,
                1,
            )?,
            seed,
            certs,
        )?;
        let verified = super::verify_team_generic_chain(
            &response,
            &team.verified,
            foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP,
        )?;
        self.verify_generic_chain_roots(host, &team.verified.tree_root(), &verified)?;
        Ok(verified)
    }
    pub fn load_local_invitation_team(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        destination: &EntityId,
        row: &foks_proto::RawInboxRow,
    ) -> Result<super::VerifiedTeamRecipient> {
        // Reject a row this destination can never resolve before paying for
        // the authentication it would need, as this loader always has.
        local_team_joiner(row)?;
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let parent = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            destination,
        )?;
        self.load_local_invitation_team_with_team(host, credential, destination, &parent, row)
    }

    /// [`Self::load_local_invitation_team`] against a destination team the
    /// caller has already authenticated with the same credential, so an
    /// inbox resolves its rows against one destination load rather than one
    /// per row.
    pub fn load_local_invitation_team_with_team(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        destination: &EntityId,
        parent: &crate::AuthenticatedTeamOutcome,
        row: &foks_proto::RawInboxRow,
    ) -> Result<super::VerifiedTeamRecipient> {
        super::invitations::require_invitation_destination(host, destination, parent)?;
        let (joiner, source_role) = local_team_joiner(row)?;
        let (seed, certs) = credential.transport();
        let child = self.load_local_child_team_recipient_with_material(
            host,
            credential.uid(),
            seed,
            certs,
            parent,
            joiner,
            false,
        )?;
        if child.verified().shared_key(source_role).is_none()
            || !foks_verify::rational_range_strictly_before(
                child.verified().index_range(),
                parent.verified.index_range(),
            )?
            || parent.verified.members().iter().any(|m| {
                m.party == *joiner && m.scoped_host.is_none() && m.source_role == source_role
            })
        {
            return Err(Error::TeamBinding(
                "joining team source role, range or existing membership",
            ));
        }
        Ok(child)
    }
    pub fn verify_remote_invitation_team(
        &self,
        source: &PinnedHost,
        payload: &RemoteJoinPayload,
    ) -> Result<crate::RemoteTeamOutcome> {
        if &payload.joiner.host != source.host_id()
            || !matches!(
                payload.joiner.party.entity_type(),
                foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
            )
            || payload.source_role == Role::NONE
        {
            return Err(Error::TeamBinding("remote invitation team binding"));
        }
        let team =
            self.load_remote_team_and_pin(source, &payload.joiner.party, &payload.permission)?;
        if payload.visible.index_range.as_ref() != Some(team.verified.index_range())
            || team.verified.shared_key(payload.source_role).is_none()
        {
            return Err(Error::TeamBinding(
                "remote invitation team range or source role changed",
            ));
        }
        Ok(team)
    }
}

/// The team joiner an inbox row names, when the row is the local team request
/// the team-recipient loader resolves.
fn local_team_joiner(row: &foks_proto::RawInboxRow) -> Result<(&EntityId, Role)> {
    let foks_proto::RawInboxRequest::Local {
        joiner,
        source_role,
        ..
    } = &row.request
    else {
        return Err(Error::TeamBinding("expected local team request"));
    };
    Ok((joiner, *source_role))
}
