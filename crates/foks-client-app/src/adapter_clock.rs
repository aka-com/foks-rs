//! Narrow injectable wall/monotonic clock for adapter admission and retention.
use crate::{Error, Result};
use foks_client_db::AdapterTimeSample;
use std::time::{SystemTime, UNIX_EPOCH};

pub trait AdapterClock: Send + Sync {
    fn sample(&self) -> Result<AdapterTimeSample>;
}

/// Samples [`foks_client::receipt_time_sample`], which pairs wall time with a
/// monotonic clock that keeps counting while the computer sleeps and records
/// the boot identity. The upload receipt domain samples the same clock.
#[derive(Default)]
pub struct SystemAdapterClock;
impl AdapterClock for SystemAdapterClock {
    fn sample(&self) -> Result<AdapterTimeSample> {
        // One process clock, shared with the upload receipt domain.
        foks_client::receipt_time_sample().map_err(|error| match error {
            foks_client::ReceiptClockError::Randomness => Error::Randomness,
            foks_client::ReceiptClockError::Untrusted => untrusted(),
        })
    }
}

fn untrusted() -> Error {
    Error::ClientDatabase(foks_client_db::Error::AdapterClockUntrusted)
}

pub use foks_agent_proto::data::{
    AdapterClockAdmission, AdapterClockPreview, AdapterClockRepair, AdapterClockStatus,
    ReceiptClockDomain,
};

/// How far a confirmed repair time may be from the wall time sampled when the
/// repair is written. The repair pairs the confirmed time with the monotonic
/// reading of that sample, so any difference is lost from the admission skew
/// that later writes may use; one minute leaves most of the five.
pub const ADAPTER_CLOCK_CONFIRM_SKEW_SECONDS: u64 = 60;

/// Domain-separation tag of a repair preview digest. The domains keep
/// distinct tags, so a digest previewed for one clock never confirms the
/// other.
fn preview_domain_tag(domain: ReceiptClockDomain) -> u64 {
    match domain {
        ReceiptClockDomain::Adapter => 0x136a_9e87_018e_946c,
        ReceiptClockDomain::Upload => 0x086a_9c32_a21e_7351,
    }
}

fn identity_conflict(domain: ReceiptClockDomain) -> Error {
    Error::ClientDatabase(match domain {
        ReceiptClockDomain::Adapter => foks_client_db::Error::AdapterIdentityConflict,
        ReceiptClockDomain::Upload => foks_client_db::Error::UploadIdentityConflict,
    })
}

fn read_clock(
    hard: &foks_client_db::HardStateStore,
    domain: ReceiptClockDomain,
    host: &[u8],
    user: &[u8],
) -> Result<Option<foks_client_db::AdapterClockState>> {
    Ok(match domain {
        ReceiptClockDomain::Adapter => hard.adapter_clock(host, user)?,
        ReceiptClockDomain::Upload => hard.upload_clock(host, user)?,
    })
}

impl crate::CheckedProfileSession<'_> {
    /// [`Self::receipt_clock_status`] for the adapter clock.
    pub fn adapter_clock_status(
        &self,
        alias: &str,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<AdapterClockStatus> {
        self.receipt_clock_status(ReceiptClockDomain::Adapter, alias, vault)
    }

    /// Reports whether the receipt clock in `domain` would admit new handles
    /// for `alias` now: MCP data writes for the adapter clock, file uploads
    /// for the upload clock. Reads only local state: it neither anchors nor
    /// advances the clock, and it takes the account identity from the pinned
    /// host and the stored credential, as [`Self::check_data_identity`] does,
    /// rather than authenticating to the server.
    pub fn receipt_clock_status(
        &self,
        domain: ReceiptClockDomain,
        alias: &str,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<AdapterClockStatus> {
        let user = vault.account(alias)?.credential.uid.clone();
        let host = self.pinned_host()?.host_id().clone();
        let hard = foks_client_db::HardStateStore::open(&self.paths.hard_database)?;
        let state = read_clock(&hard, domain, host.as_bytes(), user.as_bytes())?;
        let sample = self.adapter_clock.sample();
        let local_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| untrusted())?
            .as_secs();
        let admission = match (&state, sample) {
            (None, Ok(_)) => AdapterClockAdmission::Unanchored,
            (Some(_), Ok(sample)) => {
                let validated = match domain {
                    ReceiptClockDomain::Adapter => {
                        hard.validated_adapter_clock(host.as_bytes(), user.as_bytes(), sample)?
                    }
                    ReceiptClockDomain::Upload => {
                        hard.validated_upload_clock(host.as_bytes(), user.as_bytes(), sample)?
                    }
                };
                match validated {
                    None => AdapterClockAdmission::Paused,
                    Some(next) if next.validated_time_floor < next.reject_issued_before => {
                        AdapterClockAdmission::Behind
                    }
                    Some(_) => AdapterClockAdmission::Trusted,
                }
            }
            (_, Err(_)) => AdapterClockAdmission::Paused,
        };
        Ok(AdapterClockStatus {
            host_id: crate::hex(host.as_bytes()),
            user_id: crate::hex(user.as_bytes()),
            admission,
            local_time,
            validated_time_floor: state.as_ref().map(|state| state.validated_time_floor),
            reject_issued_before: state.as_ref().map(|state| state.reject_issued_before),
        })
    }

    /// [`Self::receipt_clock_preview`] for the adapter clock.
    pub fn adapter_clock_preview(
        &self,
        alias: &str,
        proposed: u64,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<AdapterClockPreview> {
        self.receipt_clock_preview(ReceiptClockDomain::Adapter, alias, proposed, vault)
    }

    pub fn receipt_clock_preview(
        &self,
        domain: ReceiptClockDomain,
        alias: &str,
        proposed: u64,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<AdapterClockPreview> {
        let (host_id, user_id) = self.data_identity(alias, vault)?;
        let host = crate::entity_id_from_hex(&host_id)?;
        let user = crate::entity_id_from_hex(&user_id)?;
        let hard = foks_client_db::HardStateStore::open(&self.paths.hard_database)?;
        let state = read_clock(&hard, domain, host.as_bytes(), user.as_bytes())?.ok_or(
            Error::InvalidAccount(match domain {
                ReceiptClockDomain::Adapter => "adapter clock has not been anchored",
                ReceiptClockDomain::Upload => "upload clock has not been anchored",
            }),
        )?;
        let mut binding = Vec::new();
        binding.extend_from_slice(host.as_bytes());
        binding.extend_from_slice(user.as_bytes());
        binding.extend_from_slice(&state.process_id);
        // The digest covers every field the repair's compare-and-swap checks.
        binding.push(u8::from(state.boot_id.is_some()));
        binding.extend_from_slice(&state.boot_id.unwrap_or_default());
        for value in [
            state.anchor_wall,
            state.anchor_monotonic,
            state.validated_time_floor,
            state.reject_issued_before,
            proposed,
        ] {
            binding.extend_from_slice(&value.to_be_bytes());
        }
        Ok(AdapterClockPreview {
            host_id,
            user_id,
            anchor_wall: state.anchor_wall,
            validated_time_floor: state.validated_time_floor,
            reject_issued_before: state.reject_issued_before,
            proposed_wall: proposed,
            confirmation_digest: crate::hex(&foks_crypto::prefixed_hash(
                preview_domain_tag(domain),
                &binding,
            )),
        })
    }

    /// [`Self::reanchor_receipt_clock`] for the adapter clock.
    pub fn reanchor_adapter_clock(
        &self,
        alias: &str,
        proposed: u64,
        confirmation: &str,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<AdapterClockRepair> {
        self.reanchor_receipt_clock(
            ReceiptClockDomain::Adapter,
            alias,
            proposed,
            confirmation,
            vault,
        )
    }

    /// The agent exposes this to the CLI and the desktop, never to MCP tools.
    /// The caller publishes the returned audit result after the checked-session
    /// boundary has checkpointed. `proposed` must be within
    /// [`ADAPTER_CLOCK_CONFIRM_SKEW_SECONDS`] of the wall time sampled here,
    /// after the identity check, so the time that check takes counts against
    /// the confirmation rather than the admission skew. The two domains are
    /// independent: repairing one leaves the other unchanged.
    pub fn reanchor_receipt_clock(
        &self,
        domain: ReceiptClockDomain,
        alias: &str,
        proposed: u64,
        confirmation: &str,
        vault: &mut crate::AccountVault<'_>,
    ) -> Result<AdapterClockRepair> {
        let preview = self.receipt_clock_preview(domain, alias, proposed, vault)?;
        if confirmation != preview.confirmation_digest {
            return Err(identity_conflict(domain));
        }
        let host = crate::entity_id_from_hex(&preview.host_id)?;
        let user = crate::entity_id_from_hex(&preview.user_id)?;
        let mut hard = foks_client_db::HardStateStore::open(&self.paths.hard_database)?;
        let old = read_clock(&hard, domain, host.as_bytes(), user.as_bytes())?
            .ok_or_else(|| identity_conflict(domain))?;
        let sample = self.adapter_clock.sample()?;
        if proposed.abs_diff(sample.wall_seconds) > ADAPTER_CLOCK_CONFIRM_SKEW_SECONDS {
            return Err(Error::AdapterClockConfirmationStale);
        }
        let sample = AdapterTimeSample {
            wall_seconds: proposed,
            ..sample
        };
        let updated = match domain {
            ReceiptClockDomain::Adapter => {
                hard.reanchor_adapter_clock(host.as_bytes(), user.as_bytes(), &old, sample)?
            }
            ReceiptClockDomain::Upload => {
                hard.reanchor_upload_clock(host.as_bytes(), user.as_bytes(), &old, sample)?
            }
        };
        Ok(AdapterClockRepair {
            preview,
            resulting_floor: updated.validated_time_floor,
            checkpoint_digest: crate::hex(&self.rollback_checkpoint()?.digest()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    #[test]
    fn system_clock_reads_one_suspend_inclusive_clock_per_process() {
        let first = SystemAdapterClock.sample().unwrap();
        let second = SystemAdapterClock.sample().unwrap();
        assert_eq!(first.process_id, second.process_id);
        assert_eq!(first.boot_id, second.boot_id);
        assert!(second.monotonic_seconds >= first.monotonic_seconds);
        // The upload receipt domain samples the same process clock.
        let upload = foks_client::upload_time_sample().unwrap();
        assert_eq!(
            (upload.process_id, upload.boot_id),
            (first.process_id, first.boot_id)
        );
        #[cfg(target_os = "linux")]
        {
            // /proc/uptime reads CLOCK_BOOTTIME; Instant would count from
            // process start instead.
            let uptime = std::fs::read_to_string("/proc/uptime").unwrap();
            let uptime: f64 = uptime.split(' ').next().unwrap().parse().unwrap();
            assert!(first.monotonic_seconds.abs_diff(uptime as u64) <= 2);
            assert!(first.boot_id.is_some());
        }
    }
    #[test]
    fn status_reports_a_paused_clock_until_repair_confirms_the_current_time() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, v, m| {
            s.create_account(
                "owner",
                "clockstatus",
                "device",
                "owner@example.test",
                "",
                None,
                v,
                m,
            )?;
            Ok(())
        });
        let admission = |f: &crate::test_support::AccountFixture| {
            f.run(|s, v, _| Ok(s.adapter_clock_status("owner", v)?.admission))
        };
        assert_eq!(admission(&f), AdapterClockAdmission::Unanchored);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        f.run(|s, v, _| {
            let (host, user) = s.data_identity("owner", v)?;
            let (host, user) = (entity_id_from_hex(&host)?, entity_id_from_hex(&user)?);
            let mut hard = foks_client_db::HardStateStore::open(&s.paths.hard_database)?;
            hard.check_adapter_admission(
                host.as_bytes(),
                user.as_bytes(),
                SubmissionHandle::new(now, [8; 16]),
                s.adapter_clock.sample()?,
            )?;
            Ok(())
        });
        assert_eq!(admission(&f), AdapterClockAdmission::Trusted);
        // An anchor an hour behind this process's own monotonic reading is
        // what a wall-clock jump within one process leaves behind.
        f.run(|s, v, _| {
            let (host, user) = s.data_identity("owner", v)?;
            let (host, user) = (entity_id_from_hex(&host)?, entity_id_from_hex(&user)?);
            let mut hard = foks_client_db::HardStateStore::open(&s.paths.hard_database)?;
            let old = hard
                .adapter_clock(host.as_bytes(), user.as_bytes())?
                .unwrap();
            hard.reanchor_adapter_clock(
                host.as_bytes(),
                user.as_bytes(),
                &old,
                AdapterTimeSample {
                    wall_seconds: now - 3_600,
                    ..s.adapter_clock.sample()?
                },
            )?;
            Ok(())
        });
        let paused = f.run(|s, v, _| s.adapter_clock_status("owner", v));
        assert_eq!(paused.admission, AdapterClockAdmission::Paused);
        assert_eq!(paused.validated_time_floor, Some(now - 3_600));
        // The status names the account from local state alone, and it is the
        // identity the authenticated scope reports.
        let identity = f.run(|s, v, _| s.data_identity("owner", v));
        assert_eq!((paused.host_id, paused.user_id), identity);
        let preview = f.run(|s, v, _| s.adapter_clock_preview("owner", now, v));
        f.run(|s, v, _| s.reanchor_adapter_clock("owner", now, &preview.confirmation_digest, v));
        assert_eq!(admission(&f), AdapterClockAdmission::Trusted);
        // A clock that ran two days ahead expired an unseen handle and left a
        // watermark a day past the real time. Confirming the current time
        // restores trust, but new handles wait for the watermark.
        f.run(|s, v, _| {
            let (host, user) = s.data_identity("owner", v)?;
            let (host, user) = (entity_id_from_hex(&host)?, entity_id_from_hex(&user)?);
            let mut hard = foks_client_db::HardStateStore::open(&s.paths.hard_database)?;
            let old = hard
                .adapter_clock(host.as_bytes(), user.as_bytes())?
                .unwrap();
            let ahead = AdapterTimeSample {
                wall_seconds: now + 172_800,
                ..s.adapter_clock.sample()?
            };
            hard.reanchor_adapter_clock(host.as_bytes(), user.as_bytes(), &old, ahead)?;
            assert!(matches!(
                hard.check_adapter_admission(
                    host.as_bytes(),
                    user.as_bytes(),
                    SubmissionHandle::new(now, [9; 16]),
                    ahead,
                ),
                Err(foks_client_db::Error::AdapterExpired)
            ));
            Ok(())
        });
        let preview = f.run(|s, v, _| s.adapter_clock_preview("owner", now, v));
        f.run(|s, v, _| s.reanchor_adapter_clock("owner", now, &preview.confirmation_digest, v));
        let behind = f.run(|s, v, _| s.adapter_clock_status("owner", v));
        assert_eq!(behind.admission, AdapterClockAdmission::Behind);
        assert_eq!(behind.reject_issued_before, Some(now + 86_400));
        // A write with a handle issued at the current time is below the
        // watermark, so it returns `expired` rather than `clock-untrusted`.
        let fresh = f.run(|s, v, m| {
            let issued = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            s.prepare_data_write(
                "owner",
                SubmissionHandle::new(issued, [10; 16]),
                DataWriteSpec {
                    kind: DataWriteKind::Mkdir,
                    path: "/behind".into(),
                    destination: None,
                    team_selector: None,
                    overwrite: false,
                    mkdir_p: false,
                    recursive: false,
                    body_length: 0,
                    body_hash: foks_crypto::kv_adapter_body_hash(&[]),
                },
                v,
                m,
            )
        });
        assert_eq!(fresh.status, DataWriteStatus::Expired);
        assert_eq!(admission(&f), AdapterClockAdmission::Behind);
    }

    /// The system clock with its wall time replaced, as a computer whose
    /// clock reads `wall` would sample it.
    struct WallClock(u64);
    impl AdapterClock for WallClock {
        fn sample(&self) -> Result<AdapterTimeSample> {
            Ok(AdapterTimeSample {
                wall_seconds: self.0,
                ..SystemAdapterClock.sample()?
            })
        }
    }

    #[test]
    fn repair_refuses_a_time_that_drifted_from_the_clock_it_writes() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, v, m| {
            s.create_account(
                "owner",
                "clockdrift",
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
        f.run(|s, v, _| {
            let (host, user) = s.data_identity("owner", v)?;
            let mut hard = foks_client_db::HardStateStore::open(&s.paths.hard_database)?;
            hard.check_adapter_admission(
                entity_id_from_hex(&host)?.as_bytes(),
                entity_id_from_hex(&user)?.as_bytes(),
                SubmissionHandle::new(now, [8; 16]),
                s.adapter_clock.sample()?,
            )?;
            Ok(())
        });
        // The preview digest matches, but the clock read when the repair is
        // written is more than a minute past the confirmed time.
        let preview = f.run(|s, v, _| s.adapter_clock_preview("owner", now, v));
        let refused = f.run_with_clock(std::sync::Arc::new(WallClock(now + 61)), |s, v, _| {
            Ok(s.reanchor_adapter_clock("owner", now, &preview.confirmation_digest, v))
        });
        assert!(matches!(refused, Err(Error::AdapterClockConfirmationStale)));
        let unchanged = f.run(|s, v, _| s.adapter_clock_preview("owner", now, v));
        assert_eq!(unchanged, preview);
        let repaired = f.run_with_clock(std::sync::Arc::new(WallClock(now + 60)), |s, v, _| {
            s.reanchor_adapter_clock("owner", now, &preview.confirmation_digest, v)
        });
        assert_eq!(repaired.resulting_floor, now);
    }

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
        // The repair runs on a computer whose clock reads the proposed time.
        let earlier = || std::sync::Arc::new(WallClock(now - 100_000));
        let preview = f.run(|s, v, _| s.adapter_clock_preview("owner", now - 100_000, v));
        f.run_with_clock(earlier(), |s, v, _| {
            assert!(s
                .reanchor_adapter_clock("owner", now - 100_001, &preview.confirmation_digest, v)
                .is_err());
            Ok(())
        });
        let repaired = f.run_with_clock(earlier(), |s, v, _| {
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

    #[test]
    fn upload_clock_repair_requires_preview_preserves_floor_and_adapter_domain() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, v, k| s.create_account("me", "uploadclock", "laptop", "", "", None, v, k));
        let context = f.run(|s, v, _| s.kv_upload_issuance("me", None, v));
        let status = f.run(|s, v, _| s.receipt_clock_status(ReceiptClockDomain::Upload, "me", v));
        assert_eq!(status.admission, AdapterClockAdmission::Trusted);
        let proposed = context.issued_at.saturating_sub(100_000);
        let preview =
            f.run(|s, v, _| s.receipt_clock_preview(ReceiptClockDomain::Upload, "me", proposed, v));
        // A digest previewed for one domain never confirms the other.
        let adapter = f.run(|s, v, _| {
            let (host, uid) = s.data_identity("me", v)?;
            let (host, uid) = (entity_id_from_hex(&host)?, entity_id_from_hex(&uid)?);
            let mut hard = HardStateStore::open(&s.paths.hard_database)?;
            hard.check_adapter_admission(
                host.as_bytes(),
                uid.as_bytes(),
                SubmissionHandle::new(context.issued_at, [8; 16]),
                s.adapter_clock.sample()?,
            )?;
            s.adapter_clock_preview("me", proposed, v)
        });
        assert_ne!(adapter.confirmation_digest, preview.confirmation_digest);
        let earlier = || std::sync::Arc::new(WallClock(proposed));
        f.run_with_clock(earlier(), |s, v, _| {
            assert!(s
                .reanchor_receipt_clock(
                    ReceiptClockDomain::Upload,
                    "me",
                    proposed + 1,
                    &preview.confirmation_digest,
                    v
                )
                .is_err());
            assert!(s
                .reanchor_adapter_clock("me", proposed, &preview.confirmation_digest, v)
                .is_err());
            let host = entity_id_from_hex(&preview.host_id)?;
            let uid = entity_id_from_hex(&preview.user_id)?;
            let before = HardStateStore::open(&s.paths.hard_database)?
                .adapter_clock(host.as_bytes(), uid.as_bytes())?;
            let repaired = s.reanchor_receipt_clock(
                ReceiptClockDomain::Upload,
                "me",
                proposed,
                &preview.confirmation_digest,
                v,
            )?;
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
                .reanchor_receipt_clock(
                    ReceiptClockDomain::Upload,
                    "me",
                    proposed,
                    &preview.confirmation_digest,
                    v
                )
                .is_err());
            Ok(())
        });
    }
}
