use rusqlite::{params, Connection, OptionalExtension};

use super::{read_time, sql_time};
use crate::{Error, Result};

pub(super) use crate::repositories::receipt_clock::{reanchor, validate};
pub use crate::repositories::receipt_clock::{
    AdapterClockState, AdapterTimeSample, MAX_NEW_SUBMISSION_AGE_SECONDS, SKEW_SECONDS,
    TERMINAL_RETENTION_SECONDS,
};

pub(super) fn read(
    connection: &Connection,
    host: &[u8],
    user: &[u8],
) -> Result<Option<AdapterClockState>> {
    Ok(connection
        .query_row(
            "SELECT process_id,anchor_wall,anchor_monotonic,validated_time_floor,reject_issued_before,boot_id
         FROM kv_adapter_clocks WHERE host_id=?1 AND user_id=?2",
            params![host, user],
            |row| {
                Ok(AdapterClockState {
                    process_id: row.get(0)?,
                    boot_id: row.get(5)?,
                    anchor_wall: read_time(row, 1)?,
                    anchor_monotonic: read_time(row, 2)?,
                    validated_time_floor: read_time(row, 3)?,
                    reject_issued_before: read_time(row, 4)?,
                })
            },
        )
        .optional()?)
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
        "INSERT INTO kv_adapter_clocks VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(host_id,user_id) DO UPDATE SET process_id=excluded.process_id,
         anchor_wall=excluded.anchor_wall,anchor_monotonic=excluded.anchor_monotonic,
         validated_time_floor=excluded.validated_time_floor,reject_issued_before=excluded.reject_issued_before,
         boot_id=excluded.boot_id",
        params![host,user,state.process_id,sql_time(state.anchor_wall)?,sql_time(state.anchor_monotonic)?,
            sql_time(state.validated_time_floor)?,sql_time(state.reject_issued_before)?,state.boot_id],
    )?;
    Ok(())
}

pub(super) fn unseen_expired(
    handle: foks_proto::SubmissionHandle,
    state: &mut AdapterClockState,
) -> bool {
    let cutoff = state
        .validated_time_floor
        .saturating_sub(MAX_NEW_SUBMISSION_AGE_SECONDS);
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
    if handle.issued_at() > state.validated_time_floor.saturating_add(SKEW_SECONDS) {
        return Err(Error::AdapterFutureHandle);
    }
    // A corrected clock behind the irreversible watermark cannot mint fresh
    // identities until the clock itself has caught up, even within future skew.
    if state.validated_time_floor < state.reject_issued_before {
        return Err(Error::AdapterClockUntrusted);
    }
    Ok(())
}
