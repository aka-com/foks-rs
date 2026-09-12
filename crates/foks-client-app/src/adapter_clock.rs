//! Narrow injectable wall/monotonic clock for adapter admission and retention.
use crate::{Error, Result};
use foks_client_db::AdapterTimeSample;
use std::{
    sync::OnceLock,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub trait AdapterClock: Send + Sync {
    fn sample(&self) -> Result<AdapterTimeSample>;
}

#[derive(Default)]
pub struct SystemAdapterClock;
impl AdapterClock for SystemAdapterClock {
    fn sample(&self) -> Result<AdapterTimeSample> {
        static PROCESS: OnceLock<Option<([u8; 16], Instant)>> = OnceLock::new();
        let (process_id, started) = PROCESS
            .get_or_init(|| {
                let mut id = [0; 16];
                getrandom::fill(&mut id).ok()?;
                Some((id, Instant::now()))
            })
            .as_ref()
            .ok_or(Error::Randomness)?;
        let wall_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::ClientDatabase(foks_client_db::Error::AdapterClockUntrusted))?
            .as_secs();
        Ok(AdapterTimeSample {
            process_id: *process_id,
            wall_seconds,
            monotonic_seconds: started.elapsed().as_secs(),
        })
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AdapterClockPreview {
    pub host_id: String,
    pub user_id: String,
    pub anchor_wall: u64,
    pub admission_floor: u64,
    pub reject_issued_before: u64,
    pub proposed_wall: u64,
    pub confirmation_digest: String,
}

#[derive(Debug, serde::Serialize)]
pub struct AdapterClockRepair {
    pub preview: AdapterClockPreview,
    pub resulting_floor: u64,
    pub checkpoint_digest: String,
}

impl crate::CheckedProfileSession<'_> {
    pub fn adapter_clock_preview(
        &self,
        alias: &str,
        proposed: u64,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<AdapterClockPreview> {
        let (host_id, user_id) = self.data_identity(alias, vault)?;
        let host = crate::entity_id_from_hex(&host_id)?;
        let user = crate::entity_id_from_hex(&user_id)?;
        let state = foks_client_db::HardStateStore::open(&self.paths.hard_database)?
            .adapter_clock(host.as_bytes(), user.as_bytes())?
            .ok_or(Error::InvalidAccount("adapter clock has not been anchored"))?;
        let mut binding = Vec::new();
        binding.extend_from_slice(host.as_bytes());
        binding.extend_from_slice(user.as_bytes());
        binding.extend_from_slice(&state.process_id);
        for value in [
            state.anchor_wall,
            state.anchor_monotonic,
            state.admission_floor,
            state.reject_issued_before,
            proposed,
        ] {
            binding.extend_from_slice(&value.to_be_bytes());
        }
        Ok(AdapterClockPreview {
            host_id,
            user_id,
            anchor_wall: state.anchor_wall,
            admission_floor: state.admission_floor,
            reject_issued_before: state.reject_issued_before,
            proposed_wall: proposed,
            confirmation_digest: crate::hex(&foks_crypto::prefixed_hash(
                0x136a_9e87_018e_946c,
                &binding,
            )),
        })
    }

    /// Only the local CLI exposes this operation. The caller publishes the
    /// returned audit result after the checked-session boundary has checkpointed.
    pub fn reanchor_adapter_clock(
        &self,
        alias: &str,
        proposed: u64,
        confirmation: &str,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<AdapterClockRepair> {
        let preview = self.adapter_clock_preview(alias, proposed, vault)?;
        if confirmation != preview.confirmation_digest {
            return Err(foks_client_db::Error::AdapterIdentityConflict.into());
        }
        let host = crate::entity_id_from_hex(&preview.host_id)?;
        let user = crate::entity_id_from_hex(&preview.user_id)?;
        let mut hard = foks_client_db::HardStateStore::open(&self.paths.hard_database)?;
        let old = hard
            .adapter_clock(host.as_bytes(), user.as_bytes())?
            .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
        let sample = self.adapter_clock.sample()?;
        let updated = hard.reanchor_adapter_clock(
            host.as_bytes(),
            user.as_bytes(),
            &old,
            AdapterTimeSample {
                wall_seconds: proposed,
                ..sample
            },
        )?;
        Ok(AdapterClockRepair {
            preview,
            resulting_floor: updated.admission_floor,
            checkpoint_digest: crate::hex(&self.rollback_checkpoint()?.digest()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    #[test]
    fn repair_requires_current_preview_and_cannot_reverse_checkpointed_expiry() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, v, m| {
            s.create_account(
                "owner",
                "clockowner",
                "device",
                "owner@example.test",
                "",
                None,
                v,
                m,
            )?;
            Ok(())
        });
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let stale = SubmissionHandle::new(now - 86_401, [7; 16]);
        let before = f.run(|s, v, _| {
            let (host, user) = s.data_identity("owner", v)?;
            let mut hard = foks_client_db::HardStateStore::open(&s.paths.hard_database)?;
            hard.check_adapter_admission(
                entity_id_from_hex(&host)?.as_bytes(),
                entity_id_from_hex(&user)?.as_bytes(),
                SubmissionHandle::new(now, [8; 16]),
                s.adapter_clock.sample()?,
            )?;
            s.rollback_checkpoint()
        });
        let expired = f.run(|s, v, m| {
            assert_eq!(
                s.data_write_status("owner", stale, v, m)?.status,
                DataWriteStatus::Expired
            );
            s.rollback_checkpoint()
        });
        assert!(before.verify_descends_from(&expired).is_err());
        let preview = f.run(|s, v, _| s.adapter_clock_preview("owner", now - 100_000, v));
        f.run(|s, v, _| {
            assert!(s
                .reanchor_adapter_clock("owner", now - 100_001, &preview.confirmation_digest, v)
                .is_err());
            Ok(())
        });
        let repaired = f.run(|s, v, _| {
            s.reanchor_adapter_clock("owner", now - 100_000, &preview.confirmation_digest, v)
        });
        assert_eq!(
            repaired.preview.reject_issued_before,
            preview.reject_issued_before
        );
        assert_eq!(repaired.resulting_floor, now - 100_000);
        f.run(|s, v, m| {
            assert_eq!(
                s.data_write_status("owner", stale, v, m)?.status,
                DataWriteStatus::Expired
            );
            assert!(s
                .reanchor_adapter_clock("owner", now - 100_000, &preview.confirmation_digest, v)
                .is_err());
            assert_eq!(
                repaired.checkpoint_digest,
                hex(&s.rollback_checkpoint()?.digest()?)
            );
            assert!(expired
                .verify_descends_from(&s.rollback_checkpoint()?)
                .is_err());
            Ok(())
        });
    }
}
