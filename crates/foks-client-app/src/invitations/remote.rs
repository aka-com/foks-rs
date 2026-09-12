use super::*;
use crate::team::{StoredLocalMembership, StoredTeamRole};
#[derive(Deserialize, Serialize)]
pub(crate) struct StoredInvitationMembership {
    pub request_id: String,
    pub source_profile: String,
    pub source_host: Vec<u8>,
    pub membership: StoredLocalMembership,
}
impl std::fmt::Debug for StoredInvitationMembership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoredInvitationMembership { [REDACTED] }")
    }
}
impl Drop for StoredInvitationMembership {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.membership.removal_key.zeroize();
    }
}
impl CheckedProfileSession<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn remote_invitation_action(
        &self,
        other: &CheckedProfileSession<'_>,
        alias: &str,
        action: InvitationAction,
        parent: Option<&dyn foks_crypto::YubiDevice>,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<serde_json::Value> {
        validate_name(alias)?;
        let selected = match &action {
            InvitationAction::AcceptTeamRemote { remote_profile, .. }
            | InvitationAction::PreviewRemote { remote_profile, .. }
            | InvitationAction::AcceptRemote { remote_profile, .. }
            | InvitationAction::AttemptRemote { remote_profile, .. }
            | InvitationAction::StatusRemote { remote_profile, .. }
            | InvitationAction::InspectRemote { remote_profile, .. }
            | InvitationAction::ApproveRemote { remote_profile, .. }
            | InvitationAction::SyncRemote { remote_profile, .. } => remote_profile,
            _ => return Err(Error::InvalidConfig("action requires a remote profile")),
        };
        if selected != &other.profile.name {
            return Err(Error::InvalidConfig("invitation profile binding mismatch"));
        }
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Federation)?;
        other.profile.require(Capability::Federation)?;
        let home = self.pinned_host()?;
        let remote = other.pinned_host()?;
        if home.host_id() == remote.host_id() {
            return Err(Error::InvalidConfig(
                "remote invitation profiles must identify different hosts",
            ));
        }
        if let InvitationAction::PreviewRemote { invite, .. } = &action {
            let p = self
                .client
                .preview_team_invitation(&remote, &TeamInvite::import(invite)?)?;
            return Ok(
                serde_json::json!({"host_id":hex(remote.host_id().as_bytes()),"team_id":hex(p.advertised.team.team.as_bytes()),"name":p.advertised.name.map(|n|String::from_utf8_lossy(&n).into_owned()),"membership":false}),
            );
        }
        let software = match vault.account(alias) {
            Ok(a) => Some(a),
            Err(Error::AccountMissing) => None,
            Err(e) => return Err(e),
        };
        let hardware = if software.is_none() {
            Some(vault.yubi_account(alias)?)
        } else {
            None
        };
        if let (Some(account), None) = (&hardware, parent) {
            if let InvitationAction::StatusRemote { operation_id, .. }
            | InvitationAction::AttemptRemote { operation_id, .. } = &action
            {
                let op =
                    self.client
                        .invitation_operation(&home, &account.uid, handle(operation_id)?)?;
                let mut report = operation_report(&op);
                report["hardware_required"] = (!op.state.is_terminal()).into();
                return Ok(report);
            }
        }
        let yubi = hardware
            .as_ref()
            .map(|a| {
                let p = parent.ok_or_else(|| {
                    Error::YubiUnlockRequired("unlock the invitation account key".into())
                })?;
                if p.entity_id().p256_key()? != a.locator.signing_public_key {
                    return Err(Error::InvalidAccount("wrong invitation key"));
                }
                Ok::<_, Error>(a.credential(p))
            })
            .transpose()?;
        let credential = match &software {
            Some(a) => FederationCredential::Software(&a.credential),
            None => FederationCredential::Yubi(yubi.as_ref().unwrap()),
        };
        let mut protected = self.mutation_store(master)?;
        let attempt = matches!(&action, InvitationAction::AttemptRemote { .. });
        match action {
            InvitationAction::AcceptTeamRemote {
                invite,
                source_team_alias,
                source_role,
                ..
            } => {
                self.cleanup_invitation_receipts(&home, credential.uid(), &mut protected, vault)?;
                let source = vault.team(&source_team_alias)?;
                if source.account_alias != alias || !source.active {
                    return Err(Error::InvalidAccount("source team account binding"));
                }
                let p = self.client.prepare_remote_team_invitation(
                    &home,
                    &remote,
                    credential,
                    &EntityId::from_bytes(source.team_id.clone())?,
                    source_role.role(),
                    &TeamInvite::import(&invite)?,
                )?;
                let progress = self.client.prepare_invitation_operation(
                    &home,
                    credential,
                    InvitationIntent::RemoteAcceptance(Box::new(p)),
                    &mut protected,
                )?;
                let mut report = operation_report(&progress.operation);
                report["remote"] = true.into();
                report["remote"] = true.into();
                report["remote_profile"] = other.profile.name.clone().into();
                Ok(report)
            }
            InvitationAction::AcceptRemote { invite, .. } => {
                self.cleanup_invitation_receipts(&home, credential.uid(), &mut protected, vault)?;
                let p = self.client.prepare_remote_user_invitation(
                    &home,
                    &remote,
                    credential,
                    &TeamInvite::import(&invite)?,
                )?;
                let progress = self.client.prepare_invitation_operation(
                    &home,
                    credential,
                    InvitationIntent::RemoteAcceptance(Box::new(p)),
                    &mut protected,
                )?;
                let mut report = operation_report(&progress.operation);
                report["remote"] = true.into();
                report["remote_profile"] = other.profile.name.clone().into();
                Ok(report)
            }
            InvitationAction::AttemptRemote { operation_id, .. }
            | InvitationAction::StatusRemote { operation_id, .. } => {
                let progress = self.client.remote_invitation_progress(
                    &home,
                    &remote,
                    credential,
                    handle(&operation_id)?,
                    attempt,
                    &mut protected,
                )?;
                let mut report =
                    self.finish_invitation_progress(progress, &mut protected, vault)?;
                report["remote"] = true.into();
                report["remote_profile"] = other.profile.name.clone().into();
                Ok(report)
            }
            InvitationAction::SyncRemote {
                team_id,
                source_team_alias,
                source_role,
                ..
            } => {
                let team = entity_id_from_hex(&team_id)?;
                let loaded = if let Some(alias_of_source) = source_team_alias {
                    let stored = vault.team(&alias_of_source)?;
                    if stored.account_alias != alias || !stored.active {
                        return Err(Error::InvalidAccount("source team account binding"));
                    }
                    self.client.load_invited_remote_team_as_team(
                        &home,
                        &remote,
                        credential,
                        &EntityId::from_bytes(stored.team_id.clone())?,
                        source_role.unwrap_or(TeamMemberRole::Admin).role(),
                        &team,
                    )?
                } else {
                    self.client
                        .load_invited_remote_team(&home, &remote, credential, &team)?
                };
                Ok(
                    serde_json::json!({"team_id":team_id,"host_id":hex(remote.host_id().as_bytes()),"membership_verified":true,"key_generations":loaded.ptks.len(),"team_sequence":loaded.verified.chain_seqno()}),
                )
            }
            InvitationAction::InspectRemote {
                team_alias,
                request_id,
                ..
            } => {
                let stored = vault.team(&team_alias)?;
                if stored.account_alias != alias {
                    return Err(Error::InvalidAccount("team belongs to another account"));
                }
                let team = EntityId::from_bytes(stored.team_id.clone())?;
                let row =
                    self.invitation_inbox_handle(&home, credential, &team, &request_id, vault)?;
                let foks_proto::RawInboxRequest::Remote(request) = row.request else {
                    return Err(Error::InvalidAccount(
                        "local request does not need a remote profile",
                    ));
                };
                let payload = self
                    .client
                    .open_remote_invitation(&home, credential, &team, &request)?;
                if payload.joiner.party.entity_type() == foks_proto::ENTITY_USER {
                    let expanded = self
                        .client
                        .verify_remote_invitation_user(&remote, payload)?;
                    Ok(
                        serde_json::json!({"request_id":request_id,"joiner_id":hex(expanded.user.verified.uid().as_bytes()),"host_id":hex(remote.host_id().as_bytes()),"username":String::from_utf8_lossy(expanded.user.verified.username_utf8()),"verified":true,"joiner_kind":"user"}),
                    )
                } else {
                    let source_role = payload.source_role;
                    let expanded = self
                        .client
                        .verify_remote_invitation_team(&remote, &payload)?;
                    Ok(
                        serde_json::json!({"request_id":request_id,"joiner_id":hex(expanded.verified.team().as_bytes()),"host_id":hex(remote.host_id().as_bytes()),"username":String::from_utf8_lossy(expanded.verified.team_name_utf8()),"verified":true,"joiner_kind":"team","source_role":StoredTeamRole::from_role(source_role)}),
                    )
                }
            }

            InvitationAction::ApproveRemote {
                team_alias,
                request_id,
                role,
                ..
            } => self.approve_invitation(
                Some(other),
                alias,
                &team_alias,
                &request_id,
                role,
                credential,
                vault,
                master,
            ),
            _ => Err(Error::InvalidAccount(
                "action does not use a remote profile",
            )),
        }
    }
}
