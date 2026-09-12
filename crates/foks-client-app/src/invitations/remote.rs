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
            InvitationAction::PreviewRemote { remote_profile, .. }
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
                self.finish_invitation_progress(progress, &mut protected, vault)
            }
            InvitationAction::SyncRemote { team_id, .. } => {
                let team = entity_id_from_hex(&team_id)?;
                let loaded = self
                    .client
                    .load_invited_remote_team(&home, &remote, credential, &team)?;
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
                let expanded = self
                    .client
                    .verify_remote_invitation_user(&remote, payload)?;
                Ok(
                    serde_json::json!({"request_id":request_id,"joiner_id":hex(expanded.user.verified.uid().as_bytes()),"host_id":hex(remote.host_id().as_bytes()),"username":String::from_utf8_lossy(expanded.user.verified.username_utf8()),"verified":true}),
                )
            }
            InvitationAction::ApproveRemote {
                team_alias,
                request_id,
                role,
                ..
            } => {
                handle(&request_id)?;
                if !matches!(role, TeamMemberRole::Member { .. }) {
                    return Err(Error::InvalidAccount(
                        "remote members cannot be administrators or owners",
                    ));
                }
                let mut stored = vault.team(&team_alias)?;
                if stored.account_alias != alias
                    || !stored.active
                    || stored.kind != crate::team::StoredTeamKind::Named
                {
                    return Err(Error::InvalidAccount(
                        "invitation requires this account's active named team",
                    ));
                }
                let team = EntityId::from_bytes(stored.team_id.clone())?;
                if let Some(existing) = stored
                    .invitation_members
                    .iter()
                    .find(|m| m.request_id == request_id)
                {
                    if existing.source_profile != other.profile.name
                        || existing.source_host != remote.host_id().as_bytes()
                        || existing.membership.destination_role.role()? != role.role()
                    {
                        return Err(Error::InvalidAccount("approval binding cannot change"));
                    }
                    let plan = crate::team::local_addition_plan(&existing.membership)?;
                    if HardStateStore::open(&self.paths.hard_database)?
                        .team_mutation(&plan.operation_id)?
                        .is_some()
                    {
                        let added = self.client.resume_invited_remote_addition(
                            &home,
                            credential,
                            &team,
                            remote.host_id(),
                            &plan,
                            &mut protected,
                        )?;
                        let found = stored
                            .invitation_members
                            .iter_mut()
                            .find(|m| m.request_id == request_id)
                            .unwrap();
                        found.membership.active = true;
                        vault.put_team(&stored)?;
                        return Ok(
                            serde_json::json!({"operation_id":hex(&added.operation_id),"request_id":request_id,"state":"complete","team_sequence":added.authenticated.verified.chain_seqno()}),
                        );
                    }
                }
                if stored.local_members.iter().any(|m| !m.active)
                    || stored
                        .invitation_members
                        .iter()
                        .any(|m| !m.membership.active && m.request_id != request_id)
                    || vault.team_member_edit(&team_alias)?.is_some()
                    || vault.team_rekey(&team_alias)?.is_some()
                {
                    return Err(Error::InvalidAccount(
                        "resume the pending team mutation first",
                    ));
                }
                if stored.invitation_members.len() >= 1000 {
                    return Err(Error::InvalidAccount(
                        "invitation member receipt capacity reached",
                    ));
                }
                let row =
                    self.invitation_inbox_handle(&home, credential, &team, &request_id, vault)?;
                let foks_proto::RawInboxRequest::Remote(request) = &row.request else {
                    return Err(Error::InvalidAccount("expected a remote request"));
                };
                let payload = self
                    .client
                    .open_remote_invitation(&home, credential, &team, request)?;
                let expanded = self
                    .client
                    .verify_remote_invitation_user(&remote, payload)?;
                let user = self
                    .client
                    .authenticate_credential_and_pin(&home, credential)?;
                let loaded = self.client.load_and_pin_team_with_credential(
                    &home,
                    credential,
                    &user.verified,
                    &user.puks,
                    &team,
                )?;
                let removal = SecretSeed::new(random_array()?);
                let plan = self.client.invited_remote_user_addition_plan(
                    credential.uid(),
                    &team,
                    &loaded,
                    &expanded,
                    role.role(),
                    &removal,
                )?;
                // No journal means no send. Replacing preflight-only material is safe;
                // a recorded exact edit always takes the recovery branch above.
                stored
                    .invitation_members
                    .retain(|m| m.request_id != request_id);
                stored.invitation_members.push(StoredInvitationMembership {
                    request_id: request_id.clone(),
                    source_profile: other.profile.name.clone(),
                    source_host: remote.host_id().as_bytes().to_vec(),
                    membership: StoredLocalMembership {
                        username: String::from_utf8_lossy(expanded.user.verified.username_utf8())
                            .into_owned(),
                        target_id: plan.target_id.as_bytes().to_vec(),
                        target_verify_key: plan.target_verify_key.as_bytes().to_vec(),
                        target_generation: plan.target_generation,
                        target_source_role: StoredTeamRole::from_role(plan.target_source_role),
                        destination_role: StoredTeamRole::from_role(plan.destination_role),
                        removal_key_commitment: plan.removal_key_commitment,
                        removal_key: *removal.as_bytes(),
                        expected_seqno: plan.expected_seqno,
                        operation_id: plan.operation_id,
                        active: false,
                    },
                });
                vault.put_team(&stored)?;
                let added = self.client.add_invited_remote_user_durable(
                    &home,
                    credential,
                    &team,
                    &expanded,
                    &row.receipt,
                    &plan,
                    &removal,
                    &mut protected,
                )?;
                stored
                    .invitation_members
                    .iter_mut()
                    .find(|m| m.request_id == request_id)
                    .unwrap()
                    .membership
                    .active = true;
                vault.put_team(&stored)?;
                Ok(
                    serde_json::json!({"operation_id":hex(&added.operation_id),"request_id":request_id,"state":"complete","team_sequence":added.authenticated.verified.chain_seqno()}),
                )
            }
            _ => Err(Error::InvalidAccount(
                "action does not use a remote profile",
            )),
        }
    }
}
