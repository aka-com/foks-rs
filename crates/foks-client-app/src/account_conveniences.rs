//! Frontend-independent account rename ownership and durable local label updates.
use super::*;
use foks_client::{FederationCredential, UsernameChangeProgress};

#[derive(Clone, Debug)]
pub enum RenameAction {
    Prepare(String),
    Attempt([u8; 16]),
    Status([u8; 16]),
    Cancel([u8; 16]),
}
#[derive(Clone, Debug, Serialize)]
pub struct RenameReport {
    pub operation_id: String,
    pub account_alias: String,
    pub state: &'static str,
    pub target: Option<String>,
    pub current_username: Option<String>,
    pub hardware_required: bool,
}
fn state(s: MutationState) -> &'static str {
    match s {
        MutationState::Prepared => "prepared",
        MutationState::Submitting => "submitting",
        MutationState::SubmissionUnknown => "submission-unknown",
        MutationState::RemoteVerified => "remote-verified",
        MutationState::Rejected => "rejected",
        MutationState::Finalized => "complete",
    }
}
fn report(alias: &str, p: UsernameChangeProgress) -> RenameReport {
    RenameReport {
        operation_id: hex(&p.operation.operation_id),
        account_alias: alias.into(),
        state: state(p.operation.state),
        target: p.target,
        current_username: p
            .current
            .as_ref()
            .map(|u| String::from_utf8_lossy(u.username_utf8()).into_owned()),
        hardware_required: false,
    }
}
impl CheckedProfileSession<'_> {
    pub fn account_renames(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<RenameReport>> {
        self.profile.require(Capability::DeviceAdministration)?;
        let (uid, device) = match vault.account(alias) {
            Ok(a) => (a.credential.uid.clone(), a.credential.public_material()?.id),
            Err(Error::AccountMissing) => {
                let a = vault.yubi_account(alias)?;
                (
                    a.uid,
                    EntityId::from_bytes(
                        [
                            vec![foks_proto::ENTITY_YUBI],
                            a.locator.signing_public_key.to_vec(),
                        ]
                        .concat(),
                    )?,
                )
            }
            Err(e) => return Err(e),
        };
        let host = self.pinned_host()?;
        Ok(HardStateStore::open(&self.paths.hard_database)?
            .username_changes(host.host_id().as_bytes(), uid.as_bytes(), device.as_bytes())?
            .into_iter()
            .map(|operation| {
                report(
                    alias,
                    UsernameChangeProgress {
                        operation,
                        target: None,
                        current: None,
                    },
                )
            })
            .collect())
    }

    pub fn rename_account(
        &self,
        alias: &str,
        action: RenameAction,
        parent: Option<&dyn foks_crypto::YubiDevice>,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<RenameReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(alias)?;
        let host = self.pinned_host()?;
        let mut protected = self.mutation_store(master)?;
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
            .map(|a| a.credential.uid.clone())
            .unwrap_or_else(|| hardware.as_ref().unwrap().uid.clone());
        let device = match &software {
            Some(a) => a.credential.public_material()?.id,
            None => EntityId::from_bytes(
                [
                    vec![foks_proto::ENTITY_YUBI],
                    hardware
                        .as_ref()
                        .unwrap()
                        .locator
                        .signing_public_key
                        .to_vec(),
                ]
                .concat(),
            )?,
        };
        if let RenameAction::Cancel(id) = action {
            self.client
                .cancel_username_change(&host, &uid, &device, id, &mut protected)?;
            let op = self
                .client
                .username_change_operation(&host, &uid, &device, id)?;
            return Ok(report(
                alias,
                UsernameChangeProgress {
                    operation: op,
                    target: None,
                    current: None,
                },
            ));
        }
        if software.is_none() && parent.is_none() {
            let id = match action {
                RenameAction::Status(id) | RenameAction::Attempt(id) => id,
                _ => {
                    return Err(Error::YubiUnlockRequired(
                        "unlock the enrolled key to prepare a rename".into(),
                    ))
                }
            };
            let op = self
                .client
                .username_change_operation(&host, &uid, &device, id)?;
            let mut p = report(
                alias,
                UsernameChangeProgress {
                    operation: op,
                    target: None,
                    current: None,
                },
            );
            p.hardware_required = !matches!(p.state, "complete" | "rejected");
            p.current_username = Some(hardware.as_ref().unwrap().username.clone());
            return Ok(p);
        }
        if parent.is_some_and(|p| p.entity_id() != &device) {
            return Err(Error::InvalidAccount(
                "rename key differs from selected account",
            ));
        }
        let yubi = hardware.as_ref().zip(parent).map(|(a, p)| a.credential(p));
        let credential = match &software {
            Some(a) => FederationCredential::Software(&a.credential),
            None => FederationCredential::Yubi(yubi.as_ref().unwrap()),
        };
        let progress = match action {
            RenameAction::Prepare(name) => {
                self.client
                    .prepare_username_change(&host, credential, &name, &mut protected)?
            }
            RenameAction::Attempt(id) => {
                self.client
                    .username_change_progress(&host, credential, id, true, &mut protected)?
            }
            RenameAction::Status(id) => self.client.username_change_progress(
                &host,
                credential,
                id,
                false,
                &mut protected,
            )?,
            RenameAction::Cancel(_) => unreachable!(),
        };
        if let Some(current) = &progress.current {
            self.refresh_verified_account_labels(current, vault)?;
        }
        let finalize = progress.operation.state == MutationState::RemoteVerified;
        let id = progress.operation.operation_id;
        let mut result = report(alias, progress);
        if finalize {
            MutationCoordinator::new(&self.paths.hard_database, &mut protected).finalize(&id)?;
            result.state = "complete";
        }
        Ok(result)
    }

    pub(super) fn refresh_verified_account_labels(
        &self,
        user: &foks_verify::VerifiedUserState,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        if user.host() != self.pinned_host()?.host_id() {
            return Err(Error::InvalidAccount(
                "label evidence belongs to another host",
            ));
        }
        vault.refresh_uid_labels(user.uid(), user.username_utf8())
    }
}

#[cfg(test)]
mod tests;
