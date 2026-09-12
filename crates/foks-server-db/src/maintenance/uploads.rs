//! Bounded, restartable reclamation of uploads with no retained dirent reference.

use std::time::{Duration, Instant};

use rusqlite::{params, OptionalExtension, Transaction};

use crate::{error::sql_integer, Error, Result};

const CANDIDATES: usize = 128;
const CHUNK_ROWS: u64 = 16;
const CHUNK_BYTES: u64 = 32 * 1024 * 1024;
const WORK_BUDGET: Duration = Duration::from_millis(50);

#[derive(Default)]
pub(super) struct UploadReport {
    pub examined: u64,
    pub marked: u64,
    pub uploads: u64,
    pub chunks: u64,
    pub bytes: u64,
    pub deferred: bool,
}

pub(super) fn reclaim(transaction: &Transaction<'_>, cutoff: u64) -> Result<UploadReport> {
    let started = Instant::now();
    let mut report = UploadReport::default();
    let cursor: (i64, Vec<u8>, Vec<u8>) = transaction.query_row(
        "SELECT updated_at, uid, file_id FROM kv_upload_maintenance WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let candidates = {
        let mut statement = transaction.prepare(
            "SELECT updated_at, uid, file_id FROM kv_file_uploads
             WHERE reclaiming = 0 AND updated_at <= ?1
               AND (updated_at, uid, file_id) > (?2, ?3, ?4)
             ORDER BY updated_at, uid, file_id LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![
                sql_integer(cutoff)?,
                cursor.0,
                cursor.1,
                cursor.2,
                CANDIDATES as i64
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut last = None;
    for (updated_at, uid, file_id) in &candidates {
        if started.elapsed() >= WORK_BUDGET {
            report.deferred = true;
            break;
        }
        // This liveness decision and publication share the same SQLite writer.
        // The partial expression index includes *all* historical versions.
        report.marked += transaction.execute(
            "UPDATE kv_file_uploads SET reclaiming = 1
             WHERE uid = ?1 AND file_id = ?2 AND reclaiming = 0
               AND NOT EXISTS (
                 SELECT 1 FROM kv_dirents d
                 WHERE d.uid = ?1 AND substr(d.node_id, 1, 1) = X'02'
                   AND substr(d.node_id, 2, 16) = ?2
               )",
            params![uid, file_id],
        )? as u64;
        report.examined += 1;
        last = Some((updated_at, uid, file_id));
    }
    if let Some((updated_at, uid, file_id)) = last {
        transaction.execute(
            "UPDATE kv_upload_maintenance SET updated_at = ?1, uid = ?2, file_id = ?3
             WHERE singleton = 1",
            params![updated_at, uid, file_id],
        )?;
    }
    if report.examined == candidates.len() as u64 && candidates.len() < CANDIDATES {
        // Wrap only after exhausting this age window. Newly aged or updated rows
        // behind the cursor are examined in the next cycle, including after restart.
        transaction.execute(
            "UPDATE kv_upload_maintenance SET updated_at = 0, uid = X'', file_id = X''
             WHERE singleton = 1",
            [],
        )?;
    }
    report.deferred |= candidates.len() == CANDIDATES;
    let reclaiming = {
        let mut statement = transaction.prepare(
            "SELECT uid, file_id FROM kv_file_uploads WHERE reclaiming = 1
             ORDER BY updated_at, uid, file_id LIMIT ?1",
        )?;
        let rows = statement.query_map([CANDIDATES as i64], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (uid, file_id) in &reclaiming {
        loop {
            if started.elapsed() >= WORK_BUDGET {
                report.deferred = true;
                return Ok(report);
            }
            // Fetch lengths, never payloads. One accepted chunk fits the byte
            // budget, so the first row always makes progress in a fresh batch.
            let chunk: Option<(i64, i64)> = transaction
                .query_row(
                    "SELECT clear_offset, length(ciphertext) + length(exact_upload_chunk)
                 FROM kv_file_chunks WHERE uid = ?1 AND file_id = ?2
                 ORDER BY clear_offset LIMIT 1",
                    params![uid, file_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((offset, bytes)) = chunk else {
                // No remaining cascade: every chunk was removed within budgets.
                report.uploads += transaction.execute(
                    "DELETE FROM kv_file_uploads WHERE uid = ?1 AND file_id = ?2
                     AND reclaiming = 1 AND NOT EXISTS (
                       SELECT 1 FROM kv_file_chunks WHERE uid = ?1 AND file_id = ?2
                     )",
                    params![uid, file_id],
                )? as u64;
                break;
            };
            let bytes = u64::try_from(bytes).map_err(|_| Error::IntegerRange)?;
            if report.chunks >= CHUNK_ROWS || bytes > CHUNK_BYTES - report.bytes {
                report.deferred = true;
                return Ok(report);
            }
            transaction.execute(
                "DELETE FROM kv_file_chunks WHERE uid = ?1 AND file_id = ?2 AND clear_offset = ?3",
                params![uid, file_id, offset],
            )?;
            report.chunks += 1;
            report.bytes += bytes;
        }
    }
    report.deferred |= reclaiming.len() == CANDIDATES;
    Ok(report)
}
