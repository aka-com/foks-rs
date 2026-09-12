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
