//! Local upload identities and conservative receipt/cleanup ownership.
use super::*;
use crate::repositories::receipt_clock;
use foks_proto::UploadHandle;

pub type UploadTimeSample = AdapterTimeSample;
pub type UploadClockState = AdapterClockState;
pub const MAX_NEW_UPLOAD_AGE_SECONDS: u64 = 24 * 60 * 60;
pub const UPLOAD_TERMINAL_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
const RECEIPT_BYTES: u64 = 1024; // Logical metadata reservation, not SQLite file size.
pub(crate) const SCHEMA42: &str = include_str!("schema42.sql");
pub(crate) const TABLES42: &[&str] = &[
    "kv_upload_receipt_limits",
    "kv_upload_handles",
    "kv_upload_clocks",
    "kv_upload_cleanup",
    "kv_upload_children",
];
// Keep these exact production statements available to VM-cost regressions.
macro_rules! maintenance_summary_sql {
    ($tail:literal) => {
        concat!("SELECT h.handle,u.operation_id,u.host_id,u.uid,u.party_id,u.state,u.file_id,u.chunk_count,u.acknowledged_chunks,u.plaintext_bytes,u.reserved_bytes,u.receipt_version ", $tail)
    };
}
const CLEANUP_FIRST: &str = maintenance_summary_sql!("FROM kv_upload_cleanup c INDEXED BY kv_upload_cleanup_pending CROSS JOIN kv_uploads u ON u.operation_id=c.operation_id CROSS JOIN kv_upload_handles h ON h.operation_id=u.operation_id WHERE c.phase<4 AND u.state IN (5,6,9,11) ORDER BY c.operation_id LIMIT 1");
const CLEANUP_AFTER: &str = maintenance_summary_sql!("FROM kv_upload_cleanup c INDEXED BY kv_upload_cleanup_pending CROSS JOIN kv_uploads u ON u.operation_id=c.operation_id CROSS JOIN kv_upload_handles h ON h.operation_id=u.operation_id WHERE c.phase<4 AND c.operation_id>?1 AND u.state IN (5,6,9,11) ORDER BY c.operation_id LIMIT 1");
const UNSTAMPED_FIRST: &str = maintenance_summary_sql!("FROM kv_upload_handles h INDEXED BY kv_upload_handles_unstamped CROSS JOIN kv_uploads u ON u.operation_id=h.operation_id WHERE h.handle_version=1 AND h.terminal_at IS NULL AND u.state IN (6,9,11) ORDER BY h.operation_id LIMIT 1");
const UNSTAMPED_AFTER: &str = maintenance_summary_sql!("FROM kv_upload_handles h INDEXED BY kv_upload_handles_unstamped CROSS JOIN kv_uploads u ON u.operation_id=h.operation_id WHERE h.handle_version=1 AND h.terminal_at IS NULL AND h.operation_id>?1 AND u.state IN (6,9,11) ORDER BY h.operation_id LIMIT 1");
const CLOCK_FIRST: &str =
    "SELECT host_id,user_id FROM kv_upload_clocks ORDER BY host_id,user_id LIMIT 1";
const CLOCK_AFTER: &str = "SELECT host_id,user_id FROM kv_upload_clocks WHERE (host_id,user_id)>(?1,?2) ORDER BY host_id,user_id LIMIT 1";

// Account-local selection scans/sorts the configured V1 cap plus fixed migrated legacy receipts;
// it does not claim constant work as this account accumulates receipts.
const PRUNE_CANDIDATES: &str = "SELECT h.handle_hash,h.operation_id,h.issued_at FROM kv_uploads u INDEXED BY kv_uploads_by_owner CROSS JOIN kv_upload_handles h ON h.operation_id=u.operation_id CROSS JOIN kv_upload_cleanup c ON c.operation_id=u.operation_id WHERE u.host_id=?1 AND u.uid=?2 AND h.handle_version=1 AND h.expires_at<=?3 AND u.state IN (6,9,11) AND u.reserved_bytes=0 AND u.namespace_child IS NULL AND u.adapter_parent IS NULL AND c.phase=4 AND NOT EXISTS(SELECT 1 FROM kv_upload_children ch WHERE ch.upload_id=u.operation_id) ORDER BY h.expires_at,h.handle_hash LIMIT ?4";

fn maintenance_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<UploadSummary> {
    Ok(UploadSummary {
        handle: handle_from_sql(row.get(0)?)?,
        internal_id: row.get(1)?,
        host_id: row.get(2)?,
        uid: row.get(3)?,
        party_id: row.get(4)?,
        state: match row.get::<_, u8>(5)? {
            5 => UploadState::RemoteVerified,
            6 => UploadState::Finalized,
            9 => UploadState::Cancelled,
            11 => UploadState::InlineUntracked,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        file_id: row.get(6)?,
        chunk_count: row.get(7)?,
        acknowledged_chunks: row.get(8)?,
        plaintext_bytes: optional_u64_column(row, 9)?,
        reserved_bytes: u64_column(row, 10)?,
        receipt_version: optional_u64_column(row, 11)?,
    })
}
#[derive(Clone, Copy, Debug)]
pub struct UploadScope<'a> {
    pub host_id: &'a [u8],
    pub uid: &'a [u8],
    pub party_id: &'a [u8],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadIntentBinding {
    pub fingerprint: [u8; 32],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadIssuanceContext {
    pub database_id: [u8; 16],
    pub issued_at: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadRegistration {
    pub handle: UploadHandle,
    pub internal_id: [u8; 16],
    pub state: UploadState,
    pub intent: Option<UploadIntentBinding>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadRetentionLimits {
    pub max_rows: u32,
    pub max_bytes: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadSummary {
    pub handle: UploadHandle,
    pub internal_id: [u8; 16],
    pub host_id: Vec<u8>,
    pub uid: Vec<u8>,
    pub party_id: Vec<u8>,
    pub state: UploadState,
    pub file_id: [u8; 17],
    pub chunk_count: u32,
    pub acknowledged_chunks: u32,
    pub plaintext_bytes: Option<u64>,
    pub reserved_bytes: u64,
    pub receipt_version: Option<u64>,
}
/// Exact ledger debt for explicit status/drain reporting. This aggregate scans
/// retained V1 owners plus fixed migrated legacy receipts; it is not constant work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadCleanupDebt {
    pub reserved_bytes: u64,
    pub incomplete: u64,
    pub actionable: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadSummaryPage {
    pub items: Vec<UploadSummary>,
    pub next: Option<[u8; 32]>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UploadCleanupPhase {
    Chunks = 0,
    Manifest = 1,
    Temporaries = 2,
    Dependencies = 3,
    Complete = 4,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadCleanup {
    pub internal_id: [u8; 16],
    pub phase: UploadCleanupPhase,
    pub next_chunk: u32,
    pub last_chunk: Option<u32>,
    pub reserved_bytes: u64,
}

pub fn upload_handle_hash(handle: UploadHandle) -> [u8; 32] {
    foks_crypto::prefixed_hash(0x6d70_59b1_32f8_ecca, handle.to_string().as_bytes())
}
fn scope_valid(scope: UploadScope<'_>, listing: bool) -> Result<()> {
    if scope.host_id.len() != 33
        || scope.uid.len() != 33
        || (scope.party_id.len() != 33 && !(listing && scope.party_id.is_empty()))
    {
        return Err(Error::UploadIdentityConflict);
    }
    Ok(())
}
fn handle_from_sql(text: String) -> rusqlite::Result<UploadHandle> {
    text.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn lookup(
    c: &Connection,
    handle: UploadHandle,
    scope: Option<UploadScope<'_>>,
) -> Result<Option<UploadRegistration>> {
    let row=c.query_row("SELECT h.handle,h.operation_id,h.intent_hash,u.state,u.host_id,u.uid,u.party_id FROM kv_upload_handles h JOIN kv_uploads u ON u.operation_id=h.operation_id WHERE h.handle_hash=?1",[upload_handle_hash(handle).as_slice()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,[u8;16]>(1)?,r.get::<_,Option<[u8;32]>>(2)?,r.get::<_,u8>(3)?,r.get::<_,Vec<u8>>(4)?,r.get::<_,Vec<u8>>(5)?,r.get::<_,Vec<u8>>(6)?))).optional()?;
    let Some((canonical, id, intent, state, host, uid, party)) = row else {
        return Ok(None);
    };
    if canonical != handle.to_string()
        || scope.is_some_and(|s| s.host_id != host || s.uid != uid || s.party_id != party)
    {
        return Err(Error::UploadIdentityConflict);
    }
    Ok(Some(UploadRegistration {
        handle,
        internal_id: id,
        state: UploadState::from_u8(state)?,
        intent: intent.map(|fingerprint| UploadIntentBinding { fingerprint }),
    }))
}
fn read_clock(c: &Connection, host: &[u8], uid: &[u8]) -> Result<Option<UploadClockState>> {
    Ok(c.query_row("SELECT process_id,anchor_wall,anchor_monotonic,validated_time_floor,reject_issued_before,boot_id FROM kv_upload_clocks WHERE host_id=?1 AND user_id=?2",params![host,uid],|r|Ok(UploadClockState{process_id:r.get(0)?,boot_id:r.get(5)?,anchor_wall:u64_column(r,1)?,anchor_monotonic:u64_column(r,2)?,validated_time_floor:u64_column(r,3)?,reject_issued_before:u64_column(r,4)?})).optional()?)
}
fn write_clock(c: &Connection, host: &[u8], uid: &[u8], clock: &UploadClockState) -> Result<()> {
    if read_clock(c, host, uid)?.as_ref() == Some(clock) {
        return Ok(());
    }
    c.execute("INSERT INTO kv_upload_clocks VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(host_id,user_id) DO UPDATE SET process_id=excluded.process_id,anchor_wall=excluded.anchor_wall,anchor_monotonic=excluded.anchor_monotonic,validated_time_floor=excluded.validated_time_floor,reject_issued_before=excluded.reject_issued_before,boot_id=excluded.boot_id",params![host,uid,clock.process_id,sqlite_integer("upload time",clock.anchor_wall)?,sqlite_integer("upload time",clock.anchor_monotonic)?,sqlite_integer("upload time",clock.validated_time_floor)?,sqlite_integer("upload time",clock.reject_issued_before)?,clock.boot_id])?;
    Ok(())
}
fn validate_clock(
    old: Option<&UploadClockState>,
    sample: UploadTimeSample,
) -> Result<UploadClockState> {
    receipt_clock::validate(old, sample).map_err(|_| Error::UploadClockUntrusted)
}
pub(super) fn receipt_admit(c: &Connection) -> Result<()> {
    let okay:bool=c.query_row("SELECT reserved_rows<max_rows AND reserved_bytes<=max_bytes-?1 FROM kv_upload_receipt_limits WHERE singleton=1",[RECEIPT_BYTES as i64],|r|r.get(0))?;
    if !okay {
        return Err(Error::UploadRetentionFull);
    }
    Ok(())
}
pub(crate) fn migrate42(c: &Connection) -> Result<()> {
    c.execute_batch(SCHEMA42)?;
    let ids = c
        .prepare("SELECT operation_id FROM kv_uploads ORDER BY operation_id")?
        .query_map([], |r| r.get::<_, [u8; 16]>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if ids.len() > 1024 {
        return Err(invalid("legacy upload inventory exceeds migration bound"));
    }
    for id in ids {
        let handle = UploadHandle::legacy(id);
        c.execute("INSERT INTO kv_upload_handles(handle_hash,handle,handle_version,operation_id,receipt_bytes) VALUES(?1,?2,0,?3,0)",params![upload_handle_hash(handle),handle.to_string(),id])?;
    }
    Ok(())
}
impl HardStateStore {
    pub fn upload_issuance_context(
        &mut self,
        scope: UploadScope<'_>,
        sample: UploadTimeSample,
    ) -> Result<UploadIssuanceContext> {
        scope_valid(scope, false)?;
        let tx = self.write_transaction()?;
        let state = validate_clock(read_clock(&tx, scope.host_id, scope.uid)?.as_ref(), sample)?;
        if state.validated_time_floor < state.reject_issued_before {
            return Err(Error::UploadClockUntrusted);
        }
        let database_id = tx.query_row(
            "SELECT database_id FROM hard_state_metadata WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        write_clock(&tx, scope.host_id, scope.uid, &state)?;
        tx.commit()?;
        Ok(UploadIssuanceContext {
            database_id,
            issued_at: state.validated_time_floor,
        })
    }
    pub fn upload_registration(
        &self,
        handle: UploadHandle,
        scope: UploadScope<'_>,
    ) -> Result<Option<UploadRegistration>> {
        scope_valid(scope, false)?;
        lookup(&self.connection, handle, Some(scope))
    }
    pub fn upload_registration_for_internal(
        &self,
        id: &[u8; 16],
    ) -> Result<Option<UploadRegistration>> {
        let handle = self
            .connection
            .query_row(
                "SELECT handle FROM kv_upload_handles WHERE operation_id=?1",
                [id.as_slice()],
                |r| handle_from_sql(r.get(0)?),
            )
            .optional()?;
        handle
            .map(|h| lookup(&self.connection, h, None))
            .transpose()
            .map(Option::flatten)
    }
    /// Known exact identities bypass clock/retention admission. Caller publishes its
    /// external checkpoint even when an expired admission commits a higher floor.
    pub fn register_upload_intent(
        &mut self,
        handle: UploadHandle,
        candidate_id: [u8; 16],
        scope: UploadScope<'_>,
        binding: UploadIntentBinding,
        sample: UploadTimeSample,
    ) -> Result<UploadRegistration> {
        scope_valid(scope, false)?;
        let tx = self.write_transaction()?;
        if let Some(existing) = lookup(&tx, handle, Some(scope))? {
            if existing.intent.is_some_and(|b| b != binding) {
                return Err(Error::UploadIdentityConflict);
            }
            tx.commit()?;
            return Ok(existing);
        }
        let Some(generation) = handle.database_id() else {
            return Err(Error::UploadLegacyReadOnly);
        };
        let actual: [u8; 16] = tx.query_row(
            "SELECT database_id FROM hard_state_metadata WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if generation != actual {
            return Err(Error::UploadGenerationMismatch);
        }
        let issued = handle.issued_at().ok_or(Error::UploadLegacyReadOnly)?;
        let old = read_clock(&tx, scope.host_id, scope.uid)?;
        if old
            .as_ref()
            .is_some_and(|s| issued < s.reject_issued_before)
        {
            return Err(Error::UploadExpired);
        }
        let mut clock = validate_clock(old.as_ref(), sample)?;
        let cutoff = clock
            .validated_time_floor
            .saturating_sub(MAX_NEW_UPLOAD_AGE_SECONDS);
        if issued < cutoff {
            clock.reject_issued_before = clock.reject_issued_before.max(cutoff);
        }
        write_clock(&tx, scope.host_id, scope.uid, &clock)?;
        if issued < clock.reject_issued_before {
            tx.commit()?;
            return Err(Error::UploadExpired);
        }
        if issued
            > clock
                .validated_time_floor
                .saturating_add(receipt_clock::SKEW_SECONDS)
        {
            return Err(Error::UploadFutureHandle);
        }
        if clock.validated_time_floor < clock.reject_issued_before {
            return Err(Error::UploadClockUntrusted);
        }
        admit(&tx, 0, true)?;
        receipt_admit(&tx)?;
        let collision: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv_uploads WHERE operation_id=?1)",
            [candidate_id.as_slice()],
            |r| r.get(0),
        )?;
        if collision {
            return Err(Error::UploadIdentityConflict);
        }
        tx.execute("INSERT INTO kv_uploads(operation_id,host_id,uid,party_id,file_id,manifest_hash,state,reserved_bytes,created_at) VALUES(?1,?2,?3,?4,zeroblob(17),zeroblob(32),10,0,?5)",params![candidate_id,scope.host_id,scope.uid,scope.party_id,sqlite_integer("upload creation",clock.validated_time_floor)?])?;
        tx.execute("INSERT INTO kv_upload_handles(handle_hash,handle,handle_version,issued_at,operation_id,intent_hash,receipt_bytes) VALUES(?1,?2,1,?3,?4,?5,?6)",params![upload_handle_hash(handle),handle.to_string(),sqlite_integer("upload issue",issued)?,candidate_id,binding.fingerprint,RECEIPT_BYTES as i64])?;
        tx.execute(
            "INSERT INTO kv_upload_cleanup(operation_id) VALUES(?1)",
            [candidate_id.as_slice()],
        )?;
        tx.commit()?;
        Ok(UploadRegistration {
            handle,
            internal_id: candidate_id,
            state: UploadState::AwaitingSource,
            intent: Some(binding),
        })
    }
    pub fn claim_upload_source(
        &mut self,
        handle: UploadHandle,
        scope: UploadScope<'_>,
        binding: UploadIntentBinding,
    ) -> Result<[u8; 16]> {
        scope_valid(scope, false)?;
        let tx = self.write_transaction()?;
        let row = lookup(&tx, handle, Some(scope))?.ok_or(Error::UploadIdentityConflict)?;
        if row.intent.is_none() {
            return Err(Error::UploadLegacyReadOnly);
        }
        if row.intent != Some(binding) || row.state != UploadState::AwaitingSource {
            return Err(Error::UploadIdentityConflict);
        }
        tx.execute(
            "UPDATE kv_uploads SET state=0 WHERE operation_id=?1 AND state=10",
            [row.internal_id.as_slice()],
        )?;
        tx.commit()?;
        Ok(row.internal_id)
    }
    pub fn upload_retention_limits(&self) -> Result<UploadRetentionLimits> {
        Ok(self.connection.query_row(
            "SELECT max_rows,max_bytes FROM kv_upload_receipt_limits WHERE singleton=1",
            [],
            |r| {
                Ok(UploadRetentionLimits {
                    max_rows: r.get(0)?,
                    max_bytes: u64_column(r, 1)?,
                })
            },
        )?)
    }
    pub fn set_upload_retention_limits(&mut self, limits: UploadRetentionLimits) -> Result<()> {
        self.connection.execute(
            "UPDATE kv_upload_receipt_limits SET max_rows=?1,max_bytes=?2 WHERE singleton=1",
            params![
                limits.max_rows,
                sqlite_integer("upload retention", limits.max_bytes)?
            ],
        )?;
        Ok(())
    }
    pub fn upload_clock(&self, host: &[u8], uid: &[u8]) -> Result<Option<UploadClockState>> {
        read_clock(&self.connection, host, uid)
    }
    /// The clock state upload issuance would hold at `sample`, without
    /// writing, or `None` when issuance would refuse the sample as untrusted.
    /// An account with no anchor validates: its first issuance anchors it.
    pub fn validated_upload_clock(
        &self,
        host: &[u8],
        uid: &[u8],
        sample: UploadTimeSample,
    ) -> Result<Option<UploadClockState>> {
        let old = read_clock(&self.connection, host, uid)?;
        match receipt_clock::validate(old.as_ref(), sample) {
            Ok(state) => Ok(Some(state)),
            Err(Error::AdapterClockUntrusted) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub fn preview_upload_clock_reanchor(
        &self,
        host: &[u8],
        uid: &[u8],
        sample: UploadTimeSample,
    ) -> Result<UploadClockState> {
        receipt_clock::reanchor(read_clock(&self.connection, host, uid)?.as_ref(), sample)
            .map_err(|_| Error::UploadClockUntrusted)
    }
    pub fn reanchor_upload_clock(
        &mut self,
        host: &[u8],
        uid: &[u8],
        expected: &UploadClockState,
        sample: UploadTimeSample,
    ) -> Result<UploadClockState> {
        let tx = self.write_transaction()?;
        let old = read_clock(&tx, host, uid)?;
        if old.as_ref() != Some(expected) {
            return Err(Error::UploadClockUntrusted);
        }
        let next = receipt_clock::reanchor(old.as_ref(), sample)
            .map_err(|_| Error::UploadClockUntrusted)?;
        write_clock(&tx, host, uid, &next)?;
        tx.commit()?;
        Ok(next)
    }
    pub fn upload_summaries_page(
        &self,
        scope: UploadScope<'_>,
        after: Option<[u8; 32]>,
        limit: u32,
    ) -> Result<UploadSummaryPage> {
        scope_valid(scope, true)?;
        if !(1..=128).contains(&limit) {
            return Err(invalid("upload page limit must be 1..128"));
        }
        let mut q=self.connection.prepare("SELECT h.handle_hash,h.handle,u.operation_id,u.party_id,u.state,u.file_id,u.chunk_count,u.acknowledged_chunks,u.plaintext_bytes,u.reserved_bytes,u.receipt_version FROM kv_upload_handles h JOIN kv_uploads u ON u.operation_id=h.operation_id WHERE u.host_id=?1 AND u.uid=?2 AND (?3=zeroblob(0) OR u.party_id=?3) AND (?4 IS NULL OR h.handle_hash>?4) ORDER BY h.handle_hash LIMIT ?5")?;
        let rows = q
            .query_map(
                params![
                    scope.host_id,
                    scope.uid,
                    scope.party_id,
                    after.as_ref().map(|a| a.as_slice()),
                    limit + 1
                ],
                |r| {
                    Ok((
                        r.get::<_, [u8; 32]>(0)?,
                        UploadSummary {
                            handle: handle_from_sql(r.get(1)?)?,
                            internal_id: r.get(2)?,
                            host_id: scope.host_id.to_vec(),
                            uid: scope.uid.to_vec(),
                            party_id: r.get(3)?,
                            state: match r.get::<_, u8>(4)? {
                                0 => UploadState::Preparing,
                                1 => UploadState::Ready,
                                2 => UploadState::Uploading,
                                3 => UploadState::ContentComplete,
                                4 => UploadState::Publishing,
                                5 => UploadState::RemoteVerified,
                                6 => UploadState::Finalized,
                                7 => UploadState::NeedsSource,
                                8 => UploadState::Blocked,
                                9 => UploadState::Cancelled,
                                10 => UploadState::AwaitingSource,
                                _ => UploadState::InlineUntracked,
                            },
                            file_id: r.get(5)?,
                            chunk_count: r.get(6)?,
                            acknowledged_chunks: r.get(7)?,
                            plaintext_bytes: optional_u64_column(r, 8)?,
                            reserved_bytes: u64_column(r, 9)?,
                            receipt_version: optional_u64_column(r, 10)?,
                        },
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next = (rows.len() > limit as usize).then(|| rows[limit as usize - 1].0);
        Ok(UploadSummaryPage {
            items: rows
                .into_iter()
                .take(limit as usize)
                .map(|(_, s)| s)
                .collect(),
            next,
        })
    }
    pub fn upload_cleanup_debt(&self) -> Result<UploadCleanupDebt> {
        let (reserved_bytes, incomplete, actionable, missing): (u64,u64,u64,u64) = self.connection.query_row(
            "SELECT coalesce(sum(u.reserved_bytes),0),coalesce(sum(CASE WHEN c.phase<4 THEN 1 ELSE 0 END),0),coalesce(sum(CASE WHEN c.phase<4 AND u.state IN (5,6,9,11) THEN 1 ELSE 0 END),0),coalesce(sum(CASE WHEN c.operation_id IS NULL THEN 1 ELSE 0 END),0) FROM kv_uploads u LEFT JOIN kv_upload_cleanup c ON c.operation_id=u.operation_id",
            [],
            |r| Ok((u64_column(r,0)?,u64_column(r,1)?,u64_column(r,2)?,u64_column(r,3)?)),
        )?;
        if missing != 0 {
            return Err(invalid("upload owner has no cleanup ledger"));
        }
        Ok(UploadCleanupDebt {
            reserved_bytes,
            incomplete,
            actionable,
        })
    }
    /// Indexed actionable cleanup only; retained Complete receipts are excluded.
    /// Unresolved candidates skipped by the state filter are bounded by max_pending.
    pub fn next_upload_cleanup(&self, after: Option<[u8; 16]>) -> Result<Option<UploadSummary>> {
        Ok(match after {
            Some(id) => self
                .connection
                .query_row(CLEANUP_AFTER, [id.as_slice()], maintenance_summary)
                .optional()?,
            None => self
                .connection
                .query_row(CLEANUP_FIRST, [], maintenance_summary)
                .optional()?,
        })
    }
    /// Terminal V1 receipts needing a timestamp, even after cleanup is Complete.
    pub fn next_upload_unstamped(&self, after: Option<[u8; 16]>) -> Result<Option<UploadSummary>> {
        Ok(match after {
            Some(id) => self
                .connection
                .query_row(UNSTAMPED_AFTER, [id.as_slice()], maintenance_summary)
                .optional()?,
            None => self
                .connection
                .query_row(UNSTAMPED_FIRST, [], maintenance_summary)
                .optional()?,
        })
    }
    /// Independent clock cursor: pruning must also run when cleanup has no work.
    pub fn next_upload_clock_account(
        &self,
        after: Option<(&[u8], &[u8])>,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let decode = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?));
        Ok(match after {
            Some((host, uid)) => self
                .connection
                .query_row(CLOCK_AFTER, params![host, uid], decode)
                .optional()?,
            None => self
                .connection
                .query_row(CLOCK_FIRST, [], decode)
                .optional()?,
        })
    }
    pub fn record_upload_terminal(
        &mut self,
        id: &[u8; 16],
        sample: Option<UploadTimeSample>,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        let (host, uid, state): (Vec<u8>, Vec<u8>, u8) = tx.query_row(
            "SELECT host_id,uid,state FROM kv_uploads WHERE operation_id=?1",
            [id.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if !UploadState::from_u8(state)?.terminal() {
            return Err(invalid("upload outcome is not terminal"));
        }
        let needs_timestamp: bool = tx.query_row(
            "SELECT handle_version=1 AND terminal_at IS NULL FROM kv_upload_handles WHERE operation_id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )?;
        if !needs_timestamp {
            // No write, including clock/revision/token changes. The empty
            // transaction rolls back without invalidating a protected-file scan.
            return Ok(());
        }
        let old = read_clock(&tx, &host, &uid)?;
        if let Some(clock) = sample.and_then(|s| validate_clock(old.as_ref(), s).ok()) {
            write_clock(&tx, &host, &uid, &clock)?;
            tx.execute("UPDATE kv_upload_handles SET terminal_at=?2,expires_at=?3 WHERE operation_id=?1 AND handle_version=1 AND terminal_at IS NULL",params![id.as_slice(),sqlite_integer("upload terminal",clock.validated_time_floor)?,sqlite_integer("upload expiry",clock.validated_time_floor+UPLOAD_TERMINAL_RETENTION_SECONDS)?])?;
        }
        tx.commit()?;
        Ok(())
    }
}
fn cleanup_on(c: &Connection, id: &[u8; 16]) -> Result<Option<UploadCleanup>> {
    let row=c.query_row("SELECT c.phase,c.next_chunk,u.chunk_count,u.reserved_bytes,u.state,u.file_id FROM kv_upload_cleanup c JOIN kv_uploads u ON u.operation_id=c.operation_id WHERE c.operation_id=?1",[id.as_slice()],|r|Ok((r.get::<_,u8>(0)?,r.get::<_,u32>(1)?,r.get::<_,u32>(2)?,u64_column(r,3)?,r.get::<_,u8>(4)?,r.get::<_,[u8;17]>(5)?))).optional()?;
    let Some((phase, next_chunk, count, reserved_bytes, state, file_id)) = row else {
        return Ok(None);
    };
    if !UploadState::from_u8(state)?.terminal() {
        return Ok(None);
    }
    let phase = match phase {
        0 => UploadCleanupPhase::Chunks,
        1 => UploadCleanupPhase::Manifest,
        2 => UploadCleanupPhase::Temporaries,
        3 => UploadCleanupPhase::Dependencies,
        4 => UploadCleanupPhase::Complete,
        _ => return Err(invalid("invalid upload cleanup phase")),
    };
    // The next possible fsync-before-journal chunk remains owned until erased.
    let last_chunk = (file_id != [0; 17]).then_some(count.min(255));
    Ok(Some(UploadCleanup {
        internal_id: *id,
        phase,
        next_chunk,
        last_chunk,
        reserved_bytes,
    }))
}
impl HardStateStore {
    pub fn upload_cleanup(&self, id: &[u8; 16]) -> Result<Option<UploadCleanup>> {
        cleanup_on(&self.connection, id)
    }
    /// Caller holds profile exclusion and completes durable protected erasure before
    /// advancing a chunk/manifest, and a complete temporary sweep before advancing
    /// Temporaries. No phase releases debt before every dependency is settled.
    pub fn advance_upload_cleanup(
        &mut self,
        id: &[u8; 16],
        expected: &UploadCleanup,
    ) -> Result<UploadCleanup> {
        let tx = self.write_transaction()?;
        let current = cleanup_on(&tx, id)?.ok_or(Error::UploadCleanupDeferred)?;
        if &current != expected {
            return Err(Error::UploadCleanupDeferred);
        }
        let (phase, next) = match current.phase {
            UploadCleanupPhase::Chunks => {
                if current
                    .last_chunk
                    .is_some_and(|last| current.next_chunk < last)
                {
                    (UploadCleanupPhase::Chunks, current.next_chunk + 1)
                } else {
                    (UploadCleanupPhase::Manifest, current.next_chunk)
                }
            }
            UploadCleanupPhase::Manifest => (UploadCleanupPhase::Temporaries, current.next_chunk),
            UploadCleanupPhase::Temporaries => {
                (UploadCleanupPhase::Dependencies, current.next_chunk)
            }
            UploadCleanupPhase::Dependencies => {
                let blocked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM kv_upload_children WHERE upload_id=?1) OR EXISTS(SELECT 1 FROM kv_uploads WHERE operation_id=?1 AND (adapter_parent IS NOT NULL OR namespace_child IS NOT NULL))",[id.as_slice()],|r|r.get(0))?;
                if blocked {
                    return Err(Error::UploadCleanupDeferred);
                }
                tx.execute(
                    "UPDATE kv_uploads SET reserved_bytes=0 WHERE operation_id=?1",
                    [id.as_slice()],
                )?;
                tx.execute(
                    "DELETE FROM kv_upload_chunks WHERE operation_id=?1",
                    [id.as_slice()],
                )?;
                (UploadCleanupPhase::Complete, current.next_chunk)
            }
            UploadCleanupPhase::Complete => {
                tx.commit()?;
                return Ok(current);
            }
        };
        tx.execute(
            "UPDATE kv_upload_cleanup SET phase=?2,next_chunk=?3 WHERE operation_id=?1",
            params![id.as_slice(), phase as u8, next],
        )?;
        let result = cleanup_on(&tx, id)?.ok_or(Error::UploadCleanupDeferred)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn upload_children(&self, id: &[u8; 16]) -> Result<Vec<[u8; 16]>> {
        let rows=self.connection.prepare("SELECT child_id FROM kv_upload_children WHERE upload_id=?1 ORDER BY child_id LIMIT 257")?.query_map([id.as_slice()],|r|r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > 256 {
            return Err(Error::UploadCleanupDeferred);
        }
        Ok(rows)
    }
    /// After protected material is removed, compact a terminal standalone child or
    /// acknowledge the explicit proof left by adapter compaction. Missing alone is
    /// never proof of a settled dependency.
    pub fn retire_upload_child(&mut self, id: &[u8; 16], child: &[u8; 16]) -> Result<()> {
        let tx = self.write_transaction()?;
        let owner_terminal: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv_uploads WHERE operation_id=?1 AND state IN (6,9,11))",
            [id.as_slice()],
            |r| r.get(0),
        )?;
        if !owner_terminal {
            return Err(Error::UploadCleanupDeferred);
        }
        let compacted:Option<bool>=tx.query_row("SELECT compacted FROM kv_upload_children WHERE upload_id=?1 AND child_id=?2 AND protected_erased=1",params![id.as_slice(),child.as_slice()],|r|r.get(0)).optional()?;
        let Some(compacted) = compacted else {
            return Err(Error::UploadCleanupDeferred);
        };
        let mutation: Option<(u8, u8)> = tx
            .query_row(
                "SELECT operation_kind,state FROM mutation_operations WHERE operation_id=?1",
                [child.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let other_owner:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM mutation_children WHERE child_id=?1 OR parent_id=?1) OR EXISTS(SELECT 1 FROM kv_adapter_submissions WHERE internal_id=?1)",[child.as_slice()],|r|r.get(0))?;
        if other_owner
            || match mutation {
                Some((kind, state)) => !matches!(kind, 5..=7) || !matches!(state, 5 | 6),
                None => !compacted,
            }
        {
            return Err(Error::UploadCleanupDeferred);
        }
        tx.execute(
            "DELETE FROM kv_upload_children WHERE upload_id=?1 AND child_id=?2",
            params![id.as_slice(), child.as_slice()],
        )?;
        tx.execute("DELETE FROM mutation_operations WHERE operation_id=?1 AND operation_kind IN (5,6,7) AND state IN (5,6)",[child.as_slice()])?;
        tx.commit()?;
        Ok(())
    }
    /// Retire at most 128 fully cleaned V1 receipts and atomically advance the
    /// irreversible admission floor. Legacy receipts remain bounded and retained.
    pub fn prune_upload_receipts(
        &mut self,
        host: &[u8],
        uid: &[u8],
        sample: UploadTimeSample,
        limit: u32,
    ) -> Result<u32> {
        if !(1..=128).contains(&limit) {
            return Err(invalid("upload prune limit must be 1..128"));
        }
        let tx = self.write_transaction()?;
        let mut clock = validate_clock(read_clock(&tx, host, uid)?.as_ref(), sample)?;
        let rows = {
            let mut q = tx.prepare(PRUNE_CANDIDATES)?;
            let rows = q
                .query_map(
                    params![
                        host,
                        uid,
                        sqlite_integer("upload prune", clock.validated_time_floor)?,
                        limit
                    ],
                    |r| {
                        Ok((
                            r.get::<_, [u8; 32]>(0)?,
                            r.get::<_, [u8; 16]>(1)?,
                            u64_column(r, 2)?,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (hash, id, issued) in &rows {
            clock.reject_issued_before = clock
                .reject_issued_before
                .max(issued.checked_add(1).ok_or(Error::UploadClockUntrusted)?);
            tx.execute(
                "DELETE FROM kv_upload_handles WHERE handle_hash=?1",
                [hash.as_slice()],
            )?;
            tx.execute(
                "DELETE FROM kv_uploads WHERE operation_id=?1",
                [id.as_slice()],
            )?;
        }
        write_clock(&tx, host, uid, &clock)?;
        tx.commit()?;
        Ok(rows.len() as u32)
    }
}

impl HardStateStore {
    pub fn next_upload_child_cleanup(&self, id: &[u8; 16]) -> Result<Option<[u8; 16]>> {
        let blocked:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM kv_upload_children c LEFT JOIN mutation_operations m ON m.operation_id=c.child_id WHERE c.upload_id=?1 AND c.protected_erased=0 AND (m.operation_id IS NULL OR m.state NOT IN (5,6)))",[id.as_slice()],|r|r.get(0))?;
        if blocked {
            return Err(Error::UploadCleanupDeferred);
        }
        Ok(self.connection.query_row("SELECT child_id FROM kv_upload_children WHERE upload_id=?1 AND protected_erased=0 ORDER BY child_id LIMIT 1",[id.as_slice()],|r|r.get(0)).optional()?)
    }
    /// Must follow durable protected-record erasure under profile exclusion.
    pub fn mark_upload_child_erased(&mut self, id: &[u8; 16], child: &[u8; 16]) -> Result<()> {
        let changed=self.connection.execute("UPDATE kv_upload_children SET protected_erased=1 WHERE upload_id=?1 AND child_id=?2 AND (protected_erased=1 OR EXISTS(SELECT 1 FROM mutation_operations m WHERE m.operation_id=child_id AND m.operation_kind IN (5,6,7) AND m.state IN (5,6)))",params![id.as_slice(),child.as_slice()])?;
        if changed != 1 {
            return Err(Error::UploadCleanupDeferred);
        }
        Ok(())
    }
    pub fn upload_parent_erased(&self, id: &[u8; 16]) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT parent_erased FROM kv_upload_cleanup WHERE operation_id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )?)
    }
    /// Must follow durable adapter-parent protected-record erasure.
    pub fn mark_upload_parent_erased(&mut self, id: &[u8; 16]) -> Result<()> {
        let changed=self.connection.execute("UPDATE kv_upload_cleanup SET parent_erased=1 WHERE operation_id=?1 AND EXISTS(SELECT 1 FROM kv_uploads u WHERE u.operation_id=?1 AND (u.adapter_parent IS NULL OR EXISTS(SELECT 1 FROM mutation_operations m WHERE m.operation_id=u.adapter_parent AND m.operation_kind=8 AND m.state IN (5,6))))",[id.as_slice()])?;
        if changed != 1 {
            return Err(Error::UploadCleanupDeferred);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
