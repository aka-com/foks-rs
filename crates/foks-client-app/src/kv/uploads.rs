//! Profile-scoped upload controls; receipts are checked before network authentication.
use super::*;
use foks_client_db::{UploadLimits, UploadOperation, UploadState};

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
impl From<&UploadOperation> for KvUploadReport {
    fn from(op: &UploadOperation) -> Self {
        Self {
            upload_id: hex(&op.id),
            state: match op.state {
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
            },
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
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct KvUploadCapacity {
    pub max_reserved_bytes: u64,
    pub max_pending: u32,
}
impl CheckedProfileSession<'_> {
    pub fn prepare_kv_upload(
        &self,
        alias: &str,
        team_id: Option<&str>,
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
    ) -> Result<KvUploadReport> {
        self.profile.require(Capability::Kv)?;
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let party = team_id
            .map(entity_id_from_hex)
            .transpose()?
            .unwrap_or_else(|| account.credential.uid.clone());
        let mut hard = HardStateStore::open(&self.paths.hard_database)?;
        hard.record_upload_intent(
            id,
            host.host_id().as_bytes(),
            account.credential.uid.as_bytes(),
            party.as_bytes(),
            now_microseconds()?,
        )?;
        Ok(KvUploadReport::from(
            &hard
                .upload(&id)?
                .ok_or(Error::InvalidConfig("upload disappeared"))?,
        ))
    }

    fn finish_upload_adapter(&self, id: [u8; 16], master: &[u8; 32]) -> Result<()> {
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
            }
        }
        Ok(())
    }

    pub fn kv_upload_capacity(&self) -> Result<KvUploadCapacity> {
        let limits = HardStateStore::open(&self.paths.hard_database)?.upload_limits()?;
        Ok(KvUploadCapacity {
            max_reserved_bytes: limits.max_reserved_bytes,
            max_pending: limits.max_pending,
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
    pub fn list_kv_uploads(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<KvUploadReport>> {
        self.profile.require(Capability::Kv)?;
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        Ok(HardStateStore::open(&self.paths.hard_database)?
            .uploads(host.host_id().as_bytes(), account.credential.uid.as_bytes())?
            .iter()
            .filter(|op| op.party_id == account.credential.uid.as_bytes())
            .map(KvUploadReport::from)
            .collect())
    }
    fn checked_upload(
        &self,
        alias: &str,
        party: Option<&[u8]>,
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
    ) -> Result<UploadOperation> {
        self.profile.require(Capability::Kv)?;
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let op = HardStateStore::open(&self.paths.hard_database)?
            .upload(&id)?
            .ok_or(Error::InvalidConfig("upload handle is not recorded"))?;
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
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let uid = vault.account(alias)?.credential.uid;
        self.cancel_upload_scoped(alias, uid.as_bytes(), id, vault, master)
    }
    pub fn list_team_kv_uploads(
        &self,
        alias: &str,
        team_id_hex: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<KvUploadReport>> {
        self.profile.require(Capability::Kv)?;
        let team = entity_id_from_hex(team_id_hex)?;
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        Ok(HardStateStore::open(&self.paths.hard_database)?
            .uploads(host.host_id().as_bytes(), account.credential.uid.as_bytes())?
            .iter()
            .filter(|op| op.party_id == team.as_bytes())
            .map(KvUploadReport::from)
            .collect())
    }
    pub fn cancel_team_kv_upload(
        &self,
        alias: &str,
        team_id_hex: &str,
        id: [u8; 16],
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
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let op = self.checked_upload(alias, Some(party), id, vault)?;
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
        let mut protected = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master),
        )?;
        foks_client::cleanup_file_upload(&self.paths.hard_database, &mut protected, id)?;
        Ok(KvUploadReport::from(
            &hard
                .upload(&id)?
                .ok_or(Error::InvalidConfig("upload disappeared"))?,
        ))
    }
    pub fn resume_kv_upload(
        &self,
        alias: &str,
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let account = vault.account(alias)?;
        let op = self.checked_upload(alias, Some(account.credential.uid.as_bytes()), id, vault)?;
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
        Ok(KvUploadReport::from(
            &HardStateStore::open(&self.paths.hard_database)?
                .upload(&id)?
                .ok_or(Error::InvalidConfig("upload disappeared"))?,
        ))
    }
    pub fn resume_team_kv_upload(
        &self,
        alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<KvUploadReport> {
        let team_id = entity_id_from_hex(team_id_hex)?;
        let op = self.checked_upload(alias, Some(team_id.as_bytes()), id, vault)?;
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
        Ok(KvUploadReport::from(
            &HardStateStore::open(&self.paths.hard_database)?
                .upload(&id)?
                .ok_or(Error::InvalidConfig("upload disappeared"))?,
        ))
    }
}

/// Allocate before opening a source or sending a local IPC stream.
pub fn new_kv_upload_id() -> Result<[u8; 16]> {
    let mut id = [0; 16];
    getrandom::fill(&mut id).map_err(|_| Error::Randomness)?;
    Ok(id)
}

#[cfg(test)]
mod tests;
