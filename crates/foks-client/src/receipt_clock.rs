//! The process clock that adapter and upload receipt admission sample.
//!
//! Wall time is paired with a monotonic clock that keeps counting while the
//! computer sleeps: `CLOCK_BOOTTIME` on Linux and `CLOCK_MONOTONIC_RAW` on
//! macOS. `std::time::Instant` stops during sleep on both, so a sleep longer
//! than the admission skew would read as a wall-clock jump. Other platforms
//! fall back to `Instant` and report no boot identity. The boot identity is
//! derived from the kernel's boot identifier and, on Linux, the time
//! namespace (see [`receipt_boot_identity`]). Every sample in one
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

/// The identity of the boot clock this process reads, read once at process
/// start through the same readers as the local deadline clock domain: the
/// kernel's boot identifier and the time namespace. Without one, a restarted
/// process cannot tell whether the stored boot-clock anchor is comparable to
/// its own readings, and restarts fall back to the restart-gap rule.
fn boot_identity() -> Option<[u8; 16]> {
    receipt_boot_identity(
        foks_local_deadline::boot_id().ok()?,
        foks_local_deadline::time_namespace().ok()?,
    )
}

/// Domain-separation tag of [`receipt_boot_identity`].
const RECEIPT_BOOT_IDENTITY_DOMAIN: u64 = 0x5d2c_8e41_b7a3_f096;

/// The 16-byte boot identity a receipt clock anchor records: the first 16
/// bytes of `prefixed_hash(RECEIPT_BOOT_IDENTITY_DOMAIN, boot_id || device ||
/// inode)`, with the namespace device and inode as big-endian u64s. A time
/// namespace offsets `CLOCK_BOOTTIME`, so a process in another namespace of
/// the same boot reads a different clock and must not count as the same boot;
/// `(0, 0)` stands for the single clock of a kernel without time namespaces,
/// and for macOS, which has none. The column stays 16 bytes, as schema 43
/// defines it. An all-zero kernel identifier is treated as absent.
fn receipt_boot_identity(boot: [u8; 16], (device, inode): (u64, u64)) -> Option<[u8; 16]> {
    if boot == [0; 16] {
        return None;
    }
    let mut input = [0; 32];
    input[..16].copy_from_slice(&boot);
    input[16..24].copy_from_slice(&device.to_be_bytes());
    input[24..].copy_from_slice(&inode.to_be_bytes());
    let digest = foks_crypto::prefixed_hash(RECEIPT_BOOT_IDENTITY_DOMAIN, &input);
    let mut identity = [0; 16];
    identity.copy_from_slice(&digest[..16]);
    Some(identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boot_identity_covers_the_boot_and_the_time_namespace() {
        let boot = [7; 16];
        let host = receipt_boot_identity(boot, (4, 4_026_531_834)).unwrap();
        // Deterministic, and never the kernel identifier itself.
        assert_eq!(receipt_boot_identity(boot, (4, 4_026_531_834)), Some(host));
        assert_ne!(host, boot);
        // Another time namespace of the same boot is another clock.
        for namespace in [(4, 4_026_532_001), (5, 4_026_531_834), (0, 0)] {
            assert_ne!(receipt_boot_identity(boot, namespace), Some(host));
        }
        // Another boot in the same namespace is another clock.
        assert_ne!(
            receipt_boot_identity([8; 16], (4, 4_026_531_834)),
            Some(host)
        );
        // Device and inode are not interchangeable.
        assert_ne!(
            receipt_boot_identity(boot, (1, 2)),
            receipt_boot_identity(boot, (2, 1))
        );
        assert_eq!(receipt_boot_identity([0; 16], (0, 0)), None);
        let digest = foks_crypto::prefixed_hash(RECEIPT_BOOT_IDENTITY_DOMAIN, &{
            let mut input = [0; 32];
            input[..16].copy_from_slice(&boot);
            input[16..24].copy_from_slice(&4u64.to_be_bytes());
            input[24..].copy_from_slice(&4_026_531_834u64.to_be_bytes());
            input
        });
        assert_eq!(host[..], digest[..16]);
    }

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
            assert_eq!(
                first.boot_id,
                receipt_boot_identity(
                    foks_local_deadline::boot_id().unwrap(),
                    foks_local_deadline::time_namespace().unwrap()
                )
            );
        }
    }
}
