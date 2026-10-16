use super::{capture::Package, Control, Result, ScanError, StorageSummary, MAX_KEYS};
use foks_schema_contract::{
    is_sqlite_internal_name,
    storage::{self, Component, Dimension, OperandSource, StorageType, TableId, HOSTED_SMALL_V1},
    Contract, ContractObject,
};
use rusqlite::{
    config::DbConfig,
    limits::Limit,
    types::{Value, ValueRef},
    Connection, OpenFlags,
};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

const SCHEMA_HASH: &str = "444fc934a1336a967db1d89e078632183c2c5b001adb422df707a6439b5edb3b";
const LOCATOR_BYTES: usize = 2 * 1024 * 1024;
fn unsupported(reason: &'static str) -> ScanError {
    ScanError::Unsupported(reason)
}
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn bounded_text(row: &rusqlite::Row<'_>, column: usize, maximum: usize) -> Result<String> {
    let ValueRef::Text(bytes) = row.get_ref(column)? else {
        return Err(unsupported("schema/ledger text type"));
    };
    if bytes.len() > maximum {
        return Err(unsupported("schema/ledger text size"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| unsupported("invalid UTF-8"))?;
    let mut owned = String::new();
    owned
        .try_reserve_exact(text.len())
        .map_err(|_| ScanError::Allocation)?;
    owned.push_str(text);
    Ok(owned)
}
pub(super) struct Database {
    connection: Connection,
    control: Arc<Control>,
}
#[derive(Default)]
pub(super) struct Cursor {
    table: usize,
    key: Vec<Value>,
    companions: Companions,
    audit_max: Option<i64>,
}
impl Database {
    pub fn open(path: PathBuf, control: Arc<Control>) -> Result<Self> {
        control.check()?;
        if path.to_str().is_none() {
            return Err(unsupported("non-UTF8 private SQLite path"));
        }
        let mut uri =
            url::Url::from_file_path(&path).map_err(|_| unsupported("private SQLite URI"))?;
        uri.query_pairs_mut().append_pair("immutable", "1");
        let connection = Connection::open_with_flags(
            uri.as_str(),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // These C API settings do not prepare SQL or trust source DDL. Extension
        // loading is never enabled and no functions/modules are registered.
        connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
        for (limit, value) in [
            (Limit::SQLITE_LIMIT_LENGTH, 2_200_000),
            (Limit::SQLITE_LIMIT_SQL_LENGTH, 65536),
            (Limit::SQLITE_LIMIT_COLUMN, 128),
            (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 64),
            (Limit::SQLITE_LIMIT_ATTACHED, 0),
            (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
        ] {
            connection.set_limit(limit, value)?;
            if connection.limit(limit)? > value {
                return Err(unsupported("SQLite limit unavailable"));
            }
        }
        let hook = control.clone();
        connection.progress_handler(1000, Some(move || hook.progress()))?;
        control.statement()?;
        connection.pragma_update(None, "query_only", true)?;
        control.statement()?;
        let query_only: i64 = connection.pragma_query_value(None, "query_only", |r| r.get(0))?;
        if query_only != 1
            || !connection.db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)?
            || connection.db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA)?
        {
            return Err(unsupported("SQLite defensive settings"));
        }
        control.check()?;
        Ok(Self {
            connection,
            control,
        })
    }
    pub fn verify_schema(&self) -> Result<()> {
        self.control.statement()?;
        let version: i64 = self
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        self.control.statement()?;
        let encoding: String = self
            .connection
            .pragma_query_value(None, "encoding", |r| r.get(0))?;
        if version != 59 || encoding != "UTF-8" {
            return Err(unsupported("schema version or encoding"));
        }
        self.control.statement()?;
        let extras: i64 = self.connection.query_row("SELECT (SELECT count(*) FROM pragma_database_list WHERE name NOT IN ('main','temp')) + (SELECT count(*) FROM temp.sqlite_schema)", [], |r|r.get(0))?;
        if extras != 0 {
            return Err(unsupported("TEMP or attached schema"));
        }
        // Guard every payload in SQL before rusqlite can materialize it. Wrong
        // types, including NULL names, are never hidden by a WHERE predicate.
        let sql = "SELECT typeof(type),length(CAST(type AS BLOB)),CASE WHEN typeof(type)='text' AND length(CAST(type AS BLOB))<=8 THEN type END,typeof(name),length(CAST(name AS BLOB)),CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=256 THEN name END,typeof(tbl_name),length(CAST(tbl_name AS BLOB)),CASE WHEN typeof(tbl_name)='text' AND length(CAST(tbl_name AS BLOB))<=256 THEN tbl_name END,typeof(sql),length(CAST(sql AS BLOB)),CASE WHEN typeof(sql)='text' AND length(CAST(sql AS BLOB))<=65536 THEN sql END FROM main.sqlite_schema ORDER BY type COLLATE BINARY,name COLLATE BINARY,tbl_name COLLATE BINARY LIMIT 1025";
        self.control.statement()?;
        let mut statement = self.connection.prepare(sql)?;
        let mut rows = statement.query([])?;
        let mut objects = Vec::new();
        objects
            .try_reserve_exact(1024)
            .map_err(|_| ScanError::Allocation)?;
        let mut internals = BTreeSet::new();
        let mut total = 0usize;
        let mut count = 0usize;
        while let Some(row) = rows.next()? {
            self.control.check()?;
            count += 1;
            if count > 1024 {
                return Err(unsupported("schema object count"));
            }
            let kind = bounded_text(row, 2, 8)?;
            let name = bounded_text(row, 5, 256)?;
            let table = bounded_text(row, 8, 256)?;
            let definition = match row.get_ref(11)? {
                ValueRef::Null if row.get_ref(9)? == ValueRef::Text(b"null") => None,
                _ => Some(bounded_text(row, 11, 65536)?),
            };
            if is_sqlite_internal_name(&name) {
                if name == "sqlite_sequence" {
                    if kind != "table"
                        || table != "sqlite_sequence"
                        || definition.as_deref() != Some("CREATE TABLE sqlite_sequence(name,seq)")
                    {
                        return Err(unsupported("sqlite_sequence DDL"));
                    }
                } else if !HOSTED_SMALL_V1
                    .descriptor(Component::Foks)
                    .tables()
                    .any(|t| {
                        t.name() == table
                            && t.indexes().any(|i| {
                                !i.table_btree_alias()
                                    && i.name() == name
                                    && i.sql_sha256().is_none()
                            })
                    })
                    || kind != "index"
                    || definition.is_some()
                {
                    return Err(unsupported("unknown SQLite-owned object"));
                }
                if !internals.insert(name) {
                    return Err(unsupported("duplicate SQLite-owned object"));
                }
            } else {
                let definition = definition.ok_or(unsupported("missing application DDL"))?;
                total = total
                    .checked_add(definition.len())
                    .ok_or(ScanError::Allocation)?;
                if total > 2 * 1024 * 1024 {
                    return Err(unsupported("aggregate DDL bytes"));
                }
                objects.push(ContractObject::owned(kind, name, table, definition));
            }
        }
        let expected_internal = 1 + HOSTED_SMALL_V1
            .descriptor(Component::Foks)
            .tables()
            .map(|t| {
                t.indexes()
                    .filter(|i| !i.table_btree_alias() && i.sql_sha256().is_none())
                    .count()
            })
            .sum::<usize>();
        if internals.len() != expected_internal || !internals.contains("sqlite_sequence") {
            return Err(unsupported("missing SQLite-owned object"));
        }
        // Exact canonical hash, not NativeRequired's extension-permitting policy.
        let contract = Contract::new(objects)?;
        let observed = contract
            .hash()
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        if observed != SCHEMA_HASH {
            return Err(unsupported("exact native schema mismatch"));
        }
        self.control.check()
    }
    pub fn advance(
        &self,
        cursor: &mut Cursor,
        summary: &mut StorageSummary,
        maximum: usize,
    ) -> Result<bool> {
        let tables = HOSTED_SMALL_V1.descriptor(Component::Foks);
        for _ in 0..maximum {
            self.control.check()?;
            let Some(table) = tables.tables().nth(cursor.table) else {
                return Ok(true);
            };
            let sql = row_sql(table, !cursor.key.is_empty())?;
            self.control.statement()?;
            let mut statement = self.connection.prepare(&sql)?;
            let mut rows = statement.query(rusqlite::params_from_iter(cursor.key.iter()))?;
            let Some(row) = rows.next()? else {
                cursor.table += 1;
                cursor.key.clear();
                summary.tables = summary
                    .tables
                    .checked_add(1)
                    .ok_or(unsupported("table count overflow"))?;
                continue;
            };
            let count = table.columns().len();
            let mut dimensions = [Dimension::Null; 32];
            if count > dimensions.len() {
                return Err(unsupported("compiled column bound"));
            }
            for (i, dimension) in dimensions[..count].iter_mut().enumerate() {
                let kind: &str = row
                    .get_ref(2 * i)?
                    .as_str()
                    .map_err(|_| unsupported("dimension type"))?;
                *dimension = match kind {
                    "null" => Dimension::Null,
                    "integer" => Dimension::Integer,
                    "real" => Dimension::Real,
                    "blob" | "text" => {
                        let bytes = u64::try_from(row.get::<_, i64>(2 * i + 1)?)
                            .map_err(|_| unsupported("dimension length"))?;
                        if kind == "blob" {
                            Dimension::Blob { bytes }
                        } else {
                            Dimension::TextUtf8 { bytes }
                        }
                    }
                    _ => return Err(unsupported("unknown SQLite storage class")),
                };
            }
            let bounds = storage::check_utf8_row_dimensions(table, &dimensions[..count])
                .map_err(|violation| ScanError::Row { table, violation })?;
            let next = next_key(table, row, 2 * count)?;
            if next == cursor.key {
                return Err(unsupported("nonadvancing row locator"));
            }
            if table.name() == "kv_file_chunks" {
                let external = table.columns().enumerate().any(|(ordinal, column)| {
                    matches!(
                        column.name(),
                        "external_object_id" | "external_digest" | "external_encoded_bytes"
                    ) && dimensions[ordinal] != Dimension::Null
                });
                if external {
                    summary.external_chunk_references = summary
                        .external_chunk_references
                        .checked_add(1)
                        .ok_or(unsupported("row count overflow"))?;
                }
            }
            summary.rows = summary
                .rows
                .checked_add(1)
                .ok_or(unsupported("row count overflow"))?;
            summary.maximum_table_bytes =
                summary.maximum_table_bytes.max(bounds.table_record_bytes);
            summary.maximum_index_bytes =
                summary.maximum_index_bytes.max(bounds.maximum_index_bytes);
            cursor
                .companions
                .observe(table, row, 2 * count + next.len())?;
            if table.name() == "admin_audit" {
                let Value::Integer(id) = next[0] else {
                    return Err(unsupported("audit locator"));
                };
                cursor.audit_max = Some(cursor.audit_max.map_or(id, |prior| prior.max(id)));
            }
            cursor.key = next;
        }
        self.control.check()?;
        Ok(cursor.table == tables.tables().len())
    }
    pub fn sequence(&self, cursor: &Cursor) -> Result<Option<i64>> {
        self.control.statement()?;
        let mut stmt = self.connection.prepare("SELECT CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))=11 THEN name END,seq FROM main.sqlite_sequence LIMIT 2")?;
        let mut rows = stmt.query([])?;
        let value = if let Some(row) = rows.next()? {
            if bounded_text(row, 0, 11)? != "admin_audit" {
                return Err(unsupported("SQLite sequence name"));
            }
            let seq: i64 = row.get(1)?;
            if seq < 0 {
                return Err(unsupported("negative SQLite sequence"));
            }
            if rows.next()?.is_some() {
                return Err(unsupported("extra SQLite sequence"));
            }
            Some(seq)
        } else {
            None
        };
        if cursor
            .audit_max
            .is_some_and(|maximum| value.is_none_or(|v| v < maximum.max(0)))
        {
            return Err(unsupported("SQLite sequence below audit high-water"));
        }
        // 9 + 9*2 + 11 + 8 = 46 bytes, separately admitted internal record.
        self.control.check()?;
        Ok(value)
    }
    pub fn verify_companions(&self, package: &Package, cursor: &Cursor) -> Result<()> {
        self.control.check()?;
        cursor.companions.verify(package)?;
        self.control.check()
    }
}

#[derive(Default)]
struct Companions {
    names: BTreeSet<String>,
    host_genesis: Option<[u8; 16]>,
    capability_genesis: Option<[u8; 16]>,
    capability_active: bool,
    host_active: Option<[u8; 16]>,
    host_staged: Option<[u8; 16]>,
    host_retiring: Option<[u8; 16]>,
    rotation: Option<([u8; 16], [u8; 16], i64)>,
    manifest: Option<Vec<u8>>,
}
fn id(row: &rusqlite::Row<'_>, column: usize) -> Result<[u8; 16]> {
    row.get_ref(column)?
        .as_blob()
        .map_err(|_| unsupported("generation/rotation ID type"))?
        .try_into()
        .map_err(|_| unsupported("generation/rotation ID width"))
}
impl Companions {
    // Called once per already-admitted source row. Retired history is traversed
    // without retaining one item per row or imposing a new history cardinality cap.
    fn observe(&mut self, table: TableId, row: &rusqlite::Row<'_>, offset: usize) -> Result<()> {
        match table.name() {
            "host_key_generations" | "capability_key_generations" => {
                let host = table.name() == "host_key_generations";
                let generation = id(row, offset)?;
                let name = bounded_text(row, offset + 1, 96)?;
                let state: i64 = row.get(offset + 2)?;
                if !(1..=if host { 4 } else { 3 }).contains(&state) {
                    return Err(unsupported("generation state"));
                }
                if host {
                    let slot = match state {
                        1 => Some(&mut self.host_staged),
                        2 => Some(&mut self.host_active),
                        3 => Some(&mut self.host_retiring),
                        _ => None,
                    };
                    if let Some(slot) = slot {
                        if slot.replace(generation).is_some() {
                            return Err(unsupported("multiple host generation state"));
                        }
                    }
                } else if state == 1 && std::mem::replace(&mut self.capability_active, true) {
                    return Err(unsupported("multiple active capability generations"));
                }
                let original = if host { "host.key" } else { "capability.key" };
                if name == original {
                    let slot = if host {
                        &mut self.host_genesis
                    } else {
                        &mut self.capability_genesis
                    };
                    if slot.replace(generation).is_some() {
                        return Err(unsupported("multiple genesis generations"));
                    }
                } else if name
                    != if host {
                        crate::host::generation_file_name(generation)
                    } else {
                        crate::keys::capability_generation_file_name(generation)
                    }
                {
                    return Err(unsupported("generation filename mismatch"));
                }
                if state != if host { 4 } else { 3 } {
                    if self.names.len() >= MAX_KEYS {
                        return Err(unsupported("key companion count"));
                    }
                    if !self.names.insert(name) {
                        return Err(unsupported("duplicate generation filename"));
                    }
                }
            }
            "host_metadata" => {
                let bytes = row
                    .get_ref(offset)?
                    .as_blob()
                    .map_err(|_| unsupported("bootstrap manifest type/size"))?;
                if bytes.len() > 4096 || self.manifest.is_some() {
                    return Err(unsupported("bootstrap manifest size/cardinality"));
                }
                let mut copy = Vec::new();
                copy.try_reserve_exact(bytes.len())
                    .map_err(|_| ScanError::Allocation)?;
                copy.extend_from_slice(bytes);
                self.manifest = Some(copy);
            }
            "host_rotation_operations" => {
                let phase: i64 = row.get(offset + 2)?;
                if !(1..=3).contains(&phase) {
                    return Err(unsupported("rotation phase"));
                }
                if phase != 3 {
                    let old = id(row, offset)?;
                    let new = id(row, offset + 1)?;
                    if self.rotation.replace((old, new, phase)).is_some() {
                        return Err(unsupported("multiple active rotations"));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn verify(&self, package: &Package) -> Result<()> {
        let bytes = package.manifest()?;
        let manifest = crate::keys::KeyGenerationManifest::decode(&bytes)
            .map_err(|_| unsupported("key manifest"))?;
        if self.manifest.as_deref() != Some(bytes.as_slice()) {
            return Err(unsupported("bootstrap manifest mismatch"));
        }
        for (purpose, observed) in [
            (crate::keys::KeyPurpose::Host, self.host_genesis),
            (crate::keys::KeyPurpose::Capability, self.capability_genesis),
        ] {
            if observed.is_none() || manifest.generation(purpose).map(|v| v.as_bytes()) != observed
            {
                return Err(unsupported("genesis manifest mismatch"));
            }
        }
        if !self.capability_active || self.host_active.is_none() {
            return Err(unsupported("generation active cardinality"));
        }
        let rotation_valid = match self.rotation {
            None => self.host_staged.is_none() && self.host_retiring.is_none(),
            Some((old, new, 1)) => {
                old != new
                    && self.host_active == Some(old)
                    && self.host_staged == Some(new)
                    && self.host_retiring.is_none()
            }
            Some((old, new, 2)) => {
                old != new
                    && self.host_active == Some(new)
                    && self.host_retiring == Some(old)
                    && self.host_staged.is_none()
            }
            _ => false,
        };
        if !rotation_valid {
            return Err(unsupported("active host rotation state"));
        }
        // Five non-rotating bootstrap purpose keys plus wrapping key.
        let fixed = [
            "metadata.key",
            "merkle.key",
            "client-ca.key",
            "delegated-tls.key",
            "recovery.key",
            crate::keys::WRAPPING_KEY_FILE,
        ];
        let expected_count = self.names.len() + fixed.len();
        if expected_count > MAX_KEYS
            || package.key_names().count() != expected_count
            || !package
                .key_names()
                .all(|name| self.names.contains(name) || fixed.contains(&name))
        {
            return Err(unsupported("closed key companion set mismatch"));
        }
        Ok(())
    }
}

fn key_columns(table: TableId) -> Result<Vec<(usize, String)>> {
    let mut keys = Vec::new();
    let operands = table
        .locator()
        .primary_key()
        .ok_or(unsupported("missing WR locator"))?;
    if operands.len() > 32 {
        return Err(unsupported("compiled locator width"));
    }
    for operand in operands {
        let OperandSource::Column { ordinal } = operand.source() else {
            return Err(unsupported("WR locator source"));
        };
        let column = table
            .columns()
            .nth(ordinal as usize)
            .ok_or(unsupported("WR locator ordinal"))?;
        if operand.descending()
            || operand.collation() != "BINARY"
            || column.storage_type() != StorageType::Blob
            || column.nullable()
        {
            return Err(unsupported("unsupported WR locator layout"));
        }
        keys.push((ordinal as usize, quoted(column.name())));
    }
    Ok(keys)
}
pub(super) fn row_sql(table: TableId, continuation: bool) -> Result<String> {
    if table.columns().len() > 32 {
        return Err(unsupported("compiled column bound"));
    }
    let mut projection = table.columns().map(|c| { let name=quoted(c.name()); format!("typeof({name}),CASE WHEN typeof({name}) IN ('blob','text') THEN length(CAST({name} AS BLOB)) ELSE 0 END") }).collect::<Vec<_>>();
    let (order, predicate) = if let Some(rowid) = table.locator().rowid_spelling() {
        let name = quoted(rowid);
        projection.push(name.clone());
        (
            name.clone(),
            if continuation {
                format!(" WHERE {name}>?1")
            } else {
                String::new()
            },
        )
    } else {
        let keys = key_columns(table)?;
        let joined = keys
            .iter()
            .map(|(_, n)| n.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let mut guards = Vec::new();
        for (ordinal, name) in &keys {
            let maximum = table
                .columns()
                .nth(*ordinal)
                .and_then(|v| v.maximum_bytes())
                .ok_or(unsupported("WR locator bound"))?;
            guards.push(format!(
                "typeof({name})='blob' AND length({name})<={maximum}"
            ));
        }
        let sum = keys
            .iter()
            .map(|(_, n)| format!("length({n})"))
            .collect::<Vec<_>>()
            .join("+");
        let guard = format!("{} AND ({sum})<={LOCATOR_BYTES}", guards.join(" AND "));
        for (_, name) in &keys {
            projection.push(format!("CASE WHEN {guard} THEN {name} END"));
        }
        let parameters = (1..=keys.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        (
            joined.clone(),
            if continuation {
                format!(" WHERE ({joined})>({parameters})")
            } else {
                String::new()
            },
        )
    };
    match table.name() {
        "host_key_generations" | "capability_key_generations" => {
            projection.push("CASE WHEN typeof(generation_id)='blob' AND length(generation_id)=16 THEN generation_id END".into());
            projection.push("CASE WHEN typeof(encrypted_file_name)='text' AND length(CAST(encrypted_file_name AS BLOB))<=96 THEN encrypted_file_name END".into());
            projection.push("state".into());
        },
        "host_metadata" => projection.push("CASE WHEN typeof(key_manifest_blob)='blob' AND length(key_manifest_blob)<=4096 THEN key_manifest_blob END".into()),
        "host_rotation_operations" => {
            projection.push("CASE WHEN typeof(old_generation_id)='blob' AND length(old_generation_id)=16 THEN old_generation_id END".into());
            projection.push("CASE WHEN typeof(new_generation_id)='blob' AND length(new_generation_id)=16 THEN new_generation_id END".into());
            projection.push("phase".into());
        },
        _ => {},
    }
    let sql = format!(
        "SELECT {} FROM main.{}{predicate} ORDER BY {order} LIMIT 1",
        projection.join(","),
        quoted(table.name())
    );
    if sql.len() > 65536 {
        return Err(unsupported("compiled scan SQL bound"));
    }
    Ok(sql)
}
fn next_key(table: TableId, row: &rusqlite::Row<'_>, offset: usize) -> Result<Vec<Value>> {
    if table.locator().rowid_spelling().is_some() {
        return Ok(vec![Value::Integer(row.get(offset)?)]);
    }
    let keys = key_columns(table)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(keys.len())
        .map_err(|_| ScanError::Allocation)?;
    let mut total = 0usize;
    for i in 0..keys.len() {
        let bytes = row
            .get_ref(offset + i)?
            .as_blob()
            .map_err(|_| unsupported("suppressed WR locator"))?;
        total = total
            .checked_add(bytes.len())
            .ok_or(ScanError::Allocation)?;
        if total > LOCATOR_BYTES {
            return Err(unsupported("WR locator aggregate bytes"));
        }
        let mut copy = Vec::new();
        copy.try_reserve_exact(bytes.len())
            .map_err(|_| ScanError::Allocation)?;
        copy.extend_from_slice(bytes);
        values.push(Value::Blob(copy));
    }
    Ok(values)
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };
    #[test]
    fn configured_readonly_connection_refuses_mutation_and_latches_engine_work_stop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boundary.sqlite");
        let source = Connection::open(&path).unwrap();
        source
            .execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES(1)")
            .unwrap();
        drop(source);
        let ctl = super::super::ScanControl::new(
            foks_local_deadline::Deadline::capture(Instant::now() + Duration::from_secs(300))
                .unwrap(),
            Arc::new(AtomicBool::new(false)),
        );
        let control = Control::new(&ctl).unwrap();
        let database = Database::open(path, control.clone()).unwrap();
        assert!(database.connection.is_readonly("main").unwrap());
        assert!(database
            .connection
            .db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)
            .unwrap());
        assert!(!database
            .connection
            .db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA)
            .unwrap());
        assert_eq!(
            database
                .connection
                .pragma_query_value::<i64, _>(None, "query_only", |r| r.get(0))
                .unwrap(),
            1
        );
        assert!(database.connection.execute("DELETE FROM t", []).is_err());
        control.statement().unwrap();
        let result=database.connection.query_row("WITH RECURSIVE n(v) AS (VALUES(1) UNION ALL SELECT v+1 FROM n WHERE v<100000000) SELECT sum(v) FROM n",[],|r|r.get::<_,i64>(0));
        assert!(result.is_err());
        assert!(control.check().is_err());
        assert!(control.statement().is_err());
    }
}
