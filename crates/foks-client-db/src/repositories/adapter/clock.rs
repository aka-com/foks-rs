use rusqlite::{params, Connection, OptionalExtension};

use super::{read_time, sql_time};
use crate::{Error, Result};

pub const ADMISSION_AGE_SECONDS: u64 = 24 * 60 * 60;
pub const TERMINAL_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
const SKEW_SECONDS: u64 = 5 * 60;
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
    pub admission_floor: u64,
    pub reject_issued_before: u64,
}

pub(super) fn read(
    connection: &Connection,
    host: &[u8],
    user: &[u8],
) -> Result<Option<AdapterClockState>> {
    Ok(connection
        .query_row(
            "SELECT process_id,anchor_wall,anchor_monotonic,admission_floor,reject_issued_before
         FROM kv_adapter_clocks WHERE host_id=?1 AND user_id=?2",
            params![host, user],
            |row| {
                Ok(AdapterClockState {
                    process_id: row.get(0)?,
                    anchor_wall: read_time(row, 1)?,
                    anchor_monotonic: read_time(row, 2)?,
                    admission_floor: read_time(row, 3)?,
                    reject_issued_before: read_time(row, 4)?,
                })
            },
        )
        .optional()?)
}

fn sample_in_range(sample: AdapterTimeSample) -> Result<()> {
    if sample.wall_seconds > i64::MAX as u64 - TERMINAL_RETENTION_SECONDS - SKEW_SECONDS
        || sample.monotonic_seconds > i64::MAX as u64
    {
        return Err(Error::AdapterClockUntrusted);
    }
    Ok(())
}

pub(super) fn validate(
    old: Option<&AdapterClockState>,
    sample: AdapterTimeSample,
) -> Result<AdapterClockState> {
    sample_in_range(sample)?;
    let Some(old) = old else {
        return Ok(reanchor(None, sample)?);
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
        next.admission_floor = old.admission_floor.max(sample.wall_seconds);
        Ok(next)
    } else {
        if sample.wall_seconds > old.admission_floor.saturating_add(RESTART_GAP_SECONDS)
            || sample.wall_seconds.saturating_add(SKEW_SECONDS) < old.admission_floor
        {
            return Err(Error::AdapterClockUntrusted);
        }
        let mut next = reanchor(Some(old), sample)?;
        next.admission_floor = old.admission_floor.max(sample.wall_seconds);
        Ok(next)
    }
}

pub(super) fn reanchor(
    old: Option<&AdapterClockState>,
    sample: AdapterTimeSample,
) -> Result<AdapterClockState> {
    sample_in_range(sample)?;
    Ok(AdapterClockState {
        process_id: sample.process_id,
        anchor_wall: sample.wall_seconds,
        anchor_monotonic: sample.monotonic_seconds,
        admission_floor: sample.wall_seconds,
        reject_issued_before: old.map_or(0, |old| old.reject_issued_before),
    })
}

pub(super) fn write(
    connection: &Connection,
    host: &[u8],
    user: &[u8],
    state: &AdapterClockState,
) -> Result<()> {
    if read(connection, host, user)?.as_ref() == Some(state) {
        return Ok(());
    }
    connection.execute(
        "INSERT INTO kv_adapter_clocks VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(host_id,user_id) DO UPDATE SET process_id=excluded.process_id,
         anchor_wall=excluded.anchor_wall,anchor_monotonic=excluded.anchor_monotonic,
         admission_floor=excluded.admission_floor,reject_issued_before=excluded.reject_issued_before",
        params![host,user,state.process_id,sql_time(state.anchor_wall)?,sql_time(state.anchor_monotonic)?,
            sql_time(state.admission_floor)?,sql_time(state.reject_issued_before)?],
    )?;
    Ok(())
}

pub(super) fn unseen_expired(
    handle: foks_proto::SubmissionHandle,
    state: &mut AdapterClockState,
) -> bool {
    let cutoff = state.admission_floor.saturating_sub(ADMISSION_AGE_SECONDS);
    if handle.issued_at() < cutoff {
        // The cutoff comes from validated time, never from caller-selected time.
        state.reject_issued_before = state.reject_issued_before.max(cutoff);
    }
    handle.issued_at() < state.reject_issued_before
}

pub(super) fn check_fresh(
    handle: foks_proto::SubmissionHandle,
    state: &AdapterClockState,
) -> Result<()> {
    if handle.issued_at() > state.admission_floor.saturating_add(SKEW_SECONDS) {
        return Err(Error::AdapterFutureHandle);
    }
    // A corrected clock behind the irreversible watermark cannot mint fresh
    // identities until the clock itself has caught up, even within future skew.
    if state.admission_floor < state.reject_issued_before {
        return Err(Error::AdapterClockUntrusted);
    }
    Ok(())
}
