use crate::allocation::RowAllocationExt as _;
use crate::{Error, Result};
use rusqlite::{params, Transaction};

/// A malformed legacy box stays unvalidated; resource failures never become
/// durable corruption classifications. Borrow SQLite bytes before decoding so
/// migration never first materializes an unbounded collection of exact boxes.
fn derive(exact: &[u8]) -> Result<Option<i64>> {
    // Every supported opener already rejects a larger stored node. Inspect the
    // borrowed length before decoding instead of allocating an oversized legacy
    // row merely to reach that same admission failure later.
    if exact.len() > foks_proto::MAXIMUM_KV_NODE_BYTES {
        return Err(Error::QuotaExceeded);
    }
    match foks_proto::KvSmallFileBox::ciphertext_length(exact) {
        Ok(length) => Ok(Some(crate::error::sql_integer(length)?)),
        Err(foks_proto::Error::Snowpack(error))
            if error.kind == foks_snowpack::ErrorKind::AllocationLimit =>
        {
            Err(Error::Capacity("KV usage migration allocation"))
        }
        Err(_) => Ok(None),
    }
}

pub(super) fn migrate(connection: &Transaction<'_>) -> Result<()> {
    connection.execute_batch(include_str!("kv_usage.sql"))?;
    // Upload totals predate Usage. Validate them against logical chunk data,
    // including external representation, before treating them as authoritative.
    let invalid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM kv_file_uploads u WHERE
         u.chunk_count != (SELECT count(*) FROM kv_file_chunks c WHERE c.uid=u.uid AND c.file_id=u.file_id)
         OR u.chunk_bytes != coalesce((SELECT sum(coalesce(c.external_ciphertext_bytes,length(c.ciphertext)))
          FROM kv_file_chunks c WHERE c.uid=u.uid AND c.file_id=u.file_id),0))",
        [], |r| r.budgeted_get(0),
    )?;
    if invalid {
        return Err(Error::Invalid("stored KV upload usage"));
    }
    let mut previous: Option<([u8; 33], [u8; 17])> = None;
    loop {
        let mut statement = connection.prepare(match previous {
            None => "SELECT uid,node_id,exact_node FROM kv_nodes ORDER BY uid,node_id LIMIT 1",
            Some(_) => "SELECT uid,node_id,exact_node FROM kv_nodes WHERE (uid,node_id)>(?1,?2) ORDER BY uid,node_id LIMIT 1",
        })?;
        let mut rows = match previous.as_ref() {
            None => statement.query([])?,
            Some((uid, id)) => statement.query(params![uid.as_slice(), id.as_slice()])?,
        };
        let Some(row) = rows.next()? else { break };
        let uid: [u8; 33] = row
            .get_ref(0)?
            .as_blob()
            .map_err(|_| Error::Invalid("KV namespace ID"))?
            .try_into()
            .map_err(|_| Error::Invalid("KV namespace ID"))?;
        let id: [u8; 17] = row
            .get_ref(1)?
            .as_blob()
            .map_err(|_| Error::Invalid("KV node ID"))?
            .try_into()
            .map_err(|_| Error::Invalid("KV node ID"))?;
        let exact = row
            .get_ref(2)?
            .as_blob()
            .map_err(|_| Error::Invalid("stored KV small node"))?;
        let length = derive(exact)?;
        if connection.execute(
            "UPDATE kv_nodes SET ciphertext_bytes=?4 WHERE uid=?1 AND node_id=?2 AND exact_node=?3",
            params![uid.as_slice(), id.as_slice(), exact, length],
        )? != 1
        {
            return Err(Error::Invalid("KV node changed during usage migration"));
        }
        previous = Some((uid, id));
    }
    connection.execute_batch(
        "UPDATE kv_namespaces SET
         usage_small_count=(SELECT count(*) FROM kv_nodes WHERE uid=namespace_id),
         usage_small_bytes=coalesce((SELECT sum(ciphertext_bytes) FROM kv_nodes WHERE uid=namespace_id),0),
         usage_unvalidated_small_count=(SELECT count(*) FROM kv_nodes WHERE uid=namespace_id AND ciphertext_bytes IS NULL),
         usage_large_count=(SELECT count(*) FROM kv_file_uploads WHERE uid=namespace_id),
         usage_large_bytes=coalesce((SELECT sum(chunk_bytes) FROM kv_file_uploads WHERE uid=namespace_id),0),
         usage_large_chunks=coalesce((SELECT sum(chunk_count) FROM kv_file_uploads WHERE uid=namespace_id),0);"
    )?;
    // Independent checked Rust scan validates every aggregate. Keep one namespace
    // and one borrowed payload resident, not a map proportional to tenant history.
    let mut namespaces = connection.prepare("SELECT namespace_id,usage_small_count,usage_small_bytes,usage_unvalidated_small_count,usage_large_count,usage_large_bytes,usage_large_chunks FROM kv_namespaces ORDER BY namespace_id")?;
    let mut rows = namespaces.query([])?;
    while let Some(row) = rows.next()? {
        let uid = row
            .get_ref(0)?
            .as_blob()
            .map_err(|_| Error::Invalid("KV namespace ID"))?;
        let mut expected = [0i64; 6];
        let mut nodes = connection.prepare("SELECT exact_node FROM kv_nodes WHERE uid=?1")?;
        let mut values = nodes.query([uid])?;
        while let Some(node) = values.next()? {
            let exact = node
                .get_ref(0)?
                .as_blob()
                .map_err(|_| Error::Invalid("stored KV small node"))?;
            add(&mut expected[0], 1)?;
            match derive(exact)? {
                Some(bytes) => add(&mut expected[1], bytes)?,
                None => add(&mut expected[2], 1)?,
            }
        }
        let mut uploads = connection
            .prepare("SELECT chunk_bytes,chunk_count FROM kv_file_uploads WHERE uid=?1")?;
        let mut values = uploads.query([uid])?;
        while let Some(upload) = values.next()? {
            add(&mut expected[3], 1)?;
            add(&mut expected[4], upload.budgeted_get(0)?)?;
            add(&mut expected[5], upload.budgeted_get(1)?)?;
        }
        for (index, value) in expected.into_iter().enumerate() {
            if value != row.budgeted_get::<_, i64>(index + 1)? {
                return Err(Error::Invalid("KV usage backfill mismatch"));
            }
        }
    }
    if connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Invalid("KV usage source foreign key mismatch"));
    }
    connection.execute_batch(include_str!("kv_usage_triggers.sql"))?;
    Ok(())
}
fn add(total: &mut i64, value: i64) -> Result<()> {
    if value < 0 {
        return Err(Error::Invalid("negative stored usage"));
    }
    *total = total.checked_add(value).ok_or(Error::IntegerRange)?;
    Ok(())
}
