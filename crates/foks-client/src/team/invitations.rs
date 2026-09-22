//! Certificate preview and upload. A preview never supplies membership/PTK authority.
use crate::{now_milliseconds, Error, FederationCredential, FoksClient, PinnedHost, Result};
use foks_proto::{
    EntityId, FqTeam, LocalViewPermissionPayload, PermissionToken, Role, TeamCertificate,
    TeamCertificateAndMetadata, TeamCertificatePayload, TeamInvite,
};

pub struct InvitationPreview {
    pub invite: TeamInvite,
    pub certificate: TeamCertificate,
    pub advertised: TeamCertificatePayload,
    pub unsigned_index_range: foks_proto::RationalRange,
}
pub struct PreparedTeamInvitation {
    pub invite: TeamInvite,
    pub certificate: TeamCertificate,
}
impl FoksClient {
    pub fn preview_team_invitation(
        &self,
        host: &PinnedHost,
        invite: &TeamInvite,
    ) -> Result<InvitationPreview> {
        if host.host_id() != &invite.host {
            return Err(Error::TeamBinding(
                "invite host does not match pinned destination",
            ));
        }
        let response = self.call(
            host,
            &host.registration,
            &foks_rpc::encode_lookup_team_certificate_request(invite)?,
            None,
        )?;
        let reply = TeamCertificateAndMetadata::decode(&response)?;
        let advertised = foks_crypto::verify_team_certificate(&reply.certificate, invite)?;
        Ok(InvitationPreview {
            invite: invite.clone(),
            certificate: reply.certificate,
            advertised,
            unsigned_index_range: reply.index_range,
        })
    }
    pub fn prepare_team_invitation(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
    ) -> Result<PreparedTeamInvitation> {
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let loaded = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            team,
        )?;
        super::verified_team_private_history(&loaded)?;
        let current = loaded
            .ptks
            .iter()
            .filter(|k| k.role == Role::ADMIN)
            .max_by_key(|k| k.generation)
            .ok_or(Error::TeamBinding("invitation requires admin keys"))?;
        let first = loaded
            .ptks
            .iter()
            .find(|k| k.role == Role::ADMIN && k.generation == 1)
            .ok_or(Error::TeamBinding("invitation requires original admin key"))?;
        let certificate = foks_crypto::make_team_certificate(
            FqTeam::new(team.clone(), host.host_id().clone())?,
            &first.seed,
            &current.seed,
            current.generation,
            loaded.verified.team_name_utf8().to_vec(),
            now_milliseconds()?,
        )?;
        let invite = foks_crypto::team_certificate_invite(&certificate)?;
        Ok(PreparedTeamInvitation {
            invite,
            certificate,
        })
    }
    /// The certificate is public and immutable. A lost upload reply is reconciled
    /// by exact public hash lookup; no second upload occurs within this call.
    pub fn upload_team_invitation(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        prepared: &PreparedTeamInvitation,
    ) -> Result<InvitationPreview> {
        let advertised =
            foks_crypto::verify_team_certificate(&prepared.certificate, &prepared.invite)?;
        if &advertised.team.host != host.host_id() {
            return Err(Error::TeamBinding("certificate upload host"));
        }
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let loaded = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            &advertised.team.team,
        )?;
        let (seed, certs) = credential.transport();
        let token =
            self.activate_team_admin_bearer(host, credential.uid(), seed, certs, &loaded)?;
        let submission = self.call_void_with_material(
            host,
            &host.user,
            &foks_rpc::encode_put_team_certificate_request(&token, &prepared.certificate)?,
            seed,
            certs,
        );
        match self.preview_team_invitation(host, &prepared.invite) {
            Ok(found) if found.certificate == prepared.certificate => Ok(found),
            Ok(_) => Err(Error::TeamBinding("certificate read-back differs")),
            Err(e) => Err(submission.err().unwrap_or(e)),
        }
    }
    pub fn current_team_invitations(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
    ) -> Result<Vec<PreparedTeamInvitation>> {
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
        let response = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_current_team_certificates_request(&token)?,
            seed,
            certs,
        )?;
        if response.len() > 1024 * 1024 {
            return Err(Error::TeamBinding("certificate listing limit"));
        }
        let values = match foks_snowpack::decode(&response)? {
            foks_snowpack::Value::Null => vec![],
            foks_snowpack::Value::Array(v) if v.len() <= 64 => v,
            _ => return Err(Error::TeamBinding("certificate listing shape")),
        };
        let history = loaded.verified.shared_key_history()?;
        let current = history
            .iter()
            .filter(|k| k.role == Role::ADMIN)
            .max_by_key(|k| k.generation)
            .ok_or(Error::TeamBinding("current admin key missing"))?;
        let mut output = vec![];
        for value in values {
            let certificate = TeamCertificate::decode(&foks_snowpack::encode(&value)?)?;
            let invite = foks_crypto::team_certificate_invite(&certificate)?;
            let p = foks_crypto::verify_team_certificate(&certificate, &invite)?;
            if p.team.team != *team
                || p.team.host != *host.host_id()
                || p.key.generation != current.generation
                || p.key.verify_key != current.verify_key
                || p.key.hepk_fingerprint != current.hepk_fingerprint
            {
                return Err(Error::TeamBinding("certificate listing identity"));
            }
            output.push(PreparedTeamInvitation {
                invite,
                certificate,
            });
        }
        Ok(output)
    }
    pub fn grant_local_user_view(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        viewer: &EntityId,
        viewer_role: Role,
    ) -> Result<PermissionToken> {
        let payload = LocalViewPermissionPayload {
            viewee: credential.uid().clone(),
            viewer: viewer.clone(),
            time: now_milliseconds()?,
            viewer_role: Some(viewer_role),
        };
        let (seed, certs) = credential.transport();
        let reply = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_grant_local_user_view_request(&payload)?,
            seed,
            certs,
        )?;
        Ok(PermissionToken::decode(&reply)?)
    }
    pub fn grant_local_team_view(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        viewee: &EntityId,
        source_role: Role,
        viewer: &EntityId,
        viewer_role: Role,
    ) -> Result<PermissionToken> {
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let team = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            viewee,
        )?;
        let key = team
            .ptks
            .iter()
            .filter(|k| k.role == source_role)
            .max_by_key(|k| k.generation)
            .ok_or(Error::TeamBinding("view grant source key unavailable"))?;
        let payload = LocalViewPermissionPayload {
            viewee: viewee.clone(),
            viewer: viewer.clone(),
            time: now_milliseconds()?,
            viewer_role: Some(viewer_role),
        };
        let signature = foks_crypto::sign_shared_key_typed(
            &key.seed,
            foks_proto::LOCAL_VIEW_PERMISSION_TYPE_ID,
            &payload.encoded()?,
        )?;
        let (seed, certs) = credential.transport();
        let reply = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_grant_local_team_view_request(
                &payload,
                &signature,
                key.generation,
                key.role,
            )?,
            seed,
            certs,
        )?;
        Ok(PermissionToken::decode(&reply)?)
    }
}

impl FoksClient {
    pub fn prepare_local_invitation_acceptance(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        invite: &TeamInvite,
    ) -> Result<foks_proto::LocalInviteAcceptance> {
        let preview = self.preview_team_invitation(host, invite)?;
        let user = self.authenticate_credential_and_pin(host, credential)?;
        // A verified existing membership must use the role-change flow. In particular,
        // an owner cannot accidentally admit themselves at a lower role through an invite.
        match self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            &preview.advertised.team.team,
        ) {
            Ok(team)
                if team
                    .verified
                    .members()
                    .iter()
                    .any(|m| m.party == *credential.uid() && m.source_role == Role::OWNER) =>
            {
                return Err(Error::TeamRequest("already a member; use role changes"))
            }
            Ok(_) => {}
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: 1013 | 7004 | 7006 | 7010,
                ..
            })) => {}
            Err(e) => return Err(e),
        }
        let membership_link = self.prepare_requested_user_membership(
            host,
            credential,
            &user.verified,
            &preview.advertised.team,
        )?;
        Ok(foks_proto::LocalInviteAcceptance {
            invite: invite.clone(),
            source_role: Role::OWNER,
            source_token: None,
            membership_link: Some(membership_link),
        })
    }
    pub(super) fn prepare_requested_user_membership(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        user: &foks_verify::VerifiedUserState,
        destination: &FqTeam,
    ) -> Result<foks_proto::PostGenericLinkArgument> {
        let (seed, certs) = credential.transport();
        let tail = self.load_membership_chain_tail(host, credential.uid(), seed, certs, user)?;
        let next = crate::random_bytes::<32>()?;
        let signer = credential.device_id()?;
        let unsigned = foks_proto::UnsignedUserLink::requested_membership(
            &foks_proto::RequestedMembershipLinkPublic {
                joiner: credential.uid(),
                host: host.host_id(),
                signer: &signer,
                sequence: tail.sequence,
                previous: tail.previous,
                root: &user.tree_root(),
                time: now_milliseconds()?,
                next_location_commitment: foks_crypto::prefixed_hash_signable(
                    foks_proto::TREE_LOCATION_TYPE_ID,
                    &foks_snowpack::encode(&foks_snowpack::Value::Binary(next.to_vec()))?,
                )?,
                team: destination,
                source_role: Role::OWNER,
            },
        )?;
        let signing = unsigned.signing_bytes(&[])?;
        let signature = match credential {
            FederationCredential::Software(c) => foks_crypto::sign_shared_key_typed(
                &c.seed,
                foks_proto::LINK_OUTER_V1_TYPE_ID,
                &signing,
            )?,
            FederationCredential::Yubi(c) => {
                foks_crypto::sign_yubi_typed(c.parent, foks_proto::LINK_OUTER_V1_TYPE_ID, &signing)?
            }
        };
        Ok(foks_proto::PostGenericLinkArgument {
            link: unsigned.finish(vec![signature])?,
            next_tree_location: next,
        })
    }

    /// Submit this exact prepared request once. Caller must journal before calling;
    /// a lost response cannot be replayed to recover the server-assigned RSVP.
    pub fn submit_local_invitation_acceptance(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        prepared: &foks_proto::LocalInviteAcceptance,
    ) -> Result<foks_proto::TeamRsvp> {
        if prepared.invite.host != *host.host_id() {
            return Err(Error::TeamBinding("acceptance host"));
        }
        let link = prepared.membership_link.as_ref().ok_or(Error::TeamBinding(
            "client acceptance requires Requested link",
        ))?;
        let decoded = link.link.decode_generic()?;
        if prepared.source_token.is_none() && decoded.entity != *credential.uid() {
            return Err(Error::TeamBinding("acceptance owner"));
        }
        let (seed, certs) = credential.transport();
        let response = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_call(
                foks_rpc::TEAM_MEMBER_PROTOCOL_ID,
                0,
                &prepared.encoded()?,
                0,
            )?,
            seed,
            certs,
        )?;
        let receipt = foks_proto::TeamRsvp::decode(&response)?;
        if receipt.is_remote() {
            return Err(Error::TeamBinding("local receipt kind"));
        }
        Ok(receipt)
    }
    pub fn post_invitation_team_removal(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        removal: &foks_proto::TeamRemovalAndCommitment,
    ) -> Result<()> {
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let team = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            &removal.removal.payload.team,
        )?;
        let (seed, certs) = credential.transport();
        let token = self.activate_team_admin_bearer(host, credential.uid(), seed, certs, &team)?;
        self.call_void_with_material(
            host,
            &host.user,
            &foks_rpc::encode_post_team_removal_request(&token, removal)?,
            seed,
            certs,
        )
    }
    pub fn team_invitation_inbox(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        pagination: Option<foks_proto::InboxPagination>,
    ) -> Result<Vec<foks_proto::RawInboxRow>> {
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let loaded = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            team,
        )?;
        self.team_invitation_inbox_with_team(host, credential, team, &loaded, pagination)
    }

    /// [`Self::team_invitation_inbox`] against a destination team the caller
    /// has already authenticated with the same credential, so a caller that
    /// reads a page and then escalates it authenticates and loads the team
    /// once rather than once per page.
    pub fn team_invitation_inbox_with_team(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        loaded: &crate::AuthenticatedTeamOutcome,
        pagination: Option<foks_proto::InboxPagination>,
    ) -> Result<Vec<foks_proto::RawInboxRow>> {
        require_invitation_destination(host, team, loaded)?;
        let (seed, certs) = credential.transport();
        let token = self.activate_team_admin_bearer(host, credential.uid(), seed, certs, loaded)?;
        let response = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_load_team_inbox_request(&token, pagination.as_ref())?,
            seed,
            certs,
        )?;
        Ok(foks_proto::decode_team_inbox(&response)?)
    }
    pub fn reject_team_invitation(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        receipt: &foks_proto::TeamRsvp,
    ) -> Result<()> {
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
        self.call_void_with_material(
            host,
            &host.user,
            &foks_rpc::encode_reject_join_request(&token, receipt)?,
            seed,
            certs,
        )
    }
    /// Resolves a local user row using the destination's authorized view scope.
    /// An inbox row alone never supplies the user's identity keys.
    pub fn load_local_invitation_joiner(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        row: &foks_proto::RawInboxRow,
    ) -> Result<foks_verify::VerifiedUserState> {
        // Reject a row this destination can never resolve before paying for
        // the authentication it would need, as this loader always has.
        local_user_joiner(row)?;
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let loaded = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            team,
        )?;
        self.load_local_invitation_joiner_with_team(host, credential, team, &loaded, row)
    }

    /// [`Self::load_local_invitation_joiner`] against a destination team the
    /// caller has already authenticated with the same credential. A caller
    /// that resolves several rows of one inbox authenticates and loads the
    /// destination once instead of once per row; the row-specific work is
    /// unchanged.
    pub fn load_local_invitation_joiner_with_team(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        loaded: &crate::AuthenticatedTeamOutcome,
        row: &foks_proto::RawInboxRow,
    ) -> Result<foks_verify::VerifiedUserState> {
        require_invitation_destination(host, team, loaded)?;
        let (joiner, source_role) = local_user_joiner(row)?;
        if loaded
            .verified
            .members()
            .iter()
            .any(|m| m.party == *joiner && m.source_role == source_role)
        {
            return Err(Error::TeamRequest("requester already belongs to team"));
        }
        match credential {
            FederationCredential::Software(c) => {
                self.load_and_pin_user_as_local_team(host, c, joiner, &loaded.view_token)
            }
            FederationCredential::Yubi(c) => {
                self.load_and_pin_user_as_local_team_yubi(host, c, joiner, &loaded.view_token)
            }
        }
    }
}

/// The user joiner an inbox row names, when the row is the local owner-role
/// user request the joiner loader resolves.
fn local_user_joiner(row: &foks_proto::RawInboxRow) -> Result<(&EntityId, Role)> {
    let foks_proto::RawInboxRequest::Local {
        joiner,
        source_role,
        ..
    } = &row.request
    else {
        return Err(Error::TeamRequest("expected local request"));
    };
    if joiner.entity_type() != foks_proto::ENTITY_USER || *source_role != Role::OWNER {
        return Err(Error::TeamRequest("expected owner-role user joiner"));
    }
    Ok((joiner, *source_role))
}

/// Rejects a pre-loaded destination team that is not the team the caller
/// named, so a load supplied with that team can never resolve a row against
/// another team.
pub(crate) fn require_invitation_destination(
    host: &PinnedHost,
    team: &EntityId,
    loaded: &crate::AuthenticatedTeamOutcome,
) -> Result<()> {
    if loaded.verified.team() != team || loaded.verified.host() != host.host_id() {
        return Err(Error::TeamBinding(
            "invitation destination team does not match the loaded team",
        ));
    }
    Ok(())
}
