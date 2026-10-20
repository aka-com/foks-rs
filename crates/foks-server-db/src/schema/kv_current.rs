use crate::allocation::RowAllocationExt as _;
use crate::{Error, Result};
use rusqlite::Connection;

pub(super) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(include_str!("kv_current.sql"))?;
    // Fail closed on a missing head target, including databases previously
    // written with foreign_keys disabled. Upgrade commits only a full projection.
    let invalid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM kv_dirent_heads h LEFT JOIN kv_dirents d
         ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
         WHERE d.uid IS NULL)", [], |row| row.budgeted_get(0),
    )?;
    if invalid {
        return Err(Error::Invalid("KV head projection source is incomplete"));
    }
    let invalid: bool = connection.query_row(
        "WITH expected AS (
          SELECT d.uid,d.parent_id,d.dirent_id,d.version,d.name_mac,d.creation_time,d.node_id
          FROM kv_dirent_heads h JOIN kv_dirents d
          ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
          WHERE substr(d.node_id,1,1)!=X'00')
         SELECT EXISTS(SELECT * FROM expected EXCEPT SELECT * FROM kv_dirent_current)
             OR EXISTS(SELECT * FROM kv_dirent_current EXCEPT SELECT * FROM expected)",
        [], |row| row.budgeted_get(0),
    )?;
    if invalid || connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Invalid("KV head projection validation failed"));
    }
    Ok(())
}
