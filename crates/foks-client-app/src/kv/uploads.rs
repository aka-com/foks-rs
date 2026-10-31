//! Profile-scoped upload controls; receipts are checked before network authentication.
use super::*;
use foks_client_db::{
    UploadIntentBinding, UploadIssuanceContext, UploadLimits, UploadOperation, UploadScope,
    UploadState,
};
use foks_proto::UploadHandle;

#[derive(Clone, Debug, Serialize)]
pub struct KvUploadReport {
    pub upload_id: String,
    pub state: &'static str,
    pub prepared_chunks: u32,
    pub acknowledged_chunks: u32,
    pub plaintext_bytes: Option<u64>,
    pub reserved_bytes: u64,
    pub node_id: Option<String>,
    pub version: Option<u64>,
}
impl KvUploadReport {
    fn from_operation(
        hard: &HardStateStore,
        op: &UploadOperation,
        handle: UploadHandle,
    ) -> Result<Self> {
        let state = report_state(hard, &op.id, op.state)?;
        Ok(Self {
            upload_id: handle.to_string(),
            state,
            prepared_chunks: op.chunk_count,
            acknowledged_chunks: op.acknowledged_chunks,
            plaintext_bytes: op.plaintext_bytes,
            reserved_bytes: op.reserved_bytes,
            node_id: matches!(
                op.state,
                UploadState::RemoteVerified | UploadState::Finalized
            )
            .then(|| hex(&op.file_id)),
            version: op.receipt_version,
        })
    }
}
/// The reported state of an upload. A publication the server definitely
/// rejected is reported as `publication-rejected` rather than `publishing`:
/// nothing is being checked, and Resume publishes it again.
fn report_state(hard: &HardStateStore, id: &[u8; 16], state: UploadState) -> Result<&'static str> {
    if state == UploadState::Publishing {
        if let Some(child) = hard.upload(id)?.and_then(|op| op.namespace_child) {
            if hard
                .mutation(&child)?
                .is_some_and(|mutation| mutation.state == foks_client_db::MutationState::Rejected)
            {
                return Ok("publication-rejected");
            }
        }
    }
    Ok(upload_state(state))
}
fn upload_state(state: UploadState) -> &'static str {
    match state {
        UploadState::AwaitingSource => "awaiting-source",
        UploadState::InlineUntracked => "inline-untracked",
        UploadState::Preparing | UploadState::NeedsSource => "needs-source",
        UploadState::Ready => "ready",
        UploadState::Uploading => "uploading",
        UploadState::ContentComplete => "content-complete",
        UploadState::Publishing => "publishing",
        UploadState::RemoteVerified => "verified",
        UploadState::Finalized => "completed",
        UploadState::Blocked => "blocked",
        UploadState::Cancelled => "cancelled",
    }
}
impl KvUploadReport {
    fn from_summary(hard: &HardStateStore, op: &foks_client_db::UploadSummary) -> Result<Self> {
        let state = report_state(hard, &op.internal_id, op.state)?;
        Ok(Self {
            upload_id: op.handle.to_string(),
            state,
            prepared_chunks: op.chunk_count,
            acknowledged_chunks: op.acknowledged_chunks,
            plaintext_bytes: op.plaintext_bytes,
            reserved_bytes: op.reserved_bytes,
            node_id: matches!(
                op.state,
                UploadState::RemoteVerified | UploadState::Finalized
            )
            .then(|| hex(&op.file_id)),
            version: op.receipt_version,
        })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct KvUploadsPage {
    pub uploads: Vec<KvUploadReport>,
    pub next: Option<[u8; 32]>,
    pub capacity: KvUploadCapacity,
}
#[derive(Clone, Debug, Serialize)]
pub struct KvUploadCapacity {
    pub max_reserved_bytes: u64,
    pub max_pending: u32,
    pub max_receipts: u32,
    pub max_receipt_bytes: u64,
}
impl CheckedProfileSession<'_> {
    pub fn kv_upload_issuance(
        &self,
        alias: &str,
        team_id: Option<&str>,
        vault: &mut AccountVault<'_>,
    ) -> Result<UploadIssuanceContext> {
        self.profile.require(Capability::Kv)?;
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let party = team_id
            .map(entity_id_from_hex)
            .transpose()?
            .unwrap_or_else(|| account.credential.uid.clone());
        let context = HardStateStore::open(&self.paths.hard_database)?.upload_issuance_context(
            UploadScope {
                host_id: host.host_id().as_bytes(),
                uid: account.credential.uid.as_bytes(),
                party_id: party.as_bytes(),
            },
            self.adapter_clock.sample()?,
        )?;
        self.publish_upload_checkpoint()?;
        Ok(context)
    }

    pub fn prepare_kv_upload(
        &self,
        alias: &str,
        team_id: Option<&str>,
        handle: UploadHandle,
        intent: &KvUploadIntent,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvUploadReport> {
        self.profile.require(Capability::Kv)?;
        if handle.is_legacy() {
            return Err(foks_client_db::Error::UploadLegacyReadOnly.into());
        }
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let party = team_id
            .map(entity_id_from_hex)
            .transpose()?
            .unwrap_or_else(|| account.credential.uid.clone());
        let mut hard = HardStateStore::open(&self.paths.hard_database)?;
        let scope = UploadScope {
            host_id: host.host_id().as_bytes(),
            uid: account.credential.uid.as_bytes(),
            party_id: party.as_bytes(),
        };
        let binding = intent.binding()?;
        if let Some(existing) = hard.upload_registration(handle, scope)? {
            if existing.intent != Some(binding) {
                return Err(foks_client_db::Error::UploadIdentityConflict.into());
            }
            let op = hard
                .upload(&existing.internal_id)?
                .ok_or(Error::InvalidConfig("upload disappeared"))?;
            return KvUploadReport::from_operation(&hard, &op, handle);
        }
        let registration = hard.register_upload_intent(
            handle,
            random_array()?,
            scope,
            binding,
            self.adapter_clock.sample()?,
        )?;
        let op = hard
            .upload(&registration.internal_id)?
            .ok_or(Error::InvalidConfig("upload disappeared"))?;
        self.publish_upload_checkpoint()?;
        KvUploadReport::from_operation(&hard, &op, handle)
    }

    pub(super) fn claim_upload(
        &self,
        alias: &str,
        team_id: Option<&str>,
        handle: UploadHandle,
        intent: &KvUploadIntent,
        vault: &mut AccountVault<'_>,
    ) -> Result<foks_client::ClaimedFileUpload> {
        self.prepare_kv_upload(alias, team_id, handle, intent, vault)?;
        let party = team_id
            .map(entity_id_from_hex)
            .transpose()?
            .unwrap_or(vault.account(alias)?.credential.uid);
        let op = self.checked_upload(alias, Some(party.as_bytes()), handle, vault)?;
        let claim = foks_client::claim_file_upload(
            &self.paths.hard_database,
            handle,
            UploadScope {
                host_id: &op.host_id,
                uid: &op.uid,
                party_id: &op.party_id,
            },
            intent.binding()?,
        )?;
        self.publish_upload_checkpoint()?;
        Ok(claim)
    }

    fn publish_upload_checkpoint(&self) -> Result<()> {
        let root = self
            .paths
            .directory
            .parent()
            .and_then(Path::parent)
            .ok_or(Error::StatePathChanged)?;
        ClientCredentials::open(root)?.checkpoint_before_effect(self)
    }

    fn upload_report(&self, id: [u8; 16]) -> Result<KvUploadReport> {
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        let registration = hard
            .upload_registration_for_internal(&id)?
            .ok_or(Error::InvalidConfig("upload handle disappeared"))?;
        let op = hard
            .upload(&id)?
            .ok_or(Error::InvalidConfig("upload disappeared"))?;
        KvUploadReport::from_operation(&hard, &op, registration.handle)
    }

    pub(crate) fn finish_upload_adapter(&self, id: [u8; 16], master: &[u8; 32]) -> Result<()> {
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        let upload = hard
            .upload(&id)?
            .ok_or(Error::InvalidConfig("upload disappeared"))?;
        if let Some(parent) = upload.adapter_parent {
            let entry = hard
                .adapter_submission_for_internal(&parent)?
                .ok_or(Error::InvalidConfig("upload adapter receipt missing"))?;
            if matches!(
                upload.state,
                UploadState::RemoteVerified | UploadState::Finalized
            ) {
                self.finish_data_write(entry.handle, true, false, Some(upload.file_id), master)?;
            } else if upload.state == UploadState::Cancelled {
                self.finish_data_write(
                    entry.handle,
                    false,
                    entry.ancillary_committed,
                    None,
                    master,
                )?;
            }
        }
        Ok(())
    }

    pub fn kv_upload_capacity(&self) -> Result<KvUploadCapacity> {
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        let limits = hard.upload_limits()?;
        let retention = hard.upload_retention_limits()?;
        Ok(KvUploadCapacity {
            max_reserved_bytes: limits.max_reserved_bytes,
            max_pending: limits.max_pending,
            max_receipts: retention.max_rows,
            max_receipt_bytes: retention.max_bytes,
        })
    }
    pub fn set_kv_upload_capacity(
        &self,
        max_reserved_bytes: u64,
        max_pending: u32,
    ) -> Result<KvUploadCapacity> {
        HardStateStore::open(&self.paths.hard_database)?.set_upload_limits(UploadLimits {
            max_reserved_bytes,
            max_pending,
        })?;
        self.kv_upload_capacity()
    }
    pub fn kv_upload_page(
        &self,
        alias: &str,
        team_id: Option<&str>,
        after: Option<[u8; 32]>,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvUploadsPage> {
        self.profile.require(Capability::Kv)?;
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let party = team_id
            .map(entity_id_from_hex)
            .transpose()?
            .unwrap_or_else(|| account.credential.uid.clone());
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        let page = hard.upload_summaries_page(
            UploadScope {
                host_id: host.host_id().as_bytes(),
                uid: account.credential.uid.as_bytes(),
                party_id: party.as_bytes(),
            },
            after,
            128,
        )?;
        Ok(KvUploadsPage {
            uploads: page
                .items
                .iter()
                .map(|item| KvUploadReport::from_summary(&hard, item))
                .collect::<Result<_>>()?,
            next: page.next,
            capacity: self.kv_upload_capacity()?,
        })
    }
    pub fn list_kv_uploads(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<KvUploadReport>> {
        self.all_upload_summaries(alias, None, vault)
    }
    fn all_upload_summaries(
        &self,
        alias: &str,
        team: Option<&str>,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<KvUploadReport>> {
        let mut items = Vec::new();
        let mut after = None;
        loop {
            let page = self.kv_upload_page(alias, team, after, vault)?;
            items.extend(page.uploads);
            if items.len() > 16_384 {
                return Err(Error::InvalidConfig(
                    "upload inventory exceeds supported bound",
                ));
            }
            after = page.next;
            if after.is_none() {
                return Ok(items);
            }
        }
    }
    fn checked_upload(
        &self,
        alias: &str,
        party: Option<&[u8]>,
        id: UploadHandle,
        vault: &mut AccountVault<'_>,
    ) -> Result<UploadOperation> {
        self.profile.require(Capability::Kv)?;
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        let party_id = party.unwrap_or(account.credential.uid.as_bytes());
        let registration = hard
            .upload_registration(
                id,
                UploadScope {
                    host_id: host.host_id().as_bytes(),
                    uid: account.credential.uid.as_bytes(),
                    party_id,
                },
            )?
            .ok_or(foks_client_db::Error::UploadExpired)?;
        let op = hard
            .upload(&registration.internal_id)?
            .ok_or(Error::InvalidConfig("upload disappeared"))?;
        if op.host_id != host.host_id().as_bytes()
            || op.uid != account.credential.uid.as_bytes()
            || party.is_some_and(|party| party != op.party_id)
        {
            return Err(Error::InvalidConfig(
                "upload handle belongs to another store",
            ));
        }
        Ok(op)
    }
    pub fn cancel_kv_upload(
        &self,
        alias: &str,
        id: UploadHandle,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let uid = vault.account(alias)?.credential.uid;
        self.cancel_upload_scoped(alias, uid.as_bytes(), id, vault, master)
    }
    pub fn list_team_kv_uploads(
        &self,
        alias: &str,
        team: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<KvUploadReport>> {
        self.all_upload_summaries(alias, Some(team), vault)
    }
    pub fn cancel_team_kv_upload(
        &self,
        alias: &str,
        team_id_hex: &str,
        id: UploadHandle,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let team = entity_id_from_hex(team_id_hex)?;
        self.cancel_upload_scoped(alias, team.as_bytes(), id, vault, master)
    }
    fn cancel_upload_scoped(
        &self,
        alias: &str,
        party: &[u8],
        id: UploadHandle,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let op = self.checked_upload(alias, Some(party), id, vault)?;
        let id = op.id;
        let mut hard = HardStateStore::open(&self.paths.hard_database)?;
        if !op.state.terminal() {
            hard.cancel_upload(&id)?;
        }
        if let Some(parent) = op.adapter_parent {
            let entry = hard
                .adapter_submission_for_internal(&parent)?
                .ok_or(Error::InvalidConfig("upload adapter receipt missing"))?;
            if hard
                .upload(&id)?
                .is_some_and(|upload| upload.state == UploadState::Cancelled)
            {
                self.finish_data_write(
                    entry.handle,
                    false,
                    entry.ancillary_committed,
                    None,
                    master,
                )?;
            }
        }
        hard.record_upload_terminal(&id, self.adapter_clock.sample().ok())?;
        self.upload_report(id)
    }
    pub fn resume_kv_upload(
        &self,
        alias: &str,
        id: UploadHandle,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let account = vault.account(alias)?;
        let op = self.checked_upload(alias, Some(account.credential.uid.as_bytes()), id, vault)?;
        let id = op.id;
        let host = self.pinned_host()?;
        let mut protected = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master),
        )?;
        if foks_client::verified_file_upload(
            &self.paths.hard_database,
            &mut protected,
            id,
            &op.host_id,
            &op.uid,
            &op.party_id,
        )?
        .is_none()
        {
            let authenticated = self
                .client
                .authenticate_and_pin(&host, &account.credential)?;
            let mut session = self.client.user_kv_write_session(
                &host,
                &account.credential,
                &authenticated.verified,
                &authenticated.puks,
                &self.paths.soft_database,
                &mut protected,
            )?;
            session.resume_file_upload(id)?;
        }
        self.finish_upload_adapter(id, master)?;
        self.upload_report(id)
    }
    pub fn resume_team_kv_upload(
        &self,
        alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        id: UploadHandle,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let team_id = entity_id_from_hex(team_id_hex)?;
        let op = self.checked_upload(alias, Some(team_id.as_bytes()), id, vault)?;
        let id = op.id;
        let mut protected = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master),
        )?;
        if foks_client::verified_file_upload(
            &self.paths.hard_database,
            &mut protected,
            id,
            &op.host_id,
            &op.uid,
            &op.party_id,
        )?
        .is_none()
        {
            self.with_team_kv_write_session(
                alias,
                team_alias,
                team_id_hex,
                vault,
                master,
                |session| {
                    session.resume_file_upload(id)?;
                    Ok(())
                },
            )?;
        }
        self.finish_upload_adapter(id, master)?;
        self.upload_report(id)
    }
}

/// Allocate from a checked, checkpoint-published issuance context before source reads.
pub fn new_kv_upload_id(context: UploadIssuanceContext) -> Result<UploadHandle> {
    Ok(UploadHandle::new(
        context.database_id,
        context.issued_at,
        random_array()?,
    ))
}

#[derive(Clone, Debug, Serialize)]
pub struct KvUploadIntent {
    pub path: String,
    pub expected_size: Option<u64>,
    pub overwrite: bool,
    pub expected_version: Option<u64>,
    pub selected_dirent: Option<[u8; 16]>,
    pub read_role: KvRoleSummary,
    pub write_role: KvRoleSummary,
    pub mkdir_p: bool,
}
impl KvUploadIntent {
    pub fn checked(
        path: &str,
        expected_size: Option<u64>,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
    ) -> Self {
        Self {
            path: path.into(),
            expected_size,
            overwrite: precondition != KvMutationPrecondition::Create,
            expected_version: precondition.version(),
            selected_dirent: precondition.dirent_id(),
            read_role,
            write_role,
            mkdir_p,
        }
    }
    fn binding(&self) -> Result<UploadIntentBinding> {
        split_parent(&self.path)?;
        if self.path.len() > 4096 || self.expected_size.is_some_and(|s| s > 1024 * 1024 * 1024) {
            return Err(Error::InvalidConfig("invalid upload intent"));
        }
        Ok(UploadIntentBinding {
            fingerprint: prefixed_hash(0x7475_c2c7_39e8_7b0d, &serde_json::to_vec(self)?),
        })
    }
}

/// The core claims an implicit/explicit identity before its first source read.
/// This wrapper publishes that state before the underlying IPC reader emits ready.
pub(super) struct UploadCheckpointReader<'a, R> {
    reader: &'a mut R,
    session: &'a CheckedProfileSession<'a>,
    credentials: ClientCredentials,
    published: bool,
}
impl<R: Read> Read for UploadCheckpointReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if !self.published {
            self.credentials
                .checkpoint_before_effect(self.session)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            self.published = true;
        }
        self.reader.read(bytes)
    }
}
impl CheckedProfileSession<'_> {
    pub(super) fn upload_reader<'a, R: Read>(
        &'a self,
        reader: &'a mut R,
    ) -> Result<UploadCheckpointReader<'a, R>> {
        let root = self
            .paths
            .directory
            .parent()
            .and_then(Path::parent)
            .ok_or(Error::StatePathChanged)?;
        Ok(UploadCheckpointReader {
            reader,
            session: self,
            credentials: ClientCredentials::open(root)?,
            published: false,
        })
    }
}

#[cfg(test)]
mod tests;
