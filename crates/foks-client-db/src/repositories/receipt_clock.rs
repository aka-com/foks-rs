//! Shared validated clock arithmetic; each receipt domain owns separate SQL state.
use crate::{Error, Result};
pub const MAX_NEW_SUBMISSION_AGE_SECONDS: u64 = 24 * 60 * 60;
pub const TERMINAL_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
pub(crate) const SKEW_SECONDS: u64 = 5 * 60;
const RESTART_GAP_SECONDS: u64 = 24 * 60 * 60;

/// Supplied by the application clock. The random process identity changes at
/// every process start; monotonic time is measured from that process's anchor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterTimeSample {
    pub process_id: [u8; 16],
    pub wall_seconds: u64,
    pub monotonic_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterClockState {
    pub process_id: [u8; 16],
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

pub(crate) fn validate(
    old: Option<&AdapterClockState>,
    sample: AdapterTimeSample,
) -> Result<AdapterClockState> {
    sample_in_range(sample)?;
    let Some(old) = old else {
        return reanchor(None, sample);
    };
    if sample.process_id == old.process_id {
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
        let mut next = old.clone();
        next.validated_time_floor = old.validated_time_floor.max(sample.wall_seconds);
        Ok(next)
    } else {
        if sample.wall_seconds > old.validated_time_floor.saturating_add(RESTART_GAP_SECONDS)
            || sample.wall_seconds.saturating_add(SKEW_SECONDS) < old.validated_time_floor
        {
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
        anchor_wall: sample.wall_seconds,
        anchor_monotonic: sample.monotonic_seconds,
        validated_time_floor: sample.wall_seconds,
        reject_issued_before: old.map_or(0, |old| old.reject_issued_before),
    })
}
