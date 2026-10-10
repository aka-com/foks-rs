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

/// Pairs wall time with a monotonic clock that keeps counting while the
/// computer sleeps: `CLOCK_BOOTTIME` on Linux and `CLOCK_MONOTONIC_RAW` on macOS.
/// `std::time::Instant` stops during sleep on both, so a sleep longer than the
/// admission skew would read as a wall-clock jump. Other platforms fall back to
/// `Instant` and report no boot identity.
#[derive(Default)]
pub struct SystemAdapterClock;
impl AdapterClock for SystemAdapterClock {
    fn sample(&self) -> Result<AdapterTimeSample> {
        static PROCESS: OnceLock<Option<ProcessClock>> = OnceLock::new();
        let process = PROCESS
            .get_or_init(ProcessClock::start)
            .as_ref()
            .ok_or(Error::Randomness)?;
        let wall_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| untrusted())?
            .as_secs();
        let monotonic_seconds = match process.monotonic {
            Monotonic::Boot => boot_clock_seconds().ok_or_else(untrusted)?,
            Monotonic::Process(started) => started.elapsed().as_secs(),
        };
        Ok(AdapterTimeSample {
            process_id: process.id,
            boot_id: process.boot_id,
            wall_seconds,
            monotonic_seconds,
        })
    }
}

fn untrusted() -> Error {
    Error::ClientDatabase(foks_client_db::Error::AdapterClockUntrusted)
}

struct ProcessClock {
    id: [u8; 16],
    /// Present only with `Monotonic::Boot`, whose readings other processes in
    /// the same boot can compare.
    boot_id: Option<[u8; 16]>,
    monotonic: Monotonic,
}

/// Chosen once per process so that every sample in the process reads the
/// same clock.
enum Monotonic {
    Boot,
    Process(Instant),
}

impl ProcessClock {
    fn start() -> Option<Self> {
        let mut id = [0; 16];
        getrandom::fill(&mut id).ok()?;
        let (boot_id, monotonic) = match boot_clock_seconds() {
            Some(_) => (boot_identity(), Monotonic::Boot),
            None => (None, Monotonic::Process(Instant::now())),
        };
        Some(Self {
            id,
            boot_id,
            monotonic,
        })
    }
}

/// Seconds since boot, including time spent suspended. As in the server's
/// administration clock, macOS uses `CLOCK_MONOTONIC_RAW`, which wall-clock
/// changes cannot step.
fn boot_clock_seconds() -> Option<u64> {
    #[cfg(target_os = "linux")]
    let id = nix::time::ClockId::CLOCK_BOOTTIME;
    #[cfg(target_os = "macos")]
    let id = nix::time::ClockId::from_raw(libc::CLOCK_MONOTONIC_RAW);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        u64::try_from(nix::time::clock_gettime(id).ok()?.tv_sec()).ok()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// A random identifier the kernel chooses at each boot. Without one, a
/// restarted process cannot tell whether the stored boot-clock anchor is
/// comparable to its own readings.
fn boot_identity() -> Option<[u8; 16]> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        parse_uuid(text.trim())
    }
    #[cfg(target_os = "macos")]
    {
        // This crate forbids unsafe code, which rules out calling sysctl(3)
        // directly; the process reads the value once at its first sample. If
        // the child cannot run, restarts fall back to the restart-gap rule.
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.bootsessionuuid"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_uuid(std::str::from_utf8(&output.stdout).ok()?.trim())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Parses the hyphenated 36-character UUID form; the result is never all zero.
fn parse_uuid(text: &str) -> Option<[u8; 16]> {
    if text.len() != 36
        || [8, 13, 18, 23]
            .iter()
            .any(|&index| text.as_bytes()[index] != b'-')
    {
        return None;
    }
    let digits: Vec<u8> = text.bytes().filter(|byte| *byte != b'-').collect();
    if digits.len() != 32 || !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut id = [0; 16];
    for (byte, pair) in id.iter_mut().zip(digits.chunks_exact(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    (id != [0; 16]).then_some(id)
}

#[derive(Debug, serde::Serialize)]
pub struct AdapterClockPreview {
    pub host_id: String,
    pub user_id: String,
    pub anchor_wall: u64,
    pub validated_time_floor: u64,
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
    fn boot_identities_parse_only_the_hyphenated_uuid_form() {
        assert_eq!(
            parse_uuid("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0"),
            Some([
                0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2,
                0xe1, 0xf0
            ])
        );
        assert_eq!(
            parse_uuid("0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0"),
            parse_uuid("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0")
        );
        for text in [
            "",
            "00000000-0000-0000-0000-000000000000",
            "0f1e2d3c4b5a69788796a5b4c3d2e1f0",
            "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f",
            "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1fg",
            "+f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0",
            "0f1e2d3c-4b5a-6978-8796a-5b4c3d2e1f0",
        ] {
            assert_eq!(parse_uuid(text), None, "{text}");
        }
    }

    #[test]
    fn system_clock_reads_one_suspend_inclusive_clock_per_process() {
        let first = SystemAdapterClock.sample().unwrap();
        let second = SystemAdapterClock.sample().unwrap();
        assert_eq!(first.process_id, second.process_id);
        assert_eq!(first.boot_id, second.boot_id);
        assert!(second.monotonic_seconds >= first.monotonic_seconds);
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
