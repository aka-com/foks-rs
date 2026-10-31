//! The process clock that adapter and upload receipt admission sample.
//!
//! Wall time is paired with a monotonic clock that keeps counting while the
//! computer sleeps: `CLOCK_BOOTTIME` on Linux and `CLOCK_MONOTONIC_RAW` on
//! macOS. `std::time::Instant` stops during sleep on both, so a sleep longer
//! than the admission skew would read as a wall-clock jump. Other platforms
//! fall back to `Instant` and report no boot identity. Every sample in one
//! process carries the same random process identity, whichever receipt domain
//! takes it.
use foks_client_db::AdapterTimeSample;
use std::{
    sync::OnceLock,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Why [`receipt_time_sample`] produced no sample. Each receipt domain maps
/// these to its own error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptClockError {
    /// The process identity could not be drawn.
    Randomness,
    /// The wall clock precedes the Unix epoch, or the boot clock chosen at
    /// process start can no longer be read.
    Untrusted,
}

/// Samples the process clock. The first call chooses the monotonic clock and
/// reads the boot identity; later calls in the process reuse both.
pub fn receipt_time_sample() -> std::result::Result<AdapterTimeSample, ReceiptClockError> {
    static PROCESS: OnceLock<Option<ProcessClock>> = OnceLock::new();
    let process = PROCESS
        .get_or_init(ProcessClock::start)
        .as_ref()
        .ok_or(ReceiptClockError::Randomness)?;
    let wall_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ReceiptClockError::Untrusted)?
        .as_secs();
    let monotonic_seconds = match process.monotonic {
        Monotonic::Boot => boot_clock_seconds().ok_or(ReceiptClockError::Untrusted)?,
        Monotonic::Process(started) => started.elapsed().as_secs(),
    };
    Ok(AdapterTimeSample {
        process_id: process.id,
        boot_id: process.boot_id,
        wall_seconds,
        monotonic_seconds,
    })
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

/// The kernel's identifier for this boot, read once at process start through
/// the same reader as the local deadline clock domain. Without one, a
/// restarted process cannot tell whether the stored boot-clock anchor is
/// comparable to its own readings, and restarts fall back to the restart-gap
/// rule. An all-zero identifier is treated as absent.
fn boot_identity() -> Option<[u8; 16]> {
    foks_local_deadline::boot_id()
        .ok()
        .filter(|id| *id != [0; 16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_suspend_inclusive_clock_per_process() {
        let first = receipt_time_sample().unwrap();
        let second = receipt_time_sample().unwrap();
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
}
