//! Explicit local upload-clock repair; adapter receipts use a separate domain.
use crate::{Error, Result};
use foks_client_db::UploadTimeSample;

#[derive(Debug, serde::Serialize)]
pub struct UploadClockPreview {
    pub host_id: String,
    pub user_id: String,
    pub anchor_wall: u64,
    pub validated_time_floor: u64,
    pub reject_issued_before: u64,
    pub proposed_wall: u64,
    pub confirmation_digest: String,
}

#[derive(Debug, serde::Serialize)]
pub struct UploadClockRepair {
    pub preview: UploadClockPreview,
    pub resulting_floor: u64,
    pub checkpoint_digest: String,
}

impl crate::CheckedProfileSession<'_> {
    pub fn upload_clock_preview(
        &self,
        alias: &str,
        proposed: u64,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<UploadClockPreview> {
        let (host_id, user_id) = self.data_identity(alias, vault)?;
        let host = crate::entity_id_from_hex(&host_id)?;
        let user = crate::entity_id_from_hex(&user_id)?;
        let state = foks_client_db::HardStateStore::open(&self.paths.hard_database)?
            .upload_clock(host.as_bytes(), user.as_bytes())?
            .ok_or(Error::InvalidAccount("upload clock has not been anchored"))?;
        let mut binding = Vec::new();
        binding.extend_from_slice(host.as_bytes());
        binding.extend_from_slice(user.as_bytes());
        binding.extend_from_slice(&state.process_id);
        for value in [
            state.anchor_wall,
            state.anchor_monotonic,
            state.validated_time_floor,
            state.reject_issued_before,
            proposed,
        ] {
            binding.extend_from_slice(&value.to_be_bytes());
        }
        Ok(UploadClockPreview {
            host_id,
            user_id,
            anchor_wall: state.anchor_wall,
            validated_time_floor: state.validated_time_floor,
            reject_issued_before: state.reject_issued_before,
            proposed_wall: proposed,
            confirmation_digest: crate::hex(&foks_crypto::prefixed_hash(
                0x086a_9c32_a21e_7351,
                &binding,
            )),
        })
    }

    /// Only the local CLI exposes this operation. The caller publishes the
    /// returned audit result after the checked-session boundary has checkpointed.
    pub fn reanchor_upload_clock(
        &self,
        alias: &str,
        proposed: u64,
        confirmation: &str,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<UploadClockRepair> {
        let preview = self.upload_clock_preview(alias, proposed, vault)?;
        if confirmation != preview.confirmation_digest {
            return Err(foks_client_db::Error::UploadIdentityConflict.into());
        }
        let host = crate::entity_id_from_hex(&preview.host_id)?;
        let user = crate::entity_id_from_hex(&preview.user_id)?;
        let mut hard = foks_client_db::HardStateStore::open(&self.paths.hard_database)?;
        let old = hard
            .upload_clock(host.as_bytes(), user.as_bytes())?
            .ok_or(foks_client_db::Error::UploadIdentityConflict)?;
        let sample = self.adapter_clock.sample()?;
        hard.reanchor_upload_clock(
            host.as_bytes(),
            user.as_bytes(),
            &old,
            UploadTimeSample {
                wall_seconds: proposed,
                ..sample
            },
        )?;
        Ok(UploadClockRepair {
            preview,
            resulting_floor: hard
                .upload_clock(host.as_bytes(), user.as_bytes())?
                .ok_or(foks_client_db::Error::UploadClockUntrusted)?
                .validated_time_floor,
            checkpoint_digest: crate::hex(&self.rollback_checkpoint()?.digest()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    #[test]
    fn upload_clock_repair_requires_preview_preserves_floor_and_adapter_domain() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, v, k| s.create_account("me", "uploadclock", "laptop", "", "", None, v, k));
        let context = f.run(|s, v, _| s.kv_upload_issuance("me", None, v));
        let proposed = context.issued_at.saturating_sub(100_000);
        let preview = f.run(|s, v, _| s.upload_clock_preview("me", proposed, v));
        f.run(|s, v, _| {
            assert!(s
                .reanchor_upload_clock("me", proposed + 1, &preview.confirmation_digest, v)
                .is_err());
            let host = entity_id_from_hex(&preview.host_id)?;
            let uid = entity_id_from_hex(&preview.user_id)?;
            let before = HardStateStore::open(&s.paths.hard_database)?
                .adapter_clock(host.as_bytes(), uid.as_bytes())?;
            let repaired =
                s.reanchor_upload_clock("me", proposed, &preview.confirmation_digest, v)?;
            assert_eq!(repaired.resulting_floor, proposed);
            let hard = HardStateStore::open(&s.paths.hard_database)?;
            assert_eq!(
                hard.upload_clock(host.as_bytes(), uid.as_bytes())?
                    .unwrap()
                    .reject_issued_before,
                preview.reject_issued_before
            );
            assert_eq!(hard.adapter_clock(host.as_bytes(), uid.as_bytes())?, before);
            assert!(s
                .reanchor_upload_clock("me", proposed, &preview.confirmation_digest, v)
                .is_err());
            Ok(())
        });
    }
}
