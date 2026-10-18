//! Offline acceptance checks, not a replacement for request authentication/error order.
use super::*;
use rusqlite::{types::ValueRef, OptionalExtension};

fn nonnegative(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| unsupported("authority unsigned conversion"))
}
fn blob(row: &rusqlite::Row<'_>, column: usize) -> Result<()> {
    row.get_ref(column)?
        .as_blob()
        .map(|_| ())
        .map_err(|_| unsupported("authority blob conversion"))
}
fn width<const N: usize>(row: &rusqlite::Row<'_>, column: usize) -> Result<[u8; N]> {
    row.get_ref(column)?
        .as_blob()
        .map_err(|_| unsupported("authority fixed blob type"))?
        .try_into()
        .map_err(|_| unsupported("authority fixed blob width"))
}

pub(super) fn verify(db: &scan::Database) -> Result<u64> {
    let mut previous: Option<[u8; 33]> = None;
    let mut admitted = 0u64;
    loop {
        db.control().statement()?;
        let uid = if let Some(previous) = previous {
            db.connection()
                .query_row(
                    "SELECT uid FROM main.users WHERE uid>?1 ORDER BY uid LIMIT 1",
                    [previous.as_slice()],
                    |r| r.get::<_, Vec<u8>>(0),
                )
                .optional()?
        } else {
            db.connection()
                .query_row("SELECT uid FROM main.users ORDER BY uid LIMIT 1", [], |r| {
                    r.get::<_, Vec<u8>>(0)
                })
                .optional()?
        };
        let Some(uid) = uid else { break };
        let uid: [u8; 33] = uid
            .try_into()
            .map_err(|_| unsupported("authority UID width"))?;
        if verify_user(db, &uid)? {
            admitted = admitted
                .checked_add(1)
                .ok_or(unsupported("authority audit count"))?;
        }
        previous = Some(uid);
    }
    db.control().check()?;
    Ok(admitted)
}
pub(super) fn verify_user(db: &scan::Database, uid: &[u8; 33]) -> Result<bool> {
    db.control().statement()?;
    // Native returns None before inspecting any device/key/stale row when this join is absent.
    let mut head=db.connection().prepare("SELECT h.seqno,h.link_hash,t.location,r.epoch,r.root_hash FROM main.user_chain_heads h JOIN main.tree_locations t ON t.uid=h.uid AND t.seqno=h.seqno JOIN main.merkle_root_heads rh ON rh.singleton=1 JOIN main.merkle_roots r ON r.epoch=rh.epoch WHERE h.uid=?1")?;
    let mut heads = head.query([uid.as_slice()])?;
    let Some(row) = heads.next()? else {
        return Ok(false);
    };
    // Native obtains the SQL types before its child scans. Fixed-width/unsigned checks
    // happen last below; no snapshot Vec proportional to user history is retained.
    let sequence: i64 = row.get(0)?;
    let epoch: i64 = row.get(3)?;
    let tail = row
        .get_ref(1)?
        .as_blob()
        .map_err(|_| unsupported("authority tail type"))?;
    let location = row
        .get_ref(2)?
        .as_blob()
        .map_err(|_| unsupported("authority location type"))?;
    let root = row
        .get_ref(4)?
        .as_blob()
        .map_err(|_| unsupported("authority root type"))?;
    let lengths = (tail.len(), location.len(), root.len());
    drop(heads);
    drop(head);
    // No paused statement is resumed after a subsequent statement resets VM work.
    db.control().statement()?;
    let mut devices=db.connection().prepare("SELECT device_id,active,role_type,visibility,subkey_id,exact_hepk,exact_name FROM main.devices WHERE uid=?1 ORDER BY device_id")?;
    let mut rows = devices.query([uid.as_slice()])?;
    while let Some(row) = rows.next()? {
        db.control().check()?;
        blob(row, 0)?;
        let _: i64 = row.get(1)?;
        nonnegative(row.get(2)?)?;
        let _: i64 = row.get(3)?;
        match row.get_ref(4)? {
            ValueRef::Null | ValueRef::Blob(_) => {}
            _ => return Err(unsupported("authority subkey type")),
        }
        blob(row, 5)?;
        blob(row, 6)?;
    }
    drop(rows);
    drop(devices);
    db.control().statement()?;
    let mut keys=db.connection().prepare("SELECT role_type,visibility,generation,verify_key,exact_hepk FROM main.shared_keys WHERE uid=?1 ORDER BY role_type,visibility,generation")?;
    let mut rows = keys.query([uid.as_slice()])?;
    while let Some(row) = rows.next()? {
        db.control().check()?;
        let role = nonnegative(row.get(0)?)?;
        let visibility: i64 = row.get(1)?;
        nonnegative(row.get(2)?)?;
        blob(row, 3)?;
        blob(row, 4)?;
        if crate::auth::team::stored_role(role, visibility).is_none() {
            return Err(unsupported("authority stored key role"));
        }
    }
    drop(rows);
    drop(keys);
    db.control().statement()?;
    let mut stale=db.connection().prepare("SELECT DISTINCT p.role_type,p.visibility FROM main.parcels p JOIN main.devices d ON d.uid=p.uid AND d.device_id=p.device_id JOIN (SELECT role_type,visibility,max(generation) AS generation FROM main.shared_keys WHERE uid=?1 GROUP BY role_type,visibility) current ON current.role_type=p.role_type AND current.visibility=p.visibility AND current.generation=p.generation WHERE p.uid=?1 AND d.active=0 ORDER BY p.role_type,p.visibility")?;
    let mut rows = stale.query([uid.as_slice()])?;
    while let Some(row) = rows.next()? {
        db.control().check()?;
        nonnegative(row.get(0)?)?;
        let _: i64 = row.get(1)?;
    }
    nonnegative(sequence)?;
    nonnegative(epoch)?;
    if lengths != (32, 32, 32) {
        return Err(unsupported("authority head widths"));
    }
    db.control().check()?;
    Ok(true)
}

pub(super) fn coordinate(db: &scan::Database) -> Result<foks_server_db::ReadDependency> {
    db.control().statement()?;
    let mut stmt = db.connection().prepare(
        "SELECT incarnation,revision FROM main.metadata_read_dependency WHERE singleton=1",
    )?;
    let mut rows = stmt.query([])?;
    let row = rows.next()?.ok_or(unsupported("dependency missing"))?;
    let incarnation = width::<32>(row, 0)?;
    let revision = nonnegative(row.get(1)?)?;
    if incarnation == [0; 32] || revision == 0 {
        return Err(unsupported("dependency invalid"));
    }
    if rows.next()?.is_some() {
        return Err(unsupported("dependency cardinality"));
    }
    Ok(foks_server_db::ReadDependency {
        incarnation,
        revision,
    })
}
