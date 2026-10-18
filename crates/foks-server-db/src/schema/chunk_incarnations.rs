//! Rebuild constrained placement tables without changing any payload or charge.
use crate::{allocation::RowAllocationExt as _, Error, Result};
use rusqlite::Connection;

pub(super) fn migrate(c: &Connection) -> Result<()> {
    c.execute_batch("PRAGMA defer_foreign_keys=ON")?;
    let mut definitions = Vec::new();
    for name in ["chunk_objects", "chunk_object_attempts", "kv_file_chunks"] {
        let sql: String = c.query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
            [name],
            |r| r.budgeted_get(0),
        )?;
        let quoted = format!("CREATE TABLE \"{name}\"");
        let prefix = if sql.starts_with(&quoted) {
            quoted
        } else {
            format!("CREATE TABLE {name}")
        };
        if !sql.starts_with(&prefix) {
            return Err(Error::Invalid("unexpected chunk schema definition"));
        }
        let mut replacement = sql.replacen(&prefix, &format!("CREATE TABLE {name}_v53"), 1);
        if name == "chunk_objects" {
            replacement = replacement
                .replace("CHECK(key_format=1)", "CHECK(key_format IN(1,2))")
                .replace("CHECK(state IN(1,2,3))", "CHECK(state IN(1,2,3,4,5))");
        }
        c.execute_batch(&replacement)?;
        c.execute_batch(&format!("INSERT INTO {name}_v53 SELECT * FROM {name}"))?;
        let mut q=c.prepare("SELECT type,name,sql FROM sqlite_schema WHERE tbl_name=?1 AND type IN('index','trigger') AND sql IS NOT NULL")?;
        let rows = q.query_map([name], |r| {
            Ok((
                r.budgeted_get::<_, String>(0)?,
                r.budgeted_get::<_, String>(1)?,
                r.budgeted_get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            definitions.push(row?);
        }
    }
    c.execute_batch("DROP TABLE kv_file_chunks; DROP TABLE chunk_object_attempts; DROP TABLE chunk_objects; ALTER TABLE chunk_objects_v53 RENAME TO chunk_objects; ALTER TABLE chunk_object_attempts_v53 RENAME TO chunk_object_attempts; ALTER TABLE kv_file_chunks_v53 RENAME TO kv_file_chunks;
      ALTER TABLE chunk_objects ADD COLUMN delete_token BLOB CHECK(delete_token IS NULL OR length(delete_token)=16);
      ALTER TABLE chunk_objects ADD COLUMN delete_generation INTEGER NOT NULL DEFAULT 0 CHECK(delete_generation>=0);")?;
    for (_, name, mut sql) in definitions {
        if name == "chunk_objects_digest" {
            sql="CREATE UNIQUE INDEX chunk_objects_digest ON chunk_objects(digest) WHERE digest IS NOT NULL AND state IN(1,2,3)".into();
        }
        if name.starts_with("chunk_objects_totals_") {
            sql = sql
                .replace("NEW.state=3", "NEW.state IN(3,4)")
                .replace("OLD.state=3", "OLD.state IN(3,4)")
                .replace(
                    "objects=objects+1",
                    "objects=objects+CASE WHEN NEW.state!=5 THEN 1 ELSE 0 END",
                )
                .replace(
                    "objects=objects-1",
                    "objects=objects-CASE WHEN OLD.state!=5 THEN 1 ELSE 0 END",
                );
            if name == "chunk_objects_totals_update" {
                sql=sql.replace("UPDATE chunk_object_totals SET","UPDATE chunk_object_totals SET objects=objects-CASE WHEN OLD.state!=5 THEN 1 ELSE 0 END+CASE WHEN NEW.state!=5 THEN 1 ELSE 0 END,");
            }
        }
        c.execute_batch(&sql)?;
    }
    c.execute_batch("CREATE INDEX chunk_object_gc ON chunk_objects(state,references_count,object_id); CREATE INDEX chunk_attempt_object ON chunk_object_attempts(object_id,outcome);")?;
    let broken: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
        [],
        |r| r.budgeted_get(0),
    )?;
    if broken {
        return Err(Error::Invalid("chunk incarnation foreign keys"));
    }
    let counters_valid:bool=c.query_row("SELECT objects=(SELECT count(*) FROM chunk_objects WHERE state!=5) AND present_bytes=(SELECT coalesce(sum(encoded_bytes),0) FROM chunk_objects WHERE state IN(3,4)) AND reserved_bytes=(SELECT coalesce(sum(encoded_bytes),0) FROM chunk_objects WHERE state=1) AND uncertain_bytes=(SELECT coalesce(sum(encoded_bytes),0) FROM chunk_objects WHERE state=2) AND unresolved_attempts=(SELECT count(*) FROM chunk_object_attempts WHERE outcome IN(1,2)) FROM chunk_object_totals WHERE singleton=1",[],|r|r.budgeted_get(0))?;
    let refs_valid:bool=c.query_row("SELECT NOT EXISTS(SELECT 1 FROM chunk_objects o WHERE references_count!=(SELECT count(*) FROM kv_file_chunks c WHERE c.external_object_id=o.object_id))",[],|r|r.budgeted_get(0))?;
    if !counters_valid || !refs_valid {
        return Err(Error::Invalid("chunk migration accounting mismatch"));
    }
    // Rebuilding parents can leave deferred counters referring to the retired
    // table btrees. Validate the complete rebuilt graph explicitly, then clear
    // those counters before the caller commits this same transaction.
    c.execute_batch("PRAGMA defer_foreign_keys=OFF")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn genuine_schema_52_migration_preserves_legacy_references_attempts_and_charges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(super::super::SCHEMA).unwrap();
        c.pragma_update(None, "application_id", crate::APPLICATION_ID)
            .unwrap();
        c.pragma_update(None, "user_version", 52).unwrap();
        c.execute(
            "INSERT INTO host_metadata VALUES(1,?1,'test',X'',X'')",
            [[4u8; 33].as_slice()],
        )
        .unwrap();
        c.execute("INSERT INTO chunk_storage_binding(singleton,state,namespace_id,host_id,locator) VALUES(1,4,?1,?2,'test-store')",rusqlite::params![[5u8;16].as_slice(),[4u8;33].as_slice()]).unwrap();
        c.execute("INSERT INTO chunk_objects(object_id,namespace_id,object_key,digest,encoded_bytes,state) VALUES(?1,?2,lower(hex(?1)),?1,10,3)",rusqlite::params![[1u8;32].as_slice(),[5u8;16].as_slice()]).unwrap();
        c.execute("INSERT INTO chunk_object_attempts(attempt_id,object_id,namespace_id,outcome,created_at) VALUES(?1,?2,?3,2,1)",rusqlite::params![[1u8;16].as_slice(),[1u8;32].as_slice(),[5u8;16].as_slice()]).unwrap();
        c.execute(
            "INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(?1,1,?1)",
            [[1u8; 33].as_slice()],
        )
        .unwrap();
        c.execute("INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(?1,?2,X'',0,0)",rusqlite::params![[1u8;33].as_slice(),[2u8;16].as_slice()]).unwrap();
        c.execute("INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk,external_digest,external_ciphertext_bytes,external_encoded_bytes,external_object_id) VALUES(?1,?2,0,X'',0,X'',?3,1,10,?3)",rusqlite::params![[1u8;33].as_slice(),[2u8;16].as_slice(),[1u8;32].as_slice()]).unwrap();
        c.execute("INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk) VALUES(?1,?2,2,?3,0,?4)",rusqlite::params![[1u8;33].as_slice(),[2u8;16].as_slice(),b"inline".as_slice(),b"canonical".as_slice()]).unwrap();
        let before:(i64,i64,i64,i64)=c.query_row("SELECT accounted_bytes,accounted_objects,(SELECT chunk_count FROM kv_file_uploads),(SELECT chunk_bytes FROM kv_file_uploads) FROM kv_namespaces",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        drop(c);
        let db = crate::Database::open(&path, Default::default()).unwrap();
        let after:(i64,i64,i64,i64)=db.connection.query_row("SELECT accounted_bytes,accounted_objects,(SELECT chunk_count FROM kv_file_uploads),(SELECT chunk_bytes FROM kv_file_uploads) FROM kv_namespaces",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(before, after);
        let inline = db.next_kv_chunk(None, false).unwrap().unwrap();
        assert_eq!(inline.exact_chunk.as_deref(), Some(b"canonical".as_slice()));
        assert_eq!(
            inline.chunk.payload,
            crate::KvChunkPayload::Inline(b"inline".to_vec())
        );
        let usage = db.chunk_physical_usage().unwrap();
        assert_eq!(
            (
                usage.objects,
                usage.present_bytes,
                usage.unresolved_attempts,
                usage.orphan_objects
            ),
            (1, 10, 1, 0)
        );
        let chunk = db.next_kv_chunk(None, true).unwrap().unwrap();
        let crate::KvChunkPayload::External(reference) = chunk.chunk.payload else {
            panic!("external");
        };
        assert_eq!(reference.object.key_format, 1);
        assert_eq!(reference.object.object_id, [1; 32]);
        assert!(db.integrity_check().unwrap());
    }
    #[test]
    fn inconsistent_legacy_accounting_rolls_back_migration_without_clearing_charges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(super::super::SCHEMA).unwrap();
        c.pragma_update(None, "application_id", crate::APPLICATION_ID)
            .unwrap();
        c.pragma_update(None, "user_version", 52).unwrap();
        c.execute("UPDATE chunk_object_totals SET objects=1", [])
            .unwrap();
        drop(c);
        assert!(crate::Database::open(&path, Default::default()).is_err());
        let c = rusqlite::Connection::open(&path).unwrap();
        let version: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 52);
        let modern:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('chunk_objects') WHERE name='delete_token')",[],|r|r.get(0)).unwrap();
        assert!(!modern);
        let charged: i64 = c
            .query_row("SELECT objects FROM chunk_object_totals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(charged, 1);
    }
}
