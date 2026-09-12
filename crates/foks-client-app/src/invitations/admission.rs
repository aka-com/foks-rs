use super::*;
use crate::team::{StoredLocalMembership, StoredTeamRole};
use foks_client::{AddLocalTeamMemberRequest, LocalTeamMemberAdditionPlan};

enum Party {
    User(foks_verify::VerifiedUserState),
    RemoteUser(foks_client::ExpandedRemoteInvitation),
    Team(foks_client::VerifiedTeamRecipient),
    RemoteTeam(foks_client::RemoteTeamOutcome),
}
impl Party {
    fn name(&self) -> String {
        match self {
            Self::User(u) => String::from_utf8_lossy(u.username_utf8()).into_owned(),
            Self::RemoteUser(u) => {
                String::from_utf8_lossy(u.user.verified.username_utf8()).into_owned()
            }
            Self::Team(t) => String::from_utf8_lossy(t.verified().team_name_utf8()).into_owned(),
            Self::RemoteTeam(t) => {
                String::from_utf8_lossy(t.verified.team_name_utf8()).into_owned()
            }
        }
    }
}
impl CheckedProfileSession<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn approve_invitation(
        &self,
        other: Option<&CheckedProfileSession<'_>>,
        alias: &str,
        team_alias: &str,
        request_id: &str,
        role: TeamMemberRole,
        credential: FederationCredential<'_, '_>,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<serde_json::Value> {
        handle(request_id)?;
        let home = self.pinned_host()?;
        let source = other.map_or_else(|| self.pinned_host(), |s| s.pinned_host())?;
        let source_profile = other.map_or(&self.profile.name, |s| &s.profile.name);
        if other.is_some() && !matches!(role, TeamMemberRole::Member { .. }) {
            return Err(Error::InvalidAccount(
                "remote members cannot be administrators or owners",
            ));
        }
        let mut stored = vault.team(team_alias)?;
        if stored.account_alias != alias
            || !stored.active
            || stored.kind != crate::team::StoredTeamKind::Named
        {
            return Err(Error::InvalidAccount(
                "invitation requires this account's active named team",
            ));
        }
        let team = EntityId::from_bytes(stored.team_id.clone())?;
        let mut protected = self.mutation_store(master)?;
        if let Some(existing) = stored
            .invitation_members
            .iter()
            .find(|m| m.request_id == request_id)
        {
            if &existing.source_profile != source_profile
                || existing.source_host != source.host_id().as_bytes()
                || existing.membership.destination_role.role()? != role.role()
            {
                return Err(Error::InvalidAccount("approval binding cannot change"));
            }
            let plan = crate::team::local_addition_plan(&existing.membership)?;
            if HardStateStore::open(&self.paths.hard_database)?
                .team_mutation(&plan.operation_id)?
                .is_some()
            {
                let added = if other.is_some() {
                    self.client.resume_invited_remote_addition(
                        &home,
                        credential,
                        &team,
                        source.host_id(),
                        &plan,
                        &mut protected,
                    )?
                } else {
                    self.client.resume_invited_local_addition(
                        &home,
                        credential,
                        &team,
                        &plan,
                        &mut protected,
                    )?
                };
                stored
                    .invitation_members
                    .iter_mut()
                    .find(|m| m.request_id == request_id)
                    .unwrap()
                    .membership
                    .active = true;
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
            || vault.team_member_edit(team_alias)?.is_some()
            || vault.team_rekey(team_alias)?.is_some()
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
        let row = self.invitation_inbox_handle(&home, credential, &team, request_id, vault)?;
        let (party, source_role) = match (&row.request, other) {
            (
                foks_proto::RawInboxRequest::Local {
                    joiner,
                    source_role,
                    ..
                },
                None,
            ) => {
                let party = if joiner.entity_type() == foks_proto::ENTITY_USER {
                    Party::User(
                        self.client
                            .load_local_invitation_joiner(&home, credential, &team, &row)?,
                    )
                } else {
                    Party::Team(
                        self.client
                            .load_local_invitation_team(&home, credential, &team, &row)?,
                    )
                };
                (party, *source_role)
            }
            (foks_proto::RawInboxRequest::Remote(request), Some(_)) => {
                let payload = self
                    .client
                    .open_remote_invitation(&home, credential, &team, request)?;
                let source_role = payload.source_role;
                let party = if payload.joiner.party.entity_type() == foks_proto::ENTITY_USER {
                    Party::RemoteUser(
                        self.client
                            .verify_remote_invitation_user(&source, payload)?,
                    )
                } else {
                    Party::RemoteTeam(
                        self.client
                            .verify_remote_invitation_team(&source, &payload)?,
                    )
                };
                (party, source_role)
            }
            _ => {
                return Err(Error::InvalidAccount(
                    "request local/remote profile mismatch",
                ))
            }
        };
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
        let plan: LocalTeamMemberAdditionPlan = match &party {
            Party::User(u) => self.client.local_team_member_addition_plan(
                credential.uid(),
                &team,
                &loaded,
                &AddLocalTeamMemberRequest {
                    target_user: u,
                    destination_role: role.role(),
                    removal_key: &removal,
                },
            )?,
            Party::RemoteUser(u) => self.client.invited_remote_user_addition_plan(
                credential.uid(),
                &team,
                &loaded,
                u,
                role.role(),
                &removal,
            )?,
            Party::Team(t) => self.client.invited_team_addition_plan(
                credential.uid(),
                &loaded,
                t.verified(),
                source_role,
                role.role(),
                &removal,
            )?,
            Party::RemoteTeam(t) => self.client.invited_team_addition_plan(
                credential.uid(),
                &loaded,
                &t.verified,
                source_role,
                role.role(),
                &removal,
            )?,
        };
        stored
            .invitation_members
            .retain(|m| m.request_id != request_id);
        stored.invitation_members.push(StoredInvitationMembership {
            request_id: request_id.into(),
            source_profile: source_profile.clone(),
            source_host: source.host_id().as_bytes().to_vec(),
            membership: StoredLocalMembership {
                username: party.name(),
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
        let added = match &party {
            Party::User(u) => self.client.add_invited_local_user_durable(
                &home,
                credential,
                &team,
                &plan,
                &AddLocalTeamMemberRequest {
                    target_user: u,
                    destination_role: role.role(),
                    removal_key: &removal,
                },
                &mut protected,
            )?,
            Party::RemoteUser(u) => self.client.add_invited_remote_user_durable(
                &home,
                credential,
                &team,
                u,
                &row.receipt,
                &plan,
                &removal,
                &mut protected,
            )?,
            Party::Team(t) => self.client.add_invited_team_durable(
                &home,
                credential,
                &team,
                t.verified(),
                None,
                &row.receipt,
                &plan,
                &removal,
                &mut protected,
            )?,
            Party::RemoteTeam(t) => self.client.add_invited_team_durable(
                &home,
                credential,
                &team,
                &t.verified,
                Some(t.view_permission()),
                &row.receipt,
                &plan,
                &removal,
                &mut protected,
            )?,
        };
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
}
