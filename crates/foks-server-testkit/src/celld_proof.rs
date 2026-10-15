//! Expensive local qualification bridge. Never a production storage backend.
//!
//! Typed snapshots and a narrow base-row delta preserve native SQLite values;
//! the accompanying Worker owns the authoritative SQL transaction and triggers.

mod bridge;
mod fixture;
pub use bridge::{BridgeMetrics, CellEndpoint, CellSnapshot, NativeBridge};
pub use fixture::{claim, CellServer, FrozenInstallation};

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::Path;

use base64::Engine as _;
use rusqlite::{params_from_iter, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ROWS: usize = 10_000;
const BASE_TABLES: &[&str] = &["kv_nodes", "kv_dirents", "kv_dirent_heads", "kv_locks"];
const DERIVED_TABLES: &[&str] = &["kv_namespaces"];

fn invalid(message: &'static str) -> Box<dyn std::error::Error + Send + Sync> {
    std::io::Error::other(message).into()
}

/// JSON input is byte bounded before parsing. Nested arrays also have hard caps.
pub fn read_json<T: serde::de::DeserializeOwned>(reader: impl Read) -> Result<T> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("celld proof JSON byte limit"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn write_json(value: &impl Serialize, mut writer: impl Write) -> Result<()> {
    let bytes = encode(value)?;
    writer.write_all(&bytes)?;
    Ok(())
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("celld proof JSON byte limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(Vec::new());
    serde_json::to_writer(&mut output, value)?;
    Ok(output.0)
}

fn bounded<'de, D, T, const N: usize>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T, const N: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Visitor<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "array with at most {N} entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = seq.next_element()? {
                if values.len() == N {
                    return Err(serde::de::Error::custom("celld proof array limit"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Visitor::<T, N>(std::marker::PhantomData))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "lowercase",
    deny_unknown_fields
)]
pub enum CellValue {
    Null,
    Integer(String),
    Text(String),
    Blob(String),
}

impl CellValue {
    fn from_sql(value: rusqlite::types::ValueRef<'_>) -> Result<Self> {
        use rusqlite::types::ValueRef;
        let b64 = base64::engine::general_purpose::STANDARD;
        Ok(match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(value) => Self::Integer(value.to_string()),
            ValueRef::Text(value) => {
                std::str::from_utf8(value)?;
                Self::Text(b64.encode(value))
            }
            ValueRef::Blob(value) => Self::Blob(b64.encode(value)),
            ValueRef::Real(_) => return Err(invalid("floating-point proof values unsupported")),
        })
    }

    fn sql(&self) -> Result<rusqlite::types::Value> {
        use rusqlite::types::Value;
        let decode = |value: &str| -> Result<Vec<u8>> {
            let b64 = base64::engine::general_purpose::STANDARD;
            let bytes = b64.decode(value)?;
            if b64.encode(&bytes) != value {
                return Err(invalid("noncanonical proof base64"));
            }
            Ok(bytes)
        };
        Ok(match self {
            Self::Null => Value::Null,
            Self::Integer(value) => {
                let number: i64 = value.parse()?;
                if number.to_string() != *value {
                    return Err(invalid("noncanonical proof integer"));
                }
                Value::Integer(number)
            }
            Self::Text(value) => Value::Text(String::from_utf8(decode(value)?)?),
            Self::Blob(value) => Value::Blob(decode(value)?),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row(#[serde(deserialize_with = "bounded::<_, _, 64>")] pub Vec<CellValue>);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    pub kind: String,
    pub primary_key: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub name: String,
    #[serde(deserialize_with = "bounded::<_, _, 64>")]
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaObject {
    pub kind: String,
    pub name: String,
    pub table: String,
    pub sql: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    pub application_id: String,
    pub version: String,
    #[serde(deserialize_with = "bounded::<_, _, 512>")]
    pub objects: Vec<SchemaObject>,
    #[serde(deserialize_with = "bounded::<_, _, 128>")]
    pub tables: Vec<Table>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableRows {
    pub name: String,
    #[serde(deserialize_with = "bounded::<_, _, 10000>")]
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_hash: String,
    #[serde(deserialize_with = "bounded::<_, _, 128>")]
    pub tables: Vec<TableRows>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub table: String,
    pub old: Option<Row>,
    pub new: Option<Row>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delta {
    pub before_hash: String,
    pub after_hash: String,
    #[serde(deserialize_with = "bounded::<_, _, 10000>")]
    pub changes: Vec<Change>,
}

pub fn digest(value: &impl Serialize) -> Result<String> {
    // JSON object field insertion order must not change an interoperability hash.
    fn canonical(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                map.sort_keys();
                for value in map.values_mut() {
                    canonical(value);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    canonical(value);
                }
            }
            _ => {}
        }
    }
    let mut value: serde_json::Value = serde_json::from_slice(&encode(value)?)?;
    canonical(&mut value);
    let bytes = encode(&value)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn identifier(name: &str) -> Result<String> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        || name.starts_with("_cf_")
        || name.starts_with("proof_")
    {
        return Err(invalid("unsupported proof SQL identifier"));
    }
    Ok(format!("\"{name}\""))
}

impl Schema {
    /// Export only an initialized FOKS schema. The deployer pins this exact
    /// manifest in the Worker bundle; network initialization cannot replace it.
    pub fn export(connection: &Connection) -> Result<Self> {
        let schema = Self::export_unchecked(connection)?;
        schema.require_native_schema()?;
        Ok(schema)
    }

    fn require_native_schema(&self) -> Result<()> {
        static NATIVE: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();
        if NATIVE.get().is_none() {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("native-schema.sqlite");
            drop(foks_server_db::Database::open(
                &path,
                foks_server_db::Config::default(),
            )?);
            let connection = Connection::open(&path)?;
            let expected = Self::export_unchecked(&connection)?;
            let _ = NATIVE.set(expected);
        }
        if NATIVE.get() != Some(self) {
            return Err(invalid(
                "proof schema is not the exact current native schema",
            ));
        }
        Ok(())
    }

    fn export_unchecked(connection: &Connection) -> Result<Self> {
        let application_id: i64 =
            connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if application_id != foks_server_db::APPLICATION_ID
            || version != foks_server_db::SCHEMA_VERSION
        {
            return Err(invalid("proof requires current FOKS schema"));
        }
        let (count,bytes):(i64,i64) = connection.query_row("SELECT count(*),coalesce(sum(length(CAST(name AS BLOB))+length(CAST(tbl_name AS BLOB))+length(CAST(sql AS BLOB))),0) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if !(0..=512).contains(&count) || !(0..=MAX_BYTES as i64).contains(&bytes) {
            return Err(invalid("proof schema byte/object limit"));
        }
        let mut query = connection.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")?;
        let objects = query
            .query_map([], |row| {
                Ok(SchemaObject {
                    kind: row.get(0)?,
                    name: row.get(1)?,
                    table: row.get(2)?,
                    sql: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut tables = Vec::new();
        for object in &objects {
            identifier(&object.name)?;
            identifier(&object.table)?;
            if !["table", "index", "trigger"].contains(&object.kind.as_str())
                || object.sql.to_uppercase().contains("VIRTUAL TABLE")
            {
                return Err(invalid("unsupported proof schema object"));
            }
            if object.kind != "table" {
                continue;
            }
            let mut query = connection.prepare(&format!(
                "PRAGMA table_xinfo({})",
                identifier(&object.name)?
            ))?;
            let mut rows = query.query([])?;
            let mut columns = Vec::new();
            while let Some(row) = rows.next()? {
                let name: String = row.get(1)?;
                identifier(&name)?;
                let kind: String = row.get(2)?;
                let hidden: i64 = row.get(6)?;
                if hidden != 0 || !["INTEGER", "TEXT", "BLOB"].contains(&kind.as_str()) {
                    return Err(invalid("unsupported proof column"));
                }
                columns.push(Column {
                    name,
                    kind,
                    primary_key: row.get(5)?,
                });
                if columns.len() > 50 {
                    return Err(invalid("proof SQL column limit"));
                }
            }
            tables.push(Table {
                name: object.name.clone(),
                columns,
            });
        }
        tables.sort_by(|a, b| a.name.cmp(&b.name));
        let schema = Self {
            application_id: application_id.to_string(),
            version: version.to_string(),
            objects,
            tables,
        };
        // Round trip through bounded parsing also enforces nested array limits.
        let _: Self = read_json(encode(&schema)?.as_slice())?;
        Ok(schema)
    }

    pub fn snapshot(&self, connection: &Connection) -> Result<Snapshot> {
        let transaction = connection.unchecked_transaction()?;
        if Self::export(&transaction)? != *self {
            return Err(invalid("proof schema drift"));
        }
        let mut tables = Vec::new();
        let mut count = 0usize;
        let mut bytes = 0usize;
        for table in &self.tables {
            let names = table
                .columns
                .iter()
                .map(|c| identifier(&c.name))
                .collect::<Result<Vec<_>>>()?
                .join(",");
            let mut query = transaction
                .prepare(&format!("SELECT {names} FROM {}", identifier(&table.name)?))?;
            let mut cursor = query.query([])?;
            let mut rows = Vec::new();
            while let Some(row) = cursor.next()? {
                count += 1;
                if count > MAX_ROWS {
                    return Err(invalid("proof snapshot row limit"));
                }
                let mut values = Vec::new();
                for index in 0..table.columns.len() {
                    // Ref lengths are checked before copying/encoding large values.
                    let value = row.get_ref(index)?;
                    let size = match value {
                        rusqlite::types::ValueRef::Blob(v) | rusqlite::types::ValueRef::Text(v) => {
                            v.len().saturating_mul(2).saturating_add(64)
                        }
                        _ => 64,
                    };
                    bytes = bytes
                        .checked_add(size)
                        .ok_or_else(|| invalid("proof size overflow"))?;
                    if bytes > MAX_BYTES {
                        return Err(invalid("proof snapshot byte limit"));
                    }
                    values.push(CellValue::from_sql(value)?);
                }
                rows.push(Row(values));
            }
            rows.sort_by_cached_key(|r| serde_json::to_string(r).expect("typed row serializes"));
            tables.push(TableRows {
                name: table.name.clone(),
                rows,
            });
        }
        let snapshot = Snapshot {
            schema_hash: digest(self)?,
            tables,
        };
        self.validate_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    pub fn validate_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        self.require_native_schema()?;
        if snapshot.schema_hash != digest(self)? || snapshot.tables.len() != self.tables.len() {
            return Err(invalid("proof snapshot schema mismatch"));
        }
        let mut count = 0usize;
        for (definition, table) in self.tables.iter().zip(&snapshot.tables) {
            if definition.name != table.name {
                return Err(invalid("proof snapshot table mismatch"));
            }
            let mut previous: Option<String> = None;
            for row in &table.rows {
                count += 1;
                if count > MAX_ROWS || row.0.len() != definition.columns.len() {
                    return Err(invalid("proof snapshot row shape"));
                }
                for value in &row.0 {
                    value.sql()?;
                }
                let canonical = serde_json::to_string(row)?;
                if previous.as_ref().is_some_and(|p| p >= &canonical) {
                    return Err(invalid("unsorted or duplicate proof rows"));
                }
                previous = Some(canonical);
            }
        }
        digest(snapshot)?;
        Ok(())
    }

    /// Creates a disposable projection only. Refuses to overwrite any path.
    pub fn materialize(&self, snapshot: &Snapshot, path: &Path) -> Result<()> {
        self.validate_snapshot(snapshot)?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        // DatabasePathIdentity resolves macOS's /tmp ancestor alias while
        // rejecting symlink/hardlink leaves and checking the opened identity.
        let identity = foks_server_db::DatabasePathIdentity::existing(path)?;
        let mut connection = Connection::open_with_flags(
            identity.path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        identity.recheck()?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF;")?;
        let transaction = connection.transaction()?;
        transaction.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
        for object in self.objects.iter().filter(|o| o.kind == "table") {
            transaction.execute_batch(&object.sql)?;
        }
        for (table, data) in self.tables.iter().zip(&snapshot.tables) {
            let placeholders = vec!["?"; table.columns.len()].join(",");
            let sql = format!(
                "INSERT INTO {} VALUES ({placeholders})",
                identifier(&table.name)?
            );
            let mut query = transaction.prepare(&sql)?;
            for row in &data.rows {
                query.execute(params_from_iter(
                    row.0
                        .iter()
                        .map(CellValue::sql)
                        .collect::<Result<Vec<_>>>()?,
                ))?;
            }
        }
        for object in self.objects.iter().filter(|o| o.kind != "table") {
            transaction.execute_batch(&object.sql)?;
        }
        let broken: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
            [],
            |r| r.get(0),
        )?;
        if broken {
            return Err(invalid("proof projection foreign key violation"));
        }
        transaction.pragma_update(None, "application_id", self.application_id.parse::<i64>()?)?;
        transaction.pragma_update(None, "user_version", self.version.parse::<i64>()?)?;
        transaction.commit()?;
        if self.snapshot(&connection)? != *snapshot {
            return Err(invalid("proof projection differs"));
        }
        Ok(())
    }

    pub fn delta(&self, before: &Snapshot, after: &Snapshot) -> Result<Delta> {
        self.validate_snapshot(before)?;
        self.validate_snapshot(after)?;
        let mut changes = Vec::new();
        for (table, (old, new)) in self
            .tables
            .iter()
            .zip(before.tables.iter().zip(&after.tables))
        {
            if old == new {
                continue;
            }
            if DERIVED_TABLES.contains(&table.name.as_str()) {
                // Only trigger-owned counters may change. Namespace ownership
                // and party/host bindings are frozen in this proof.
                let identity = |data: &TableRows| -> Result<BTreeSet<String>> {
                    data.rows
                        .iter()
                        .map(|row| {
                            let values = table
                                .columns
                                .iter()
                                .zip(&row.0)
                                .filter(|(c, _)| {
                                    ![
                                        "accounted_bytes",
                                        "accounted_objects",
                                        "directory_count",
                                        "dirent_count",
                                    ]
                                    .contains(&c.name.as_str())
                                })
                                .map(|(_, v)| v)
                                .collect::<Vec<_>>();
                            Ok(serde_json::to_string(&values)?)
                        })
                        .collect()
                };
                if identity(old)? != identity(new)? {
                    return Err(invalid("proof namespace identity changed"));
                }
                continue;
            }
            if !BASE_TABLES.contains(&table.name.as_str()) {
                return Err(invalid("unexpected proof mutation table"));
            }
            let rows = |data: &TableRows| -> Result<BTreeMap<String, Row>> {
                let mut result = BTreeMap::new();
                for row in &data.rows {
                    let key = table
                        .columns
                        .iter()
                        .zip(&row.0)
                        .filter(|(c, _)| c.primary_key > 0)
                        .map(|(_, v)| v)
                        .collect::<Vec<_>>();
                    if key.is_empty() || key.iter().any(|v| matches!(v, CellValue::Null)) {
                        return Err(invalid("unsupported proof primary key"));
                    }
                    if result
                        .insert(serde_json::to_string(&key)?, row.clone())
                        .is_some()
                    {
                        return Err(invalid("duplicate proof primary key"));
                    }
                }
                Ok(result)
            };
            let old = rows(old)?;
            let new = rows(new)?;
            let keys = old.keys().chain(new.keys()).collect::<BTreeSet<_>>();
            for key in keys {
                if old.get(key) == new.get(key) {
                    continue;
                }
                if !new.contains_key(key) && table.name != "kv_locks" {
                    return Err(invalid("proof base deletion unsupported"));
                }
                changes.push(Change {
                    table: table.name.clone(),
                    old: old.get(key).cloned(),
                    new: new.get(key).cloned(),
                });
            }
        }
        changes.sort_by_key(|c| {
            BASE_TABLES
                .iter()
                .position(|t| *t == c.table)
                .expect("allowed base table")
        });
        let delta = Delta {
            before_hash: digest(before)?,
            after_hash: digest(after)?,
            changes,
        };
        digest(&delta)?;
        Ok(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Connection, Schema) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.sqlite");
        drop(foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap());
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;
          INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33));").unwrap();
        let schema = Schema::export(&connection).unwrap();
        (directory, connection, schema)
    }

    #[test]
    fn snapshot_rebuild_preserves_trigger_counts_and_exact_bytes() {
        let (directory, connection, schema) = fixture();
        connection.execute_batch("INSERT INTO kv_nodes VALUES(zeroblob(33),X'0300000000000000000000000000000000',3,X'000102ff');").unwrap();
        let snapshot = schema.snapshot(&connection).unwrap();
        let destination = directory.path().join("projection.sqlite");
        schema.materialize(&snapshot, &destination).unwrap();
        let restored = Connection::open(destination).unwrap();
        assert_eq!(schema.snapshot(&restored).unwrap(), snapshot);
        assert_eq!(
            restored
                .query_row(
                    "SELECT accounted_bytes,accounted_objects FROM kv_namespaces",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                )
                .unwrap(),
            (4, 1)
        );
        assert!(schema
            .materialize(&snapshot, &directory.path().join("projection.sqlite"))
            .is_err());
    }

    #[test]
    fn delta_replays_only_base_rows_and_rejects_namespace_identity_or_schema_changes() {
        let (_directory, connection, schema) = fixture();
        let before = schema.snapshot(&connection).unwrap();
        connection.execute_batch("INSERT INTO kv_nodes VALUES(zeroblob(33),X'0300000000000000000000000000000000',3,X'000102ff');").unwrap();
        let after = schema.snapshot(&connection).unwrap();
        let delta = schema.delta(&before, &after).unwrap();
        assert_eq!(delta.changes.len(), 1);
        assert_eq!(delta.changes[0].table, "kv_nodes");
        assert_eq!(delta.after_hash, digest(&after).unwrap());
        connection
            .execute("UPDATE kv_namespaces SET host_id=?", [[7u8; 33].as_slice()])
            .unwrap();
        assert!(schema
            .delta(&after, &schema.snapshot(&connection).unwrap())
            .is_err());
        connection
            .execute_batch("CREATE VIEW unsupported_view AS SELECT 1;")
            .unwrap();
        assert!(Schema::export(&connection).is_err());
    }

    #[test]
    fn malformed_and_over_budget_snapshots_fail_before_sql_publication() {
        let (directory, connection, schema) = fixture();
        let before = schema.snapshot(&connection).unwrap();
        let mut malformed = before.clone();
        malformed
            .tables
            .iter_mut()
            .find(|t| t.name == "kv_namespaces")
            .unwrap()
            .rows[0]
            .0[1] = CellValue::Integer("01".into());
        assert!(schema
            .materialize(&malformed, &directory.path().join("bad.sqlite"))
            .is_err());
        assert!(!directory.path().join("bad.sqlite").exists());
        assert!(read_json::<Snapshot>(std::io::repeat(b' ').take((MAX_BYTES + 1) as u64)).is_err());
        let mut malformed = before.clone();
        let table = malformed
            .tables
            .iter_mut()
            .find(|t| t.name == "kv_namespaces")
            .unwrap();
        table.rows.push(table.rows[0].clone());
        assert!(schema.validate_snapshot(&malformed).is_err());
        assert!(CellValue::Integer("9223372036854775808".into())
            .sql()
            .is_err());
        assert!(CellValue::Text("/w==".into()).sql().is_err());
    }

    #[test]
    fn canonical_hash_ignores_object_key_insertion_order_but_keeps_array_order() {
        let (directory, connection, schema) = fixture();
        let value = serde_json::to_value(&schema).unwrap();
        assert_eq!(digest(&schema).unwrap(), digest(&value).unwrap());
        assert_ne!(digest(&[1, 2]).unwrap(), digest(&[2, 1]).unwrap());
        let number = CellValue::Integer("9007199254740993".into());
        assert_eq!(
            number.sql().unwrap(),
            rusqlite::types::Value::Integer(9_007_199_254_740_993)
        );
        drop(connection);
        drop(directory);
    }
}
