//! Shared validated clock arithmetic for the adapter and upload receipt
//! domains; each domain owns separate SQL state (kv_adapter_clocks and
//! kv_upload_clocks) and maps untrusted samples to its own error.
use crate::{Error, Result};

pub const MAX_NEW_SUBMISSION_AGE_SECONDS: u64 = 24 * 60 * 60;
pub const TERMINAL_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
/// How far wall time may differ from the anchor plus monotonic elapsed time.
pub const SKEW_SECONDS: u64 = 5 * 60;
const RESTART_GAP_SECONDS: u64 = 24 * 60 * 60;

/// Supplied by the application clock. The random process identity changes at
/// every process start. Monotonic time is comparable only within one process,
/// or across processes when both samples carry the same boot identity: the
/// application clock then reads a clock that counts from boot and includes
/// suspended time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterTimeSample {
    pub process_id: [u8; 16],
    pub boot_id: Option<[u8; 16]>,
    pub wall_seconds: u64,
    pub monotonic_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterClockState {
    pub process_id: [u8; 16],
    pub boot_id: Option<[u8; 16]>,
    pub anchor_wall: u64,
    pub anchor_monotonic: u64,
    pub validated_time_floor: u64,
    pub reject_issued_before: u64,
}

fn sample_in_range(sample: AdapterTimeSample) -> Result<()> {
    if sample.wall_seconds > i64::MAX as u64 - TERMINAL_RETENTION_SECONDS - SKEW_SECONDS
        || sample.monotonic_seconds > i64::MAX as u64
    {
        return Err(Error::AdapterClockUntrusted);
    }
    Ok(())
}

/// Wall time must match the anchor plus monotonic elapsed time within skew.
fn check_elapsed(old: &AdapterClockState, sample: AdapterTimeSample) -> Result<()> {
    let elapsed = sample
        .monotonic_seconds
        .checked_sub(old.anchor_monotonic)
        .ok_or(Error::AdapterClockUntrusted)?;
    let expected = old
        .anchor_wall
        .checked_add(elapsed)
        .ok_or(Error::AdapterClockUntrusted)?;
    if sample.wall_seconds.abs_diff(expected) > SKEW_SECONDS {
        return Err(Error::AdapterClockUntrusted);
    }
    Ok(())
}

pub(crate) fn validate(
    old: Option<&AdapterClockState>,
    sample: AdapterTimeSample,
) -> Result<AdapterClockState> {
    sample_in_range(sample)?;
    let Some(old) = old else {
        return reanchor(None, sample);
    };
    if sample.process_id == old.process_id {
        check_elapsed(old, sample)?;
        let mut next = old.clone();
        next.validated_time_floor = old.validated_time_floor.max(sample.wall_seconds);
        Ok(next)
    } else {
        // A restart within one boot reads the same boot clock, so elapsed time
        // since the stored anchor is still measured, however long the gap. The
        // restart-gap rule still applies otherwise, so a wall clock stepped
        // within one boot (NTP's first sync, a restored VM) recovers on restart
        // as before. The new process then anchors at its own validated sample.
        let same_boot = sample.boot_id.is_some()
            && sample.boot_id == old.boot_id
            && check_elapsed(old, sample).is_ok();
        if !same_boot
            && sample.wall_seconds > old.validated_time_floor.saturating_add(RESTART_GAP_SECONDS)
        {
            return Err(Error::AdapterClockUntrusted);
        }
        if sample.wall_seconds.saturating_add(SKEW_SECONDS) < old.validated_time_floor {
            return Err(Error::AdapterClockUntrusted);
        }
        let mut next = reanchor(Some(old), sample)?;
        next.validated_time_floor = old.validated_time_floor.max(sample.wall_seconds);
        Ok(next)
    }
}

pub(crate) fn reanchor(
    old: Option<&AdapterClockState>,
    sample: AdapterTimeSample,
) -> Result<AdapterClockState> {
    sample_in_range(sample)?;
    Ok(AdapterClockState {
        process_id: sample.process_id,
        boot_id: sample.boot_id,
        anchor_wall: sample.wall_seconds,
        anchor_monotonic: sample.monotonic_seconds,
        validated_time_floor: sample.wall_seconds,
        reject_issued_before: old.map_or(0, |old| old.reject_issued_before),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALL: u64 = 1_700_000_000;

    fn sample(process: u8, boot: Option<u8>, wall: u64, monotonic: u64) -> AdapterTimeSample {
        AdapterTimeSample {
            process_id: [process; 16],
            boot_id: boot.map(|boot| [boot; 16]),
            wall_seconds: wall,
            monotonic_seconds: monotonic,
        }
    }

    fn anchored(boot: Option<u8>) -> AdapterClockState {
        validate(None, sample(1, boot, WALL, 500)).unwrap()
    }

    #[test]
    fn sleep_is_accepted_when_the_monotonic_clock_includes_it() {
        let old = anchored(Some(1));
        let next = validate(Some(&old), sample(1, Some(1), WALL + 3_600, 4_100)).unwrap();
        assert_eq!(next.anchor_wall, WALL);
        assert_eq!(next.validated_time_floor, WALL + 3_600);
    }

    #[test]
    fn wall_jumps_and_backward_clocks_are_rejected_in_one_process() {
        let old = anchored(Some(1));
        for sample in [
            // Wall time advanced an hour while the monotonic clock did not.
            sample(1, Some(1), WALL + 3_600, 500),
            sample(1, Some(1), WALL - 3_600, 500),
            sample(1, Some(1), WALL, 499),
            sample(1, Some(1), WALL + SKEW_SECONDS + 1, 500),
        ] {
            assert!(matches!(
                validate(Some(&old), sample),
                Err(Error::AdapterClockUntrusted)
            ));
        }
        assert!(validate(Some(&old), sample(1, Some(1), WALL + SKEW_SECONDS, 500)).is_ok());
    }

    #[test]
    fn restart_in_the_same_boot_checks_elapsed_time_instead_of_the_gap() {
        let old = anchored(Some(1));
        let week = 7 * 86_400;
        let next = validate(Some(&old), sample(2, Some(1), WALL + week, 500 + week)).unwrap();
        assert_eq!(next.process_id, [2; 16]);
        assert_eq!(next.boot_id, Some([1; 16]));
        assert_eq!(
            (next.anchor_wall, next.anchor_monotonic),
            (WALL + week, 500 + week)
        );
        // Beyond the restart gap, the wall clock must agree with the boot clock.
        let late = WALL + RESTART_GAP_SECONDS + 3_600;
        for sample in [
            sample(2, Some(1), late, 500),
            sample(2, Some(1), late, 500 + week),
            sample(2, Some(1), WALL - 3_600, 500 + 7_200),
        ] {
            assert!(matches!(
                validate(Some(&old), sample),
                Err(Error::AdapterClockUntrusted)
            ));
        }
        // Within the gap, a wall clock stepped since the anchor recovers on
        // restart, as it did before boot identities.
        let stepped = validate(Some(&old), sample(2, Some(1), WALL + 3_600, 560)).unwrap();
        assert_eq!(stepped.anchor_wall, WALL + 3_600);
    }

    #[test]
    fn restart_without_a_shared_boot_keeps_the_gap_rule() {
        for (old, boot) in [
            (anchored(Some(1)), Some(2)),
            (anchored(Some(1)), None),
            (anchored(None), None),
            (anchored(None), Some(1)),
        ] {
            let gap = WALL + RESTART_GAP_SECONDS;
            let next = validate(Some(&old), sample(2, boot, gap, 0)).unwrap();
            assert_eq!(next.boot_id, boot.map(|boot| [boot; 16]));
            assert!(matches!(
                validate(Some(&old), sample(2, boot, gap + 1, 0)),
                Err(Error::AdapterClockUntrusted)
            ));
            assert!(matches!(
                validate(Some(&old), sample(2, boot, WALL - SKEW_SECONDS - 1, 0)),
                Err(Error::AdapterClockUntrusted)
            ));
        }
    }
}
