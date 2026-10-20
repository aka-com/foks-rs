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
const DERIVED_TABLES: &[&str] = &[
    "kv_namespaces",
    "kv_dirent_current",
    "metadata_read_dependency",
];
/// The trigger-owned columns of each derived table whose other columns are
/// frozen in a proof.
const NAMESPACE_COUNTERS: &[&str] = &[
    "accounted_bytes",
    "accounted_objects",
    "directory_count",
    "dirent_count",
    "usage_small_count",
    "usage_small_bytes",
    "usage_large_count",
    "usage_large_bytes",
    "usage_large_chunks",
    "usage_unvalidated_small_count",
];
const READ_DEPENDENCY_COUNTERS: &[&str] = &["revision"];
/// Trigger-owned counters that count trigger firings rather than describe the
/// resulting rows: the authority's replay writes a kv_nodes row in two
/// statements, so it advances them a different number of times than the
/// native write. They are left out of the state hash ([`Schema::state_digest`]);
/// the authority checks that they advanced. Because no proof covers their
/// values, an output routed through the proof must not include them (the
/// read-dependency revision is the read coordinate of a read token). Must
/// match UNHASHED_COUNTERS in tools/foks-celld-proof/authority.mjs, which a
/// test compares.
const UNHASHED_COUNTERS: &[(&str, &str)] = &[("metadata_read_dependency", "revision")];

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

    /// The hash of `snapshot` that a proof compares: [`digest`] with the
    /// [`UNHASHED_COUNTERS`] columns set to null.
    pub fn state_digest(&self, snapshot: &Snapshot) -> Result<String> {
        let mut normalized = snapshot.clone();
        for (table, rows) in self.tables.iter().zip(&mut normalized.tables) {
            for (name, column) in UNHASHED_COUNTERS {
                if table.name != *name {
                    continue;
                }
                let index = table
                    .columns
                    .iter()
                    .position(|c| c.name == *column)
                    .ok_or_else(|| invalid("proof counter column missing"))?;
                for row in &mut rows.rows {
                    let cell = row
                        .0
                        .get_mut(index)
                        .ok_or_else(|| invalid("proof row width"))?;
                    *cell = CellValue::Null;
                }
            }
        }
        digest(&normalized)
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
                if table.name == "kv_dirent_current" {
                    // Replay only base mutations. The authority reconstructs this
                    // projection through triggers and compares the FULL after hash;
                    // accepting it as a direct write would bypass that check.
                    continue;
                }
                // Only trigger-owned counters may change: the namespace usage
                // counters, and the read-dependency revision that every base
                // write advances. Namespace ownership, party/host bindings and
                // the read-dependency incarnation are frozen in this proof; the
                // authority recomputes the counters by replaying the base rows
                // and compares the full after hash.
                let counters = if table.name == "metadata_read_dependency" {
                    READ_DEPENDENCY_COUNTERS
                } else {
                    NAMESPACE_COUNTERS
                };
                let identity = |data: &TableRows| -> Result<BTreeSet<String>> {
                    data.rows
                        .iter()
                        .map(|row| {
                            let values = table
                                .columns
                                .iter()
                                .zip(&row.0)
                                .filter(|(c, _)| !counters.contains(&c.name.as_str()))
                                .map(|(_, v)| v)
                                .collect::<Vec<_>>();
                            Ok(serde_json::to_string(&values)?)
                        })
                        .collect()
                };
                if identity(old)? != identity(new)? {
                    return Err(invalid(if table.name == "metadata_read_dependency" {
                        "proof read dependency identity changed"
                    } else {
                        "proof namespace identity changed"
                    }));
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
            before_hash: self.state_digest(before)?,
            after_hash: self.state_digest(after)?,
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
        // Deliberately malformed legacy state: snapshot import must preserve its
        // NULL derivation/error sentinel exactly. Replay of a new node separately
        // requires the shared decoder and never accepts this payload.
        connection.execute_batch("INSERT INTO kv_nodes(uid,node_id,node_type,exact_node) VALUES(zeroblob(33),X'0300000000000000000000000000000000',3,X'000102ff');").unwrap();
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
        connection.execute_batch("INSERT INTO kv_nodes(uid,node_id,node_type,exact_node) VALUES(zeroblob(33),X'0300000000000000000000000000000000',3,X'000102ff');").unwrap();
        let after = schema.snapshot(&connection).unwrap();
        let delta = schema.delta(&before, &after).unwrap();
        assert_eq!(delta.changes.len(), 1);
        assert_eq!(delta.changes[0].table, "kv_nodes");
        assert_eq!(delta.after_hash, schema.state_digest(&after).unwrap());
        for name in [
            "usage_small_count",
            "usage_small_bytes",
            "usage_large_count",
            "usage_large_bytes",
            "usage_large_chunks",
            "usage_unvalidated_small_count",
        ] {
            connection
                .execute(&format!("UPDATE kv_namespaces SET {name}={name}+1"), [])
                .unwrap();
            let forged = schema.snapshot(&connection).unwrap();
            let delta = schema.delta(&after, &forged).unwrap();
            assert!(delta.changes.is_empty(), "{name} cannot be a base write");
            assert_ne!(
                delta.before_hash, delta.after_hash,
                "{name} remains in full hash"
            );
            connection
                .execute(&format!("UPDATE kv_namespaces SET {name}={name}-1"), [])
                .unwrap();
        }
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
    fn read_dependency_revision_is_unhashed_and_its_incarnation_is_frozen() {
        let (_directory, connection, schema) = fixture();
        let before = schema.snapshot(&connection).unwrap();
        connection
            .execute(
                "UPDATE metadata_read_dependency SET revision=revision+1",
                [],
            )
            .unwrap();
        let after = schema.snapshot(&connection).unwrap();
        let delta = schema.delta(&before, &after).unwrap();
        assert!(delta.changes.is_empty());
        // The revision counts trigger firings, so the state hash leaves it out.
        assert_eq!(delta.before_hash, delta.after_hash);
        assert_ne!(digest(&before).unwrap(), digest(&after).unwrap());
        connection
            .execute(
                "UPDATE metadata_read_dependency SET incarnation=?",
                [[9u8; 32].as_slice()],
            )
            .unwrap();
        assert!(schema
            .delta(&after, &schema.snapshot(&connection).unwrap())
            .is_err());
    }

    /// Applies `delta` to a materialized `before` as the authority in
    /// tools/foks-celld-proof/authority.mjs does: a new kv_nodes row is
    /// inserted with ciphertext_bytes NULL and then updated, other rows are
    /// inserted, updated or deleted with one statement each.
    fn replay_as_authority(
        schema: &Schema,
        before: &Snapshot,
        delta: &Delta,
        path: &Path,
    ) -> (Connection, Snapshot) {
        schema.materialize(before, path).unwrap();
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        for change in &delta.changes {
            let table = schema
                .tables
                .iter()
                .find(|t| t.name == change.table)
                .unwrap();
            let values = |row: &Row| {
                row.0
                    .iter()
                    .map(CellValue::sql)
                    .collect::<Result<Vec<_>>>()
                    .unwrap()
            };
            let length_index = (change.table == "kv_nodes")
                .then(|| {
                    table
                        .columns
                        .iter()
                        .position(|c| c.name == "ciphertext_bytes")
                })
                .flatten();
            let mut written = change.new.as_ref().map(values);
            let length = match (&mut written, length_index) {
                (Some(row), Some(index)) => Some(std::mem::replace(
                    &mut row[index],
                    rusqlite::types::Value::Null,
                )),
                _ => None,
            };
            let name = identifier(&table.name).unwrap();
            let changed = match (&change.old, &written) {
                (None, Some(row)) => connection
                    .execute(
                        &format!(
                            "INSERT INTO {name} VALUES ({})",
                            vec!["?"; row.len()].join(",")
                        ),
                        params_from_iter(row),
                    )
                    .unwrap(),
                (Some(old), next) => {
                    let old = values(old);
                    let filter = table
                        .columns
                        .iter()
                        .map(|c| format!("{} IS ?", identifier(&c.name).unwrap()))
                        .collect::<Vec<_>>()
                        .join(" AND ");
                    match next {
                        Some(row) => {
                            let set = table
                                .columns
                                .iter()
                                .map(|c| format!("{}=?", identifier(&c.name).unwrap()))
                                .collect::<Vec<_>>()
                                .join(",");
                            connection
                                .execute(
                                    &format!("UPDATE {name} SET {set} WHERE {filter}"),
                                    params_from_iter(row.iter().chain(&old)),
                                )
                                .unwrap()
                        }
                        None => connection
                            .execute(
                                &format!("DELETE FROM {name} WHERE {filter}"),
                                params_from_iter(&old),
                            )
                            .unwrap(),
                    }
                }
                (None, None) => unreachable!(),
            };
            assert_eq!(changed, 1);
            if let Some(length) = length {
                let new = change.new.as_ref().unwrap();
                let column = |name: &str| {
                    new.0[table.columns.iter().position(|c| c.name == name).unwrap()]
                        .sql()
                        .unwrap()
                };
                let updated = connection
                    .execute(
                        "UPDATE kv_nodes SET ciphertext_bytes=? WHERE uid=? AND node_id=? AND exact_node=? AND ciphertext_bytes IS NULL",
                        rusqlite::params![length, column("uid"), column("node_id"), column("exact_node")],
                    )
                    .unwrap();
                assert_eq!(updated, 1);
            }
        }
        let snapshot = schema.snapshot(&connection).unwrap();
        (connection, snapshot)
    }

    fn read_dependency_revision(snapshot: &Snapshot, schema: &Schema) -> String {
        let table = schema
            .tables
            .iter()
            .position(|t| t.name == "metadata_read_dependency")
            .unwrap();
        let column = schema.tables[table]
            .columns
            .iter()
            .position(|c| c.name == "revision")
            .unwrap();
        match &snapshot.tables[table].rows[0].0[column] {
            CellValue::Integer(value) => value.clone(),
            other => panic!("revision {other:?}"),
        }
    }

    #[test]
    fn the_authority_leaves_out_the_same_counters() {
        let authority = include_str!("../../../tools/foks-celld-proof/authority.mjs");
        let line = authority
            .lines()
            .find(|line| line.starts_with("const UNHASHED_COUNTERS"))
            .expect("authority.mjs declares UNHASHED_COUNTERS");
        let declared: String = line
            .split_once('=')
            .unwrap()
            .1
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let expected = format!(
            "[{}];",
            UNHASHED_COUNTERS
                .iter()
                .map(|(table, column)| format!("[\"{table}\",\"{column}\"]"))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(declared, expected);
    }

    /// The authority accepts a delta when its replay reaches `after_hash`.
    /// The replay fires the read-dependency triggers a different number of
    /// times than the native write, which the state hash must not see.
    #[test]
    fn an_authority_style_replay_reaches_the_native_after_hash() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.sqlite");
        let mut database =
            foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;
          INSERT INTO names(normalized_name,reservation_sequence,uid) VALUES(X'70726f6f66',1,zeroblob(33));
          INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES(zeroblob(33),X'70726f6f66',X'70726f6f66',1,zeroblob(16),0);
          INSERT INTO host_metadata VALUES(1,zeroblob(33),'proof.invalid',X'00',X'00');
          INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33));
          INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'010203');
          INSERT INTO kv_directory_heads VALUES(zeroblob(33),zeroblob(16),1);
          INSERT INTO kv_roots VALUES(zeroblob(33),1,zeroblob(16),1,X'0405');").unwrap();
        let schema = Schema::export(&connection).unwrap();
        let before = schema.snapshot(&connection).unwrap();
        let exact = foks_proto::KvSmallFileBox {
            key: foks_proto::RoleAndGeneration {
                role: foks_proto::Role::OWNER,
                generation: 1,
            },
            ciphertext: vec![0xa5; 4],
        }
        .encode()
        .unwrap();
        let mut node = [0_u8; 17];
        node[0] = 3;
        database
            .put_kv_node(&foks_server_db::KvNodeMutation {
                uid: &[0; 33],
                id: &node,
                node_type: 3,
                exact: &exact,
            })
            .unwrap();
        database
            .put_kv_dirents(
                &[0; 33],
                None,
                foks_proto::Role::OWNER,
                &[foks_server_db::KvDirentMutation {
                    parent: &[0; 16],
                    id: &[0; 16],
                    version: 1,
                    directory_version: 1,
                    node_id: &node,
                    name_mac: &[0; 32],
                    creation_time: 1,
                    exact: &[7, 8, 9],
                }],
            )
            .unwrap();
        let after = schema.snapshot(&connection).unwrap();
        let delta = schema.delta(&before, &after).unwrap();
        assert!(!delta.changes.is_empty());

        let (_replayed, replayed) = replay_as_authority(
            &schema,
            &before,
            &delta,
            &directory.path().join("replay.sqlite"),
        );
        assert_eq!(schema.state_digest(&replayed).unwrap(), delta.after_hash);
        let start: i64 = read_dependency_revision(&before, &schema).parse().unwrap();
        let native: i64 = read_dependency_revision(&after, &schema).parse().unwrap();
        let replay: i64 = read_dependency_revision(&replayed, &schema)
            .parse()
            .unwrap();
        assert!(native > start && replay > start);
        // The two revisions need not be equal (the replay fires more
        // triggers), which is why the revision is not hashed; the state
        // hash above matches either way.
    }

    #[test]
    fn current_projection_is_derived_and_never_a_delta_write() {
        let (_directory, connection, schema) = fixture();
        connection
            .execute_batch(
                "INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'01')",
            )
            .unwrap();
        let before = schema.snapshot(&connection).unwrap();
        connection.execute_batch("INSERT INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),zeroblob(16),1,1,X'0300000000000000000000000000000000',zeroblob(32),1,X'01');
            INSERT INTO kv_dirent_heads VALUES(zeroblob(33),zeroblob(16),zeroblob(16),1);").unwrap();
        let after = schema.snapshot(&connection).unwrap();
        let delta = schema.delta(&before, &after).unwrap();
        assert_eq!(
            delta
                .changes
                .iter()
                .map(|c| c.table.as_str())
                .collect::<Vec<_>>(),
            vec!["kv_dirents", "kv_dirent_heads"]
        );
        assert_eq!(
            after
                .tables
                .iter()
                .find(|t| t.name == "kv_dirent_current")
                .unwrap()
                .rows
                .len(),
            1
        );
        connection
            .execute("UPDATE kv_dirent_current SET creation_time=2", [])
            .unwrap();
        let tampered = schema.snapshot(&connection).unwrap();
        let derived_only = schema.delta(&after, &tampered).unwrap();
        assert!(derived_only.changes.is_empty());
        // The Worker must reject this after-hash mismatch: no base mutations
        // could reproduce it. Its actual SQL replay is tested in authority.cases.mjs
        // (run through authority.test.mjs) and in an_authority_style_replay_reaches_the_native_after_hash.
        assert_ne!(derived_only.before_hash, derived_only.after_hash);
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
