//! Stable adapter replay ledger. Callers hold the checked-profile boundary and
//! publish the external rollback checkpoint even when admission returns an error.
use foks_proto::SubmissionHandle;
use rusqlite::{params, OptionalExtension};

use crate::*;

mod clock;
pub use clock::{
    AdapterClockState, AdapterTimeSample, ADMISSION_AGE_SECONDS, TERMINAL_RETENTION_SECONDS,
};

const HANDLE_HASH_DOMAIN: u64 = 0x14d8_6ced_ea89_0362;
const PRUNE_BATCH: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterLedgerState {
    Live,
    Committed,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterSubmission {
    pub handle: SubmissionHandle,
    pub host_id: Vec<u8>,
    pub user_id: Vec<u8>,
    pub team_id: Vec<u8>,
    pub input_hash: [u8; 32],
    pub internal_id: Option<[u8; 16]>,
    pub state: AdapterLedgerState,
    pub ancillary_committed: bool,
    pub node_id: Option<[u8; 17]>,
    pub terminal_at: Option<u64>,
    pub expires_at: Option<u64>,
}

pub fn adapter_handle_hash(handle: SubmissionHandle) -> [u8; 32] {
    foks_crypto::prefixed_hash(HANDLE_HASH_DOMAIN, handle.to_string().as_bytes())
}

fn read_time(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn read_optional_time(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
        })
        .transpose()
}

fn sql_time(value: u64) -> Result<i64> {
    sqlite_integer("adapter time", value)
}

fn lookup(
    connection: &rusqlite::Connection,
    handle: SubmissionHandle,
) -> Result<Option<AdapterSubmission>> {
    let stored = connection.query_row(
        "SELECT handle,host_id,user_id,team_id,input_hash,internal_id,state,ancillary_committed,node_id,terminal_at,expires_at
         FROM kv_adapter_submissions WHERE handle_hash=?1",
        [adapter_handle_hash(handle)], |r| {
            let state = match r.get::<_, u8>(6)? {
                0 => AdapterLedgerState::Live, 1 => AdapterLedgerState::Committed, 2 => AdapterLedgerState::Rejected,
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            Ok((r.get::<_, String>(0)?, AdapterSubmission {
                handle, host_id:r.get(1)?, user_id:r.get(2)?, team_id:r.get(3)?, input_hash:r.get(4)?,
                internal_id:r.get(5)?, state, ancillary_committed:r.get(7)?, node_id:r.get(8)?,
                terminal_at:read_optional_time(r,9)?, expires_at:read_optional_time(r,10)?,
            }))
        },
    ).optional()?;
    match stored {
        Some((canonical, row)) if canonical == handle.to_string() => Ok(Some(row)),
        Some(_) => Err(Error::AdapterIdentityConflict),
        None => Ok(None),
    }
}

impl HardStateStore {
    pub fn adapter_submission(
        &self,
        handle: SubmissionHandle,
    ) -> Result<Option<AdapterSubmission>> {
        lookup(&self.connection, handle)
    }

    /// Bounded ledger inventory: pending work or a terminal cleanup batch.
    pub fn adapter_submission_batch(
        &self,
        host: &[u8],
        user: &[u8],
        pending: bool,
    ) -> Result<Vec<AdapterSubmission>> {
        let mut stmt = self.connection.prepare(if pending {
            "SELECT handle FROM kv_adapter_submissions WHERE host_id=?1 AND user_id=?2 AND state=0 ORDER BY handle_hash LIMIT 65"
        } else {
            "SELECT handle FROM kv_adapter_submissions INDEXED BY kv_adapter_account_maintenance WHERE host_id=?1 AND user_id=?2 AND state IN (1,2) AND (internal_id IS NOT NULL OR terminal_at IS NULL) ORDER BY handle_hash LIMIT 8"
        })?;
        let handles = stmt
            .query_map(params![host, user], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        handles
            .into_iter()
            .map(|text| {
                let handle = text.parse().map_err(|_| Error::AdapterIdentityConflict)?;
                lookup(&self.connection, handle)?.ok_or(Error::AdapterIdentityConflict)
            })
            .collect()
    }

    pub fn next_adapter_maintenance(&self, after: &[u8]) -> Result<Option<AdapterSubmission>> {
        let handle: Option<String> = self
            .connection
            .query_row(
                "SELECT handle FROM kv_adapter_submissions INDEXED BY kv_adapter_maintenance
             WHERE (internal_id IS NOT NULL OR terminal_at IS NULL) AND handle_hash>?1
             ORDER BY handle_hash LIMIT 1",
                [after],
                |r| r.get(0),
            )
            .optional()?;
        handle
            .map(|text| {
                lookup(
                    &self.connection,
                    text.parse().map_err(|_| Error::AdapterIdentityConflict)?,
                )?
                .ok_or(Error::AdapterIdentityConflict)
            })
            .transpose()
    }

    pub fn next_adapter_clock_account(
        &self,
        after: Option<(&[u8], &[u8])>,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let (host, user) = after.unwrap_or((&[], &[]));
        Ok(self
            .connection
            .query_row(
                "SELECT host_id,user_id FROM kv_adapter_clocks
            WHERE (host_id,user_id)>(?1,?2) ORDER BY host_id,user_id LIMIT 1",
                params![host, user],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn adapter_clock(&self, host: &[u8], user: &[u8]) -> Result<Option<AdapterClockState>> {
        clock::read(&self.connection, host, user)
    }

    /// Validates unseen age, committing irreversible rejection before returning
    /// `AdapterExpired`. Existing rows always take precedence over age and clocks.
    pub fn check_adapter_admission(
        &mut self,
        host: &[u8],
        user: &[u8],
        handle: SubmissionHandle,
        sample: AdapterTimeSample,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        if let Some(row) = lookup(&tx, handle)? {
            if row.host_id != host || row.user_id != user {
                return Err(Error::AdapterIdentityConflict);
            }
            return Ok(());
        }
        let old = clock::read(&tx, host, user)?;
        if old
            .as_ref()
            .is_some_and(|c| handle.issued_at() < c.reject_issued_before)
        {
            return Err(Error::AdapterExpired);
        }
        let mut state = clock::validate(old.as_ref(), sample)?;
        let expired = clock::unseen_expired(handle, &mut state);
        clock::write(&tx, host, user, &state)?;
        tx.commit()?;
        if expired {
            return Err(Error::AdapterExpired);
        }
        clock::check_fresh(handle, &state)
    }

    /// Protected installation precedes this transaction. A total ledger slot is
    /// reserved here and survives terminal cleanup failure until physical pruning.
    pub fn record_adapter_submission(
        &mut self,
        handle: SubmissionHandle,
        operation: &MutationOperation,
        sample: AdapterTimeSample,
    ) -> Result<()> {
        if operation.kind != MutationKind::KvAdapter {
            return Err(Error::AdapterIdentityConflict);
        }
        let tx = self.write_transaction()?;
        if let Some(row) = lookup(&tx, handle)? {
            if row.host_id == operation.host_id
                && row.user_id == operation.scope_id
                && row.team_id == operation.subject_id
                && row.input_hash == operation.request_hash
                && row.internal_id == Some(operation.operation_id)
            {
                return Ok(());
            }
            return Err(Error::AdapterIdentityConflict);
        }
        let old = clock::read(&tx, &operation.host_id, &operation.scope_id)?;
        let mut state = clock::validate(old.as_ref(), sample)?;
        if clock::unseen_expired(handle, &mut state) {
            clock::write(&tx, &operation.host_id, &operation.scope_id, &state)?;
            tx.commit()?;
            return Err(Error::AdapterExpired);
        }
        clock::check_fresh(handle, &state)?;
        let (total, active): (i64, i64) = tx.query_row(
            "SELECT (SELECT count(*) FROM kv_adapter_submissions WHERE host_id=?1 AND user_id=?2),
                    (SELECT count(*) FROM kv_adapter_submissions WHERE host_id=?1 AND user_id=?2 AND state=0)",
            params![operation.host_id,operation.scope_id], |r| Ok((r.get(0)?,r.get(1)?)),
        )?;
        if total >= 65_536 {
            return Err(Error::AdapterRetentionFull);
        }
        if active >= 64 {
            return Err(Error::AdapterActiveFull);
        }
        clock::write(&tx, &operation.host_id, &operation.scope_id, &state)?;
        tx.execute(
            "INSERT INTO kv_adapter_submissions(handle_hash,handle,handle_version,schema_version,host_id,user_id,team_id,input_hash,internal_id,state,issued_at,created_at)
             VALUES (?1,?2,1,1,?3,?4,?5,?6,?7,0,?8,?9)",
            params![adapter_handle_hash(handle),handle.to_string(),operation.host_id,operation.scope_id,
                operation.subject_id,operation.request_hash,operation.operation_id,sql_time(handle.issued_at())?,sql_time(state.admission_floor)?],
        )?;
        super::journals::record_mutation_on(&tx, operation)?;
        tx.commit()?;
        Ok(())
    }

    /// Non-secret bounded proof is committed in the same transaction as terminal
    /// generic state, before any protected evidence is erased. An unavailable
    /// clock defers expiry without blocking evidence recording or recovery.
    pub fn finish_adapter_submission(
        &mut self,
        handle: SubmissionHandle,
        committed: bool,
        ancillary: bool,
        node_id: Option<[u8; 17]>,
        sample: Option<AdapterTimeSample>,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        let row = lookup(&tx, handle)?.ok_or(Error::AdapterIdentityConflict)?;
        let expected = if committed {
            AdapterLedgerState::Committed
        } else {
            AdapterLedgerState::Rejected
        };
        if row.state != AdapterLedgerState::Live && row.state != expected {
            return Err(Error::AdapterIdentityConflict);
        }
        if row.node_id.is_some() && node_id.is_some() && row.node_id != node_id {
            return Err(Error::AdapterIdentityConflict);
        }
        let old_clock = clock::read(&tx, &row.host_id, &row.user_id)?;
        let now = match sample
            .map(|sample| clock::validate(old_clock.as_ref(), sample))
            .transpose()
        {
            Ok(Some(state)) => {
                clock::write(&tx, &row.host_id, &row.user_id, &state)?;
                Some(state.admission_floor)
            }
            Ok(None) | Err(Error::AdapterClockUntrusted) => None,
            Err(error) => return Err(error),
        };
        if let Some(id) = row.internal_id {
            let current: u8 = tx.query_row(
                "SELECT state FROM mutation_operations WHERE operation_id=?1",
                [id],
                |r| r.get(0),
            )?;
            let allowed = if committed {
                matches!(current, 2 | 3 | 4 | 6)
            } else {
                matches!(current, 1 | 2 | 3 | 5)
            };
            if !allowed {
                return Err(Error::AdapterIdentityConflict);
            }
            tx.execute(
                "UPDATE mutation_operations SET state=?2 WHERE operation_id=?1",
                params![id, if committed { 6 } else { 5 }],
            )?;
        }
        let terminal_at = row.terminal_at.or(now);
        let expires_at = terminal_at
            .map(|time| {
                time.checked_add(TERMINAL_RETENTION_SECONDS)
                    .ok_or(Error::AdapterClockUntrusted)
            })
            .transpose()?;
        tx.execute(
            "UPDATE kv_adapter_submissions SET state=?2,ancillary_committed=max(ancillary_committed,?3),
             node_id=coalesce(node_id,?4),terminal_at=?5,expires_at=?6 WHERE handle_hash=?1",
            params![adapter_handle_hash(handle),if committed {1} else {2},ancillary,node_id,terminal_at.map(sql_time).transpose()?,expires_at.map(sql_time).transpose()?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Called by the owning KV verifier after authenticated projection commit and
    /// before protected erasure. Recovery may repeat it for RemoteVerified children.
    pub fn record_verified_adapter_child(
        &mut self,
        child: &[u8; 16],
        node: Option<[u8; 17]>,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        let owner: Option<([u8; 16], bool)> = tx
            .query_row(
                "SELECT parent_id,completion FROM mutation_children WHERE child_id=?1",
                [child],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((parent, completion)) = owner else {
            return Ok(());
        };
        let verified: bool = tx.query_row(
            "SELECT operation_kind IN (5,6,7) AND state IN (4,6)
            FROM mutation_operations WHERE operation_id=?1",
            [child],
            |r| r.get(0),
        )?;
        if !verified {
            return Err(Error::AdapterIdentityConflict);
        }
        let node = if completion { node } else { None };
        tx.execute(
            "UPDATE kv_adapter_submissions SET ancillary_committed=1,
            node_id=coalesce(?2,node_id) WHERE internal_id=?1",
            params![parent, node],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Only called after durable removal of all parent and terminal-child material
    /// under the checked-profile boundary. The transaction rechecks every owner.
    pub fn compact_adapter_submission(&mut self, handle: SubmissionHandle) -> Result<()> {
        let tx = self.write_transaction()?;
        let row = lookup(&tx, handle)?.ok_or(Error::AdapterIdentityConflict)?;
        if row.state == AdapterLedgerState::Live {
            return Err(Error::AdapterCleanupDeferred);
        }
        let Some(id) = row.internal_id else {
            return Ok(());
        };
        let blocked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM mutation_operations WHERE operation_id=?1 AND state NOT IN (5,6))
               OR EXISTS(SELECT 1 FROM mutation_children e JOIN mutation_operations m ON m.operation_id=e.child_id
                 WHERE e.parent_id=?1 AND (m.state NOT IN (5,6) OR m.operation_kind NOT IN (5,6,7)))
               OR EXISTS(SELECT 1 FROM mutation_children WHERE child_id=?1)",
            [id], |r| r.get(0),
        )?;
        if blocked {
            return Err(Error::AdapterCleanupDeferred);
        }
        let children = {
            let mut stmt =
                tx.prepare("SELECT child_id FROM mutation_children WHERE parent_id=?1 LIMIT 257")?;
            let values = stmt
                .query_map([id], |r| r.get::<_, [u8; 16]>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if values.len() > 256 {
                return Err(Error::AdapterCleanupDeferred);
            }
            values
        };
        tx.execute(
            "UPDATE kv_adapter_submissions SET internal_id=NULL WHERE handle_hash=?1",
            [adapter_handle_hash(handle)],
        )?;
        tx.execute("DELETE FROM mutation_children WHERE parent_id=?1", [id])?;
        for child in children {
            // child_id is globally unique and foreign keys prevent deleting any
            // row still referenced by an additional durable owner.
            tx.execute(
                "DELETE FROM mutation_operations WHERE operation_id=?1 AND state IN (5,6)",
                [child],
            )?;
        }
        tx.execute("DELETE FROM mutation_operations WHERE operation_id=?1 AND operation_kind=8 AND state IN (5,6)", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn prune_adapter_submissions(
        &mut self,
        host: &[u8],
        user: &[u8],
        sample: AdapterTimeSample,
    ) -> Result<usize> {
        let tx = self.write_transaction()?;
        let old = clock::read(&tx, host, user)?;
        let mut state = clock::validate(old.as_ref(), sample)?;
        let rows = {
            let mut stmt = tx.prepare("SELECT handle_hash,issued_at FROM kv_adapter_submissions
                WHERE host_id=?1 AND user_id=?2 AND internal_id IS NULL AND state IN (1,2) AND expires_at<=?3
                ORDER BY expires_at,handle_hash LIMIT ?4")?;
            let values = stmt
                .query_map(
                    params![
                        host,
                        user,
                        sql_time(state.admission_floor)?,
                        PRUNE_BATCH as i64
                    ],
                    |r| Ok((r.get::<_, [u8; 32]>(0)?, read_time(r, 1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            values
        };
        for (hash, issued) in &rows {
            state.reject_issued_before = state
                .reject_issued_before
                .max(issued.checked_add(1).ok_or(Error::AdapterClockUntrusted)?);
            tx.execute(
                "DELETE FROM kv_adapter_submissions WHERE handle_hash=?1",
                [hash],
            )?;
        }
        if !rows.is_empty() {
            clock::write(&tx, host, user, &state)?;
        }
        tx.commit()?;
        Ok(rows.len())
    }

    /// Local operator action: CAS the complete displayed clock authority, retain
    /// irreversible rejection, and leave admission/pruning to a later invocation.
    pub fn reanchor_adapter_clock(
        &mut self,
        host: &[u8],
        user: &[u8],
        expected: &AdapterClockState,
        sample: AdapterTimeSample,
    ) -> Result<AdapterClockState> {
        let tx = self.write_transaction()?;
        let old = clock::read(&tx, host, user)?;
        if old.as_ref() != Some(expected) {
            return Err(Error::AdapterIdentityConflict);
        }
        let state = clock::reanchor(old.as_ref(), sample)?;
        clock::write(&tx, host, user, &state)?;
        tx.commit()?;
        Ok(state)
    }
}
