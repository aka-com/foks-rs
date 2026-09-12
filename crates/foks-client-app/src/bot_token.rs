//! Bot selection metadata is persistent; imported signing material is session-owned.
use super::*;
mod revocation;
mod selection;
pub use revocation::BotRevocationReport;
pub use selection::BotSelection;

#[derive(Clone, Copy, Debug)]
pub enum BotEnrollmentAction {
    Status,
    Attempt,
    Cancel,
    Export,
}
#[derive(Clone, Serialize)]
pub struct BotEnrollmentReport {
    pub operation_id: String,
    pub account_alias: String,
    pub name: String,
    pub device_id: String,
    pub role: String,
    pub state: String,
    pub hardware_required: bool,
    pub export_available: bool,
}
pub struct BotEnrollmentOutcome {
    pub report: BotEnrollmentReport,
    pub exported: Option<Zeroizing<String>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingBot {
    id: [u8; 16],
    owner: String,
    uid: Vec<u8>,
    host: Vec<u8>,
    signer: Vec<u8>,
    device: Vec<u8>,
    name: String,
    role: Vec<u8>,
    token: Option<String>,
    cancelled: bool,
    created_at: u64,
}
impl Drop for PendingBot {
    fn drop(&mut self) {
        if let Some(t) = &mut self.token {
            t.zeroize();
        }
    }
}
fn enrollment_key(id: &[u8; 16]) -> String {
    format!("bot-enrollment.{}", hex(id))
}
impl PendingBot {
    fn save(&self, vault: &mut AccountVault<'_>) -> Result<()> {
        vault.store.put(
            &enrollment_key(&self.id),
            &Zeroizing::new(serde_json::to_vec(self)?),
        )?;
        Ok(())
    }
    fn load(
        vault: &mut AccountVault<'_>,
        id: [u8; 16],
        owner: &str,
        host: &EntityId,
    ) -> Result<Self> {
        let raw = vault.store.get(&enrollment_key(&id))?;
        let p: Self = serde_json::from_slice(&raw)?;
        if p.id != id || p.owner != owner || p.host != host.as_bytes() || p.name.len() != 5 {
            return Err(Error::InvalidAccount("bot enrollment binding changed"));
        }
        EntityId::from_bytes(p.uid.clone())?.require_type(ENTITY_USER)?;
        EntityId::from_bytes(p.device.clone())?.require_type(foks_proto::ENTITY_BOT_TOKEN_KEY)?;
        Role::decode(&p.role)?;
        if let Some(text) = &p.token {
            let token = foks_crypto::BotToken::import(text).map_err(|_| Error::BotToken)?;
            if token.public_material()?.id.as_bytes() != p.device || token.name() != p.name {
                return Err(Error::BotToken);
            }
        }
        Ok(p)
    }
    fn report(
        &self,
        op: Option<&foks_client_db::MutationOperation>,
    ) -> Result<BotEnrollmentReport> {
        let state = if self.cancelled {
            "rejected"
        } else {
            match op.map(|o| o.state) {
                None | Some(MutationState::Prepared) => "prepared",
                Some(MutationState::Submitting) => "submitting",
                Some(MutationState::SubmissionUnknown) => "submission-unknown",
                Some(MutationState::RemoteVerified) => "remote-verified",
                Some(MutationState::Rejected) => "rejected",
                Some(MutationState::Finalized) => "complete",
            }
        };
        Ok(BotEnrollmentReport {
            operation_id: hex(&self.id),
            account_alias: self.owner.clone(),
            name: self.name.clone(),
            device_id: hex(&self.device),
            role: format!("{:?}", Role::decode(&self.role)?),
            state: state.into(),
            hardware_required: false,
            export_available: self.token.is_some()
                && matches!(state, "remote-verified" | "complete"),
        })
    }
}
impl CheckedProfileSession<'_> {
    fn clean_bot_receipts(
        &self,
        uid: &[u8],
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
        before: u64,
    ) -> Result<()> {
        let host = self.pinned_host()?;
        let mut store = HardStateStore::open(&self.paths.hard_database)?;
        let mut protected = self.mutation_store(master)?;
        for op in store.expired_bot_enrollment_receipts(host.host_id().as_bytes(), uid, before)? {
            let key = enrollment_key(&op.operation_id);
            match vault.store.get(&key) {
                Ok(raw) => {
                    let p: PendingBot = serde_json::from_slice(&raw)?;
                    if p.token.is_some() {
                        continue;
                    }
                    if p.id != op.operation_id || p.uid != op.scope_id || p.host != op.host_id {
                        return Err(Error::BotToken);
                    }
                    let mut coordinator =
                        MutationCoordinator::new(&self.paths.hard_database, &mut protected);
                    if op.state == MutationState::Finalized {
                        coordinator.finalize(&op.operation_id)?;
                    } else {
                        coordinator.rejected(&op.operation_id)?;
                    }
                    vault.store.remove(&key)?;
                }
                Err(foks_keystore::Error::Missing) => {
                    let mut coordinator =
                        MutationCoordinator::new(&self.paths.hard_database, &mut protected);
                    if op.state == MutationState::Finalized {
                        coordinator.finalize(&op.operation_id)?;
                    } else {
                        coordinator.rejected(&op.operation_id)?;
                    }
                }
                Err(e) => return Err(e.into()),
            }
            store.delete_bot_enrollment_receipt(&op.operation_id)?;
        }
        let mut removed = 0;
        for key in vault
            .store
            .keys()?
            .into_iter()
            .filter(|k| k.starts_with("bot-enrollment."))
        {
            if removed >= 256 {
                break;
            }
            let raw = vault.store.get(&key)?;
            let p: PendingBot = serde_json::from_slice(&raw)?;
            if p.uid == uid
                && p.host == host.host_id().as_bytes()
                && p.cancelled
                && p.token.is_none()
                && p.created_at < before
                && store.mutation(&p.id)?.is_none()
            {
                vault.store.remove(&key)?;
                removed += 1;
            }
        }
        Ok(())
    }
    pub fn prepare_bot_account(
        &self,
        owner: &str,
        role: Role,
        parent: Option<&dyn foks_crypto::YubiDevice>,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<BotEnrollmentReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(owner)?;
        let host = self.pinned_host()?;
        let (uid, signer) = self.with_account_credential(owner, parent, vault, |c| {
            let auth = self.client.authenticate_credential_and_pin(&host, c)?;
            let signer = c.device_id()?;
            if role == Role::NONE
                || !auth
                    .verified
                    .devices()
                    .iter()
                    .any(|d| d.id == signer && d.role == Role::OWNER)
            {
                return Err(Error::InvalidAccount(
                    "bot enrollment requires owner authority and an explicit role",
                ));
            }
            Ok((
                c.uid().as_bytes().to_vec(),
                c.device_id()?.as_bytes().to_vec(),
            ))
        })?;
        self.clean_bot_receipts(
            &uid,
            vault,
            master,
            now_microseconds()?.saturating_sub(30 * 24 * 60 * 60 * 1_000_000),
        )?;
        let keys = vault
            .store
            .keys()?
            .into_iter()
            .filter(|k| k.starts_with("bot-enrollment."))
            .collect::<Vec<_>>();
        if keys.len() >= 4096 {
            return Err(Error::InvalidAccount("bot enrollment receipt capacity"));
        }
        let mut pending = 0;
        for key in keys {
            let raw = vault.store.get(&key)?;
            let p: PendingBot = serde_json::from_slice(&raw)?;
            if p.uid == uid && p.token.is_some() && !p.cancelled {
                let op = HardStateStore::open(&self.paths.hard_database)?.mutation(&p.id)?;
                if op.as_ref().is_none_or(|o| {
                    matches!(o.state, MutationState::Prepared | MutationState::Submitting)
                }) {
                    return Err(Error::InvalidAccount(
                        "recover or cancel the original prepared bot enrollment",
                    ));
                }
                pending += 1;
            }
        }
        if pending >= 32 {
            return Err(Error::InvalidAccount("bot enrollment recovery capacity"));
        }
        let token = foks_crypto::BotToken::generate()?;
        let p = PendingBot {
            id: random_array()?,
            owner: owner.into(),
            uid,
            host: host.host_id().as_bytes().to_vec(),
            signer,
            device: token.public_material()?.id.as_bytes().to_vec(),
            name: token.name(),
            role: role.encoded()?,
            token: Some(token.export().to_string()),
            cancelled: false,
            created_at: now_microseconds()?,
        };
        p.save(vault)?;
        self.bot_account_enrollment(
            owner,
            p.id,
            BotEnrollmentAction::Status,
            parent,
            vault,
            master,
        )
        .map(|o| o.report)
    }
    pub fn bot_account_enrollment(
        &self,
        owner: &str,
        id: [u8; 16],
        action: BotEnrollmentAction,
        parent: Option<&dyn foks_crypto::YubiDevice>,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<BotEnrollmentOutcome> {
        self.profile.require(Capability::DeviceAdministration)?;
        let host = self.pinned_host()?;
        let mut p = PendingBot::load(vault, id, owner, host.host_id())?;
        let mut op = HardStateStore::open(&self.paths.hard_database)?.mutation(&id)?;
        if op.as_ref().is_some_and(|o| {
            o.kind != MutationKind::BotEnrollment
                || o.host_id != p.host
                || o.scope_id != p.uid
                || o.subject_id != p.device
        }) {
            return Err(Error::InvalidAccount("bot journal binding changed"));
        }
        let mut protected = self.mutation_store(master)?;
        if matches!(action, BotEnrollmentAction::Cancel) {
            if op.as_ref().is_some_and(|o| {
                !matches!(o.state, MutationState::Prepared | MutationState::Rejected)
            }) {
                return Err(Error::InvalidAccount(
                    "submitted bot enrollment must be reconciled",
                ));
            }
            if op.is_some() {
                MutationCoordinator::new(&self.paths.hard_database, &mut protected)
                    .rejected(&id)?;
            }
            p.cancelled = true;
            drop(p.token.take().map(Zeroizing::new));
            p.save(vault)?;
        } else if !p.cancelled
            && p.token.is_some()
            && op.as_ref().is_none_or(|o| !o.state.is_terminal())
        {
            let result = self.with_account_credential(owner, parent, vault, |c| {
                if c.uid().as_bytes() != p.uid || c.device_id()?.as_bytes() != p.signer {
                    return Err(Error::InvalidAccount("bot owner credential changed"));
                }
                if op.is_none() {
                    let token =
                        foks_crypto::BotToken::import(p.token.as_ref().ok_or(Error::BotToken)?)
                            .map_err(|_| Error::BotToken)?;
                    op = Some(self.client.prepare_bot_enrollment_with_id(
                        &host,
                        c,
                        Role::decode(&p.role)?,
                        &token,
                        id,
                        &mut protected,
                    )?);
                }
                op = Some(self.client.bot_enrollment_progress(
                    &host,
                    c,
                    id,
                    matches!(action, BotEnrollmentAction::Attempt),
                    &mut protected,
                )?);
                Ok(())
            });
            if matches!(result, Err(Error::YubiUnlockRequired(_))) {
                let mut report = p.report(op.as_ref())?;
                report.hardware_required = true;
                return Ok(BotEnrollmentOutcome {
                    report,
                    exported: None,
                });
            }
            result?;
        }
        if op
            .as_ref()
            .is_some_and(|o| o.state == MutationState::RemoteVerified)
        {
            MutationCoordinator::new(&self.paths.hard_database, &mut protected).finalize(&id)?;
            op = HardStateStore::open(&self.paths.hard_database)?.mutation(&id)?;
        }
        if op
            .as_ref()
            .is_some_and(|o| o.state == MutationState::Rejected)
        {
            p.cancelled = true;
            drop(p.token.take().map(Zeroizing::new));
            p.save(vault)?;
        }
        if let Some(terminal) = op.as_ref().filter(|o| o.state.is_terminal()) {
            let mut coordinator =
                MutationCoordinator::new(&self.paths.hard_database, &mut protected);
            if terminal.state == MutationState::Finalized {
                coordinator.finalize(&id)?;
            } else {
                coordinator.rejected(&id)?;
            }
        }
        let exported = if matches!(action, BotEnrollmentAction::Export) {
            if op
                .as_ref()
                .is_none_or(|o| o.state != MutationState::Finalized)
                || p.cancelled
            {
                return Err(Error::InvalidAccount("bot enrollment is not verified"));
            }
            let secret = p
                .token
                .take()
                .ok_or(Error::InvalidAccount("bot token was already exported"))?;
            let secret = Zeroizing::new(secret);
            p.save(vault)?;
            Some(secret)
        } else {
            None
        };
        Ok(BotEnrollmentOutcome {
            report: p.report(op.as_ref())?,
            exported,
        })
    }
}

impl CheckedProfileSession<'_> {
    pub fn bot_account_enrollments(
        &self,
        owner: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<BotEnrollmentReport>> {
        validate_name(owner)?;
        let host = self.pinned_host()?;
        let store = HardStateStore::open(&self.paths.hard_database)?;
        let mut reports = Vec::new();
        for key in vault
            .store
            .keys()?
            .into_iter()
            .filter(|k| k.starts_with("bot-enrollment."))
        {
            let raw = vault.store.get(&key)?;
            let p: PendingBot = serde_json::from_slice(&raw)?;
            if p.owner == owner && p.host == host.host_id().as_bytes() {
                let op = store.mutation(&p.id)?;
                reports.push((p.created_at, p.report(op.as_ref())?));
            }
        }
        reports.sort_by_key(|(time, report)| {
            (
                !report.export_available
                    && matches!(report.state.as_str(), "complete" | "rejected"),
                std::cmp::Reverse(*time),
            )
        });
        Ok(reports.into_iter().take(160).map(|(_, p)| p).collect())
    }
}

#[cfg(test)]
mod tests;
