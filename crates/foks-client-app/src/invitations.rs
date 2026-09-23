//! One invitation owner shared by CLI and desktop. Capability material stays native.
use super::*;
use foks_client::{FederationCredential, InvitationIntent};
use foks_proto::{InboxPagination, RawInboxRow, TeamInvite};

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InvitationAction {
    AcceptTeam {
        invite: String,
        source_team_alias: String,
        source_role: TeamMemberRole,
    },
    AcceptTeamRemote {
        remote_profile: String,
        invite: String,
        source_team_alias: String,
        source_role: TeamMemberRole,
    },
    Range {
        team_alias: String,
        raise: bool,
    },
    PendingApprovals {
        team_alias: String,
    },

    PreviewRemote {
        remote_profile: String,
        invite: String,
    },
    AcceptRemote {
        remote_profile: String,
        invite: String,
    },
    AttemptRemote {
        remote_profile: String,
        operation_id: String,
    },
    StatusRemote {
        remote_profile: String,
        operation_id: String,
    },
    InspectRemote {
        remote_profile: String,
        team_alias: String,
        request_id: String,
    },
    ApproveRemote {
        remote_profile: String,
        team_alias: String,
        request_id: String,
        role: TeamMemberRole,
    },
    SyncRemote {
        remote_profile: String,
        team_id: String,
        #[serde(default)]
        source_team_alias: Option<String>,
        #[serde(default)]
        source_role: Option<TeamMemberRole>,
    },

    Preview {
        invite: String,
    },
    Create {
        team_alias: String,
    },
    Accept {
        invite: String,
    },
    Attempt {
        operation_id: String,
    },
    Status {
        operation_id: String,
    },
    Cancel {
        operation_id: String,
    },
    List,
    Inbox {
        team_alias: String,
    },
    InboxCount {
        team_alias: String,
    },
    Approve {
        team_alias: String,
        request_id: String,
        role: TeamMemberRole,
    },
    Reject {
        team_alias: String,
        request_id: String,
    },
}
#[derive(Serialize, Deserialize)]
struct InboxHandle {
    id: String,
    row: Vec<u8>,
}
impl Drop for InboxHandle {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.row.zeroize();
    }
}
fn handle(s: &str) -> Result<[u8; 16]> {
    if s.len() != 32 {
        return Err(Error::InvalidAccount("invalid invitation handle"));
    }
    let mut id = [0; 16];
    for (i, p) in s.as_bytes().chunks_exact(2).enumerate() {
        id[i] = u8::from_str_radix(
            std::str::from_utf8(p)
                .map_err(|_| Error::InvalidAccount("invalid invitation handle"))?,
            16,
        )
        .map_err(|_| Error::InvalidAccount("invalid invitation handle"))?;
    }
    Ok(id)
}
fn operation_report(op: &foks_client_db::MutationOperation) -> serde_json::Value {
    serde_json::json!({"operation_id":hex(&op.operation_id),"team_id":hex(&op.subject_id),"state":match op.state {
        MutationState::Prepared=>"prepared", MutationState::Submitting=>"submitting", MutationState::SubmissionUnknown=>"submission-unknown", MutationState::RemoteVerified=>"acknowledged", MutationState::Finalized=>"complete", MutationState::Rejected=>"cancelled"
    }})
}
impl CheckedProfileSession<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn invitation_action(
        &self,
        alias: &str,
        action: InvitationAction,
        parent: Option<&dyn foks_crypto::YubiDevice>,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<serde_json::Value> {
        self.profile.require(Capability::Teams)?;
        validate_name(alias)?;
        let host = self.pinned_host()?;
        if let InvitationAction::Preview { invite } = &action {
            let p = self
                .client
                .preview_team_invitation(&host, &TeamInvite::import(invite)?)?;
            return Ok(
                serde_json::json!({"host_id":hex(p.advertised.team.host.as_bytes()),"team_id":hex(p.advertised.team.team.as_bytes()),"name":p.advertised.name.map(|n| String::from_utf8_lossy(&n).into_owned()),"membership":false}),
            );
        }
        // Obtain an owned credential so the operation may update encrypted app records.
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
        let uid = software
            .as_ref()
            .map(|a| &a.credential.uid)
            .unwrap_or_else(|| &hardware.as_ref().unwrap().uid);
        if matches!(&action, InvitationAction::List) {
            let mut protected = self.mutation_store(master)?;
            let mut rows = Vec::new();
            for op in HardStateStore::open(&self.paths.hard_database)?
                .invitation_operations(host.host_id().as_bytes(), uid.as_bytes())?
            {
                let mut row = operation_report(&op);
                let destination = self
                    .client
                    .invitation_destination(&host, &op, &mut protected)?;
                row["remote"] = destination.is_some().into();
                if let Some(host) = destination {
                    row["host_id"] = hex(host.as_bytes()).into();
                }
                rows.push(row);
            }
            return Ok(serde_json::Value::Array(rows));
        }
        if let InvitationAction::PendingApprovals { team_alias } = &action {
            let team = vault.team(team_alias)?;
            if team.account_alias != alias {
                return Err(Error::InvalidAccount("team account binding"));
            }
            let db = HardStateStore::open(&self.paths.hard_database)?;
            let mut rows = Vec::new();
            for m in &team.invitation_members {
                let operation = db.team_mutation(&m.membership.operation_id)?;
                rows.push(serde_json::json!({"request_id":m.request_id,"source_profile":m.source_profile,"remote":m.source_host != host.host_id().as_bytes(),"operation_id":hex(&m.membership.operation_id),"state":if m.membership.active {"complete"} else if operation.is_some() {"action-required"} else {"prepared"},"role":m.membership.destination_role}));
            }
            return Ok(serde_json::Value::Array(rows));
        }
        if let InvitationAction::Cancel { operation_id } = &action {
            let id = handle(operation_id)?;
            let op = self.client.invitation_operation(&host, uid, id)?;
            if op.state != MutationState::Prepared {
                return Err(Error::InvalidAccount(
                    "only an unsubmitted invitation can be cancelled",
                ));
            }
            let mut protected = self.mutation_store(master)?;
            MutationCoordinator::new(&self.paths.hard_database, &mut protected).rejected(&id)?;
            vault
                .store
                .remove(&format!("invitation-receipt.{}", hex(&id)))?;
            return Ok(serde_json::json!({"operation_id":operation_id,"state":"cancelled"}));
        }
        if hardware.is_some() && parent.is_none() {
            if let InvitationAction::Status { operation_id }
            | InvitationAction::Attempt { operation_id } = &action
            {
                let op = self
                    .client
                    .invitation_operation(&host, uid, handle(operation_id)?)?;
                let mut report = operation_report(&op);
                report["hardware_required"] = (!op.state.is_terminal()).into();
                return Ok(report);
            }
        }
        let yubi = hardware
            .as_ref()
            .map(|a| {
                let p = parent.ok_or_else(|| {
                    Error::YubiUnlockRequired("unlock the account key for invitations".into())
                })?;
                if p.entity_id().p256_key()? != a.locator.signing_public_key {
                    return Err(Error::InvalidAccount("wrong invitation account key"));
                }
                Ok::<_, Error>(a.credential(p))
            })
            .transpose()?;
        let credential = match &software {
            Some(a) => FederationCredential::Software(&a.credential),
            None => FederationCredential::Yubi(yubi.as_ref().unwrap()),
        };
        let mut protected = self.mutation_store(master)?;
        // The stored prefix is retained so existing completion records remain readable.
        let completion_record_key = |id: &[u8; 16]| format!("invitation-receipt.{}", hex(id));
        if matches!(
            &action,
            InvitationAction::Create { .. }
                | InvitationAction::Accept { .. }
                | InvitationAction::Reject { .. }
        ) {
            self.cleanup_invitation_completion_records(
                &host,
                credential.uid(),
                &mut protected,
                vault,
            )?;
        }
        let prepare =
            |intent, protected: &mut EncryptedFileMutationStore| -> Result<serde_json::Value> {
                Ok(operation_report(
                    &self
                        .client
                        .prepare_invitation_operation(&host, credential, intent, protected)?
                        .operation,
                ))
            };
        let attempt_action = matches!(&action, InvitationAction::Attempt { .. });
        match action {
            InvitationAction::Preview { .. } => unreachable!(),
            InvitationAction::List => Ok(serde_json::Value::Array(
                HardStateStore::open(&self.paths.hard_database)?
                    .invitation_operations(host.host_id().as_bytes(), credential.uid().as_bytes())?
                    .iter()
                    .map(operation_report)
                    .collect(),
            )),
            InvitationAction::AcceptTeam {
                invite,
                source_team_alias,
                source_role,
            } => {
                self.cleanup_invitation_completion_records(
                    &host,
                    credential.uid(),
                    &mut protected,
                    vault,
                )?;
                let source = vault.team(&source_team_alias)?;
                if source.account_alias != alias || !source.active {
                    return Err(Error::InvalidAccount("source team account binding"));
                }
                let p = self.client.prepare_local_team_invitation(
                    &host,
                    credential,
                    &EntityId::from_bytes(source.team_id.clone())?,
                    source_role.role(),
                    &TeamInvite::import(&invite)?,
                )?;
                prepare(InvitationIntent::LocalAcceptance(p), &mut protected)
            }
            InvitationAction::Range { team_alias, raise } => {
                let source = vault.team(&team_alias)?;
                if source.account_alias != alias || !source.active {
                    return Err(Error::InvalidAccount("source team account binding"));
                }
                let team = EntityId::from_bytes(source.team_id.clone())?;
                let r = self.client.narrow_team_index_range_durable(
                    &host,
                    credential,
                    &team,
                    if raise {
                        foks_client::TeamIndexRangeDirection::Raise
                    } else {
                        foks_client::TeamIndexRangeDirection::Lower
                    },
                    &mut protected,
                )?;
                Ok(
                    serde_json::json!({"operation_id":hex(&r.operation_id),"state":"complete","team_sequence":r.authenticated.verified.chain_seqno()}),
                )
            }
            InvitationAction::Create { team_alias } => {
                let t = vault.team(&team_alias)?;
                if t.account_alias != alias {
                    return Err(Error::InvalidAccount("team belongs to another account"));
                }
                let team = EntityId::from_bytes(t.team_id.clone())?;
                let prepared = self
                    .client
                    .prepare_team_invitation(&host, credential, &team)?;
                let result = prepare(
                    InvitationIntent::Certificate { team, prepared },
                    &mut protected,
                )?;
                Ok(result)
            }
            InvitationAction::Accept { invite } => {
                let a = self.client.prepare_local_invitation_acceptance(
                    &host,
                    credential,
                    &TeamInvite::import(&invite)?,
                )?;
                prepare(InvitationIntent::LocalAcceptance(a), &mut protected)
            }
            InvitationAction::Attempt { operation_id }
            | InvitationAction::Status { operation_id } => {
                let id = handle(&operation_id)?;
                // Discriminant was captured below before moving the action.
                let progress = self.client.invitation_progress(
                    &host,
                    credential,
                    id,
                    attempt_action,
                    &mut protected,
                )?;
                self.finish_invitation_progress(progress, &mut protected, vault)
            }
            InvitationAction::Cancel { operation_id } => {
                let id = handle(&operation_id)?;
                let op = self
                    .client
                    .invitation_operation(&host, credential.uid(), id)?;
                if op.state != MutationState::Prepared {
                    return Err(Error::InvalidAccount(
                        "only an unsubmitted invitation can be cancelled",
                    ));
                }
                MutationCoordinator::new(&self.paths.hard_database, &mut protected)
                    .rejected(&id)?;
                vault.store.remove(&completion_record_key(&id))?;
                Ok(serde_json::json!({"operation_id":operation_id,"state":"cancelled"}))
            }
            InvitationAction::InboxCount { team_alias } => {
                let team = self.invitation_inbox_team(&team_alias, alias, vault)?;
                let destination = self.invitation_inbox_destination(&host, credential, &team)?;
                let (rows, possibly_truncated) =
                    self.invitation_inbox_rows(&host, credential, &team, &destination)?;
                // The count is a badge. It expands no row, so it loads no
                // joiner, writes no handle blob and touches nothing in the
                // vault beyond resolving the team.
                Ok(serde_json::json!({"count":rows.len(),"possibly_truncated":possibly_truncated}))
            }
            InvitationAction::Inbox { team_alias } => {
                let team = self.invitation_inbox_team(&team_alias, alias, vault)?;
                // Every request this action makes is against the same
                // destination team, so authenticate this account and load
                // that team once here: the page read, its escalation and
                // every row are resolved against one authenticated view
                // instead of one per page and one per row.
                let destination = self.invitation_inbox_destination(&host, credential, &team)?;
                let (rows, possibly_truncated) =
                    self.invitation_inbox_rows(&host, credential, &team, &destination)?;
                let key = inbox_key(&host, credential.uid(), &team);
                let previous: Vec<InboxHandle> = match vault.store.get(&key) {
                    Ok(bytes) => serde_json::from_slice(&bytes)?,
                    Err(foks_keystore::Error::Missing) => Vec::new(),
                    Err(error) => return Err(error.into()),
                };
                let mut ids = Vec::new();
                for handle in &previous {
                    if let Some(row) = foks_proto::decode_team_inbox(&handle.row)?.pop() {
                        ids.push((row.rsvp, handle.id.clone()));
                    }
                }
                let mut handles = Vec::new();
                let mut reports = Vec::new();
                for row in rows {
                    let id = match ids.iter().find(|(rsvp, _)| rsvp == &row.rsvp) {
                        Some((_, id)) => id.clone(),
                        None => hex(&random_array::<16>()?),
                    };
                    let expanded = match &row.request {
                        foks_proto::RawInboxRequest::Local { joiner, .. }
                            if joiner.entity_type() != foks_proto::ENTITY_USER =>
                        {
                            self.client
                                .load_local_invitation_team_with_team(
                                    &host,
                                    credential,
                                    &team,
                                    &destination,
                                    &row,
                                )
                                .map(|t| {
                                    (
                                        t.verified().team().clone(),
                                        String::from_utf8_lossy(t.verified().team_name_utf8())
                                            .into_owned(),
                                        "team",
                                    )
                                })
                        }
                        _ => self
                            .client
                            .load_local_invitation_joiner_with_team(
                                &host,
                                credential,
                                &team,
                                &destination,
                                &row,
                            )
                            .map(|u| {
                                (
                                    u.uid().clone(),
                                    String::from_utf8_lossy(u.username_utf8()).into_owned(),
                                    "user",
                                )
                            }),
                    };
                    let mut report = serde_json::json!({"request_id":id,"time":row.time,"remote":row.rsvp.is_remote()});
                    match expanded {
                        Ok((party, name, kind)) => {
                            report["joiner_id"] = hex(party.as_bytes()).into();
                            report["username"] = name.into();
                            report["joiner_kind"] = kind.into();
                            report["verified"] = true.into();
                        }
                        Err(_) => {
                            report["verified"] = false.into();
                            report["error"] = "request could not be verified".into();
                        }
                    }
                    handles.push(InboxHandle {
                        id,
                        row: foks_proto::encode_team_inbox(&[row])?,
                    });
                    reports.push(report);
                }
                vault
                    .store
                    .put(&key, &Zeroizing::new(serde_json::to_vec(&handles)?))?;
                Ok(serde_json::json!({"rows":reports,"possibly_truncated":possibly_truncated}))
            }
            InvitationAction::Approve {
                team_alias,
                request_id,
                role,
            } => self.approve_invitation(
                None,
                alias,
                &team_alias,
                &request_id,
                role,
                credential,
                vault,
                master,
            ),
            InvitationAction::Reject {
                team_alias,
                request_id,
            } => {
                let team = vault.team(&team_alias)?;
                if team.account_alias != alias {
                    return Err(Error::InvalidAccount("team belongs to another account"));
                }
                let team = EntityId::from_bytes(team.team_id.clone())?;
                let row = self.invitation_inbox_handle(
                    &host,
                    credential,
                    &team,
                    None,
                    &request_id,
                    vault,
                )?;
                prepare(
                    InvitationIntent::Rejection {
                        team,
                        rsvp: row.rsvp,
                    },
                    &mut protected,
                )
            }
            _ => Err(Error::InvalidConfig(
                "invitation requires its remote profile",
            )),
        }
    }
    fn cleanup_invitation_completion_records(
        &self,
        host: &foks_client::PinnedHost,
        uid: &EntityId,
        protected: &mut EncryptedFileMutationStore,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        let mut db = HardStateStore::open(&self.paths.hard_database)?;
        for old in db.expired_invitation_completion_records(
            host.host_id().as_bytes(),
            uid.as_bytes(),
            now_microseconds()?.saturating_sub(30 * 24 * 60 * 60 * 1_000_000),
        )? {
            for key in [
                old.material_ref.clone(),
                [old.operation_id.as_slice(), b"/invitation-ack"].concat(),
            ] {
                match foks_client::ProtectedMutationStore::remove(protected, &key) {
                    Ok(()) | Err(foks_client::ProtectedStoreError::Missing) => {}
                    Err(_) => return Err(Error::InvalidAccount("invitation cleanup failed")),
                }
            }
            vault
                .store
                .remove(&format!("invitation-receipt.{}", hex(&old.operation_id)))?;
            db.delete_invitation_completion_record(&old.operation_id)?;
        }
        Ok(())
    }
    fn finish_invitation_progress(
        &self,
        progress: foks_client::InvitationProgress,
        protected: &mut EncryptedFileMutationStore,
        vault: &mut AccountVault<'_>,
    ) -> Result<serde_json::Value> {
        let id = progress.operation.operation_id;
        if let Some(invite) = progress.invite {
            vault.store.put(
                &format!("invitation-receipt.{}", hex(&id)),
                &serde_json::to_vec(&serde_json::json!({"invite":invite}))?,
            )?;
        }
        if let Some(rsvp) = progress.rsvp {
            vault.store.put(
                &format!("invitation-receipt.{}", hex(&id)),
                &serde_json::to_vec(&serde_json::json!({"rsvp":rsvp.encoded()?}))?,
            )?;
        }
        let mut result = operation_report(&progress.operation);
        if progress.operation.state == MutationState::RemoteVerified {
            MutationCoordinator::new(&self.paths.hard_database, protected).finalize(&id)?;
            result["state"] = "complete".into();
        }
        if result["state"] == "complete" {
            let key = [id.as_slice(), b"/invitation-ack"].concat();
            match foks_client::ProtectedMutationStore::remove(protected, &key) {
                Ok(()) | Err(foks_client::ProtectedStoreError::Missing) => {}
                Err(_) => {
                    return Err(Error::InvalidAccount(
                        "invitation acknowledgment cleanup failed",
                    ))
                }
            }

            if let Ok(b) = vault.store.get(&format!("invitation-receipt.{}", hex(&id))) {
                let stored: serde_json::Value = serde_json::from_slice(&b)?;
                if let Some(invite) = stored.get("invite") {
                    result["invite"] = invite.clone();
                }
                if stored
                    .get("rsvp")
                    .or_else(|| stored.get("receipt"))
                    .is_some()
                {
                    result["delivery_acknowledged"] = true.into();
                }
            }
        }
        Ok(result)
    }

    /// Resolves an inbox action's destination team without any network call,
    /// refusing a team this account does not own.
    fn invitation_inbox_team(
        &self,
        team_alias: &str,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<EntityId> {
        let team = vault.team(team_alias)?;
        if team.account_alias != alias {
            return Err(Error::InvalidAccount("team belongs to another account"));
        }
        Ok(EntityId::from_bytes(team.team_id.clone())?)
    }

    /// Authenticates this account and loads an inbox action's destination
    /// team once, for every request that action then makes.
    fn invitation_inbox_destination(
        &self,
        host: &foks_client::PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
    ) -> Result<foks_client::AuthenticatedTeamOutcome> {
        // Only the software arm has read caches; a hardware credential
        // authenticates on the card every time.
        if let FederationCredential::Software(software) = credential {
            let user = self.authenticated_user(host, software)?;
            return self.load_team_for_read(host, software, &user, team);
        }
        let user = self
            .client
            .authenticate_credential_and_pin(host, credential)?;
        Ok(self.client.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            team,
        )?)
    }

    /// The team's pending inbox rows and whether the server may have held
    /// more back. Go caps the merged page; Rust caps each request kind.
    /// A full combined page is escalated once; an equal-timestamp group is
    /// never skipped. Both inbox actions read their rows here, so the page
    /// they see and the truncation they report are one fact.
    fn invitation_inbox_rows(
        &self,
        host: &foks_client::PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        destination: &foks_client::AuthenticatedTeamOutcome,
    ) -> Result<(Vec<RawInboxRow>, bool)> {
        read_inbox_pages(|limit| {
            Ok(self.client.team_invitation_inbox_with_team(
                host,
                credential,
                team,
                destination,
                Some(InboxPagination {
                    start: 0,
                    end: 0,
                    limit,
                }),
            )?)
        })
    }

    /// Re-reads one stored handle from the live pending inbox. `destination`
    /// lets a caller that has already authenticated and loaded this team
    /// within the same operation supply that outcome instead of paying for
    /// another; the inbox page itself is still read from the server, so the
    /// row's continued presence is observed, not assumed.
    fn invitation_inbox_handle(
        &self,
        host: &foks_client::PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
        destination: Option<&foks_client::AuthenticatedTeamOutcome>,
        id: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<RawInboxRow> {
        handle(id)?;
        let b = vault.store.get(&inbox_key(host, credential.uid(), team))?;
        let stored: Vec<InboxHandle> = serde_json::from_slice(&b)?;
        let row = stored
            .into_iter()
            .find(|h| h.id == id)
            .ok_or(Error::InvalidAccount("refresh the invitation inbox"))?;
        let row = foks_proto::decode_team_inbox(&row.row)?
            .pop()
            .ok_or(Error::InvalidAccount("empty invitation handle"))?;
        let pagination = Some(inbox_handle_pagination(row.time));
        let fresh = match destination {
            Some(destination) => self.client.team_invitation_inbox_with_team(
                host,
                credential,
                team,
                destination,
                pagination,
            )?,
            None => self
                .client
                .team_invitation_inbox(host, credential, team, pagination)?,
        };
        fresh
            .into_iter()
            .find(|r| r.rsvp == row.rsvp)
            .ok_or(Error::InvalidAccount(
                "request is no longer in the pending inbox; refresh it",
            ))
    }
}
fn read_inbox_pages(
    mut page: impl FnMut(u64) -> Result<Vec<RawInboxRow>>,
) -> Result<(Vec<RawInboxRow>, bool)> {
    let mut rows = page(100)?;
    if rows.len() >= 100 {
        rows = page(1000)?;
    }
    let possibly_truncated = rows.len() >= 1000;
    Ok((rows, possibly_truncated))
}

fn inbox_handle_pagination(time: u64) -> InboxPagination {
    // Go stores sub-millisecond ctime values but exports Unix milliseconds.
    // Its inclusive end bound must cover the entire exported millisecond,
    // otherwise an exact-time lookup excludes a still-pending request.
    // The next millisecond's boundary can also be returned; the caller
    // matches the RSVP, not the timestamp, to select the original request.
    InboxPagination {
        start: time,
        end: time.saturating_add(1),
        limit: 1000,
    }
}
fn inbox_key(host: &foks_client::PinnedHost, uid: &EntityId, team: &EntityId) -> String {
    format!(
        "invitation-inbox.{}",
        hex(&foks_crypto::prefixed_hash(
            0xa0d1_447b_3649_61f8,
            &[host.host_id().as_bytes(), uid.as_bytes(), team.as_bytes()].concat()
        ))
    )
}

#[cfg(test)]
mod tests;

mod admission;
mod remote;
pub(super) use remote::StoredInvitationMembership;

/// Validate known vault families without refreshing or accepting an invitation.
pub(crate) fn validate_inventory_record(
    vault: &mut AccountVault<'_>,
    family: &str,
    suffix: &str,
    hard: &HardStateStore,
) -> Result<bool> {
    let bytes = vault.store.get(&format!("{family}.{suffix}"))?;
    if family == "invitation-inbox" {
        crate::portability::trust::validate_digest(suffix)?;
        let rows: Vec<InboxHandle> = serde_json::from_slice(&bytes)?;
        if rows.len() > 2000 {
            return Err(Error::InvalidAccount(
                "invitation inbox inventory exceeds limit",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for row in &rows {
            handle(&row.id)?;
            if !ids.insert(&row.id) || foks_proto::decode_team_inbox(&row.row)?.len() != 1 {
                return Err(Error::InvalidAccount("invalid invitation inbox record"));
            }
        }
        return Ok(true);
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InvitationCompletionRecord {
        invite: Option<String>,
        #[serde(alias = "receipt")]
        rsvp: Option<Vec<u8>>,
    }
    impl Drop for InvitationCompletionRecord {
        fn drop(&mut self) {
            if let Some(v) = &mut self.invite {
                v.zeroize();
            }
            if let Some(v) = &mut self.rsvp {
                v.zeroize();
            }
        }
    }
    let id = handle(suffix)?;
    let completion_record: InvitationCompletionRecord = serde_json::from_slice(&bytes)?;
    match (&completion_record.invite, &completion_record.rsvp) {
        (Some(invite), None) => {
            TeamInvite::import(invite)?;
        }
        (None, Some(rsvp)) => {
            foks_proto::TeamRsvp::decode(rsvp)?;
        }
        _ => {
            return Err(Error::InvalidAccount(
                "invalid invitation completion record",
            ))
        }
    }
    let op = hard.mutation(&id)?.ok_or(Error::InvalidAccount(
        "invitation completion record has no journal owner",
    ))?;
    if op.kind != MutationKind::Invitation {
        return Err(Error::InvalidAccount(
            "invitation completion record has wrong journal owner",
        ));
    }
    Ok(op.state.is_terminal())
}
