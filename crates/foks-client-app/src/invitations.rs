//! One invitation owner shared by CLI and desktop. Capability material stays native.
use super::*;
use foks_client::{FederationCredential, InvitationIntent};
use foks_proto::{InboxPagination, RawInboxRow, TeamInvite};

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InvitationAction {
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
            return Ok(serde_json::Value::Array(
                HardStateStore::open(&self.paths.hard_database)?
                    .invitation_operations(host.host_id().as_bytes(), uid.as_bytes())?
                    .iter()
                    .map(operation_report)
                    .collect(),
            ));
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
        let receipt_key = |id: &[u8; 16]| format!("invitation-receipt.{}", hex(id));
        if matches!(
            &action,
            InvitationAction::Create { .. }
                | InvitationAction::Accept { .. }
                | InvitationAction::Reject { .. }
        ) {
            self.cleanup_invitation_receipts(&host, credential.uid(), &mut protected, vault)?;
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
                vault.store.remove(&receipt_key(&id))?;
                Ok(serde_json::json!({"operation_id":operation_id,"state":"cancelled"}))
            }
            InvitationAction::Inbox { team_alias } => {
                let t = vault.team(&team_alias)?;
                if t.account_alias != alias {
                    return Err(Error::InvalidAccount("team belongs to another account"));
                }
                let team = EntityId::from_bytes(t.team_id.clone())?;
                let mut rows = self.client.team_invitation_inbox(
                    &host,
                    credential,
                    &team,
                    Some(InboxPagination {
                        start: 0,
                        end: 0,
                        limit: 100,
                    }),
                )?;
                // Go limits each request kind separately. Escalate a full first
                // page once; never skip an unrepresentable equal-timestamp group.
                if rows.iter().filter(|r| r.receipt.is_remote()).count() >= 100
                    || rows.iter().filter(|r| !r.receipt.is_remote()).count() >= 100
                {
                    rows = self.client.team_invitation_inbox(
                        &host,
                        credential,
                        &team,
                        Some(InboxPagination {
                            start: 0,
                            end: 0,
                            limit: 1000,
                        }),
                    )?;
                }
                let possibly_truncated = rows.iter().filter(|r| r.receipt.is_remote()).count()
                    >= 1000
                    || rows.iter().filter(|r| !r.receipt.is_remote()).count() >= 1000;
                let mut handles = Vec::new();
                let mut reports = Vec::new();
                for row in rows {
                    let id = hex(&random_array::<16>()?);
                    let expanded = self
                        .client
                        .load_local_invitation_joiner(&host, credential, &team, &row);
                    let mut report = serde_json::json!({"request_id":id,"time":row.time,"remote":row.receipt.is_remote()});
                    match expanded {
                        Ok(user) => {
                            report["joiner_id"] = hex(user.uid().as_bytes()).into();
                            report["username"] = String::from_utf8_lossy(user.username_utf8())
                                .into_owned()
                                .into();
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
                let key = inbox_key(&host, credential.uid(), &team);
                vault
                    .store
                    .put(&key, &Zeroizing::new(serde_json::to_vec(&handles)?))?;
                Ok(serde_json::json!({"rows":reports,"possibly_truncated":possibly_truncated}))
            }
            InvitationAction::Approve {
                team_alias,
                request_id,
                role,
            } => {
                let team = vault.team(&team_alias)?;
                if team.account_alias != alias {
                    return Err(Error::InvalidAccount("team belongs to another account"));
                }
                let team_id = EntityId::from_bytes(team.team_id.clone())?;
                let row =
                    self.invitation_inbox_handle(&host, credential, &team_id, &request_id, vault)?;
                let target = self
                    .client
                    .load_local_invitation_joiner(&host, credential, &team_id, &row)?;
                Ok(serde_json::to_value(self.add_verified_local_team_member(
                    &team_alias,
                    &target,
                    role,
                    vault,
                    master,
                )?)?)
            }
            InvitationAction::Reject {
                team_alias,
                request_id,
            } => {
                let team = vault.team(&team_alias)?;
                if team.account_alias != alias {
                    return Err(Error::InvalidAccount("team belongs to another account"));
                }
                let team = EntityId::from_bytes(team.team_id.clone())?;
                let row =
                    self.invitation_inbox_handle(&host, credential, &team, &request_id, vault)?;
                prepare(
                    InvitationIntent::Rejection {
                        team,
                        receipt: row.receipt,
                    },
                    &mut protected,
                )
            }
            _ => Err(Error::InvalidConfig(
                "invitation requires its remote profile",
            )),
        }
    }
    fn cleanup_invitation_receipts(
        &self,
        host: &foks_client::PinnedHost,
        uid: &EntityId,
        protected: &mut EncryptedFileMutationStore,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        let mut db = HardStateStore::open(&self.paths.hard_database)?;
        for old in db.expired_invitation_receipts(
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
            db.delete_invitation_receipt(&old.operation_id)?;
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
        if let Some(receipt) = progress.receipt {
            vault.store.put(
                &format!("invitation-receipt.{}", hex(&id)),
                &serde_json::to_vec(&serde_json::json!({"receipt":receipt.encoded()?}))?,
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
                        "invitation acknowledgement cleanup failed",
                    ))
                }
            }

            if let Ok(b) = vault.store.get(&format!("invitation-receipt.{}", hex(&id))) {
                let stored: serde_json::Value = serde_json::from_slice(&b)?;
                if let Some(invite) = stored.get("invite") {
                    result["invite"] = invite.clone();
                }
                if stored.get("receipt").is_some() {
                    result["delivery_acknowledged"] = true.into();
                }
            }
        }
        Ok(result)
    }

    fn invitation_inbox_handle(
        &self,
        host: &foks_client::PinnedHost,
        credential: FederationCredential<'_, '_>,
        team: &EntityId,
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
        let fresh = self.client.team_invitation_inbox(
            host,
            credential,
            team,
            Some(InboxPagination {
                start: row.time,
                end: row.time,
                limit: 1000,
            }),
        )?;
        fresh
            .into_iter()
            .find(|r| r.receipt == row.receipt)
            .ok_or(Error::InvalidAccount(
                "request is no longer in the pending inbox; refresh it",
            ))
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

mod remote;
pub(super) use remote::StoredInvitationMembership;
