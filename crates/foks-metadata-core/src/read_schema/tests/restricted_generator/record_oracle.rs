//! Development-only independent SQLite leaf-cell payload-size oracle. It reads
//! one bounded page; logical payload lengths include overflow without reading it.
use super::*;
use foks_schema_contract::storage::{
    self, Component as RuntimeComponent, Dimension, OperandSource, StorageType as RuntimeType,
    TableId, HOSTED_SMALL_V1,
};
use rusqlite::types::Value;
use std::io::{Read, Seek, SeekFrom};

fn identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn varint(bytes: &[u8]) -> u64 {
    let mut value = 0u64;
    for i in 0..9 {
        let byte = *bytes.get(i).expect("complete SQLite varint");
        if i == 8 {
            return (value << 8) | u64::from(byte);
        }
        value = (value << 7) | u64::from(byte & 127);
        if byte & 128 == 0 {
            return value;
        }
    }
    unreachable!()
}
fn payload(path: &Path, root: u32) -> Option<u64> {
    assert!(root > 0);
    let mut file = std::fs::File::open(path).unwrap();
    let mut header = [0u8; 100];
    file.read_exact(&mut header).unwrap();
    assert_eq!(&header[..16], b"SQLite format 3\0");
    let encoded = u16::from_be_bytes([header[16], header[17]]);
    let size = if encoded == 1 {
        65_536usize
    } else {
        usize::from(encoded)
    };
    assert!(size.is_power_of_two() && (512..=65_536).contains(&size));
    let mut page = vec![0u8; size];
    file.seek(SeekFrom::Start(
        u64::from(root - 1) * u64::try_from(size).unwrap(),
    ))
    .unwrap();
    file.read_exact(&mut page).unwrap();
    let start = if root == 1 { 100 } else { 0 };
    assert!(
        matches!(page[start], 0x0a | 0x0d),
        "oracle requires a leaf root"
    );
    let cells = u16::from_be_bytes([page[start + 3], page[start + 4]]);
    if cells == 0 {
        return None;
    }
    assert_eq!(
        cells, 1,
        "oracle intentionally supports one-row fixtures only"
    );
    let pointer = usize::from(u16::from_be_bytes([page[start + 8], page[start + 9]]));
    assert!(pointer >= start + 10 && pointer < size);
    Some(varint(&page[pointer..]))
}
fn dimension(value: &Value) -> Dimension {
    match value {
        Value::Null => Dimension::Null,
        Value::Integer(_) => Dimension::Integer,
        Value::Real(_) => Dimension::Real,
        Value::Blob(v) => Dimension::Blob {
            bytes: v.len() as u64,
        },
        Value::Text(v) => Dimension::TextUtf8 {
            bytes: v.len() as u64,
        },
    }
}
// Independent reference formula over public physical operands. Actual SQLite
// payload is independently decoded above, rather than derived from this formula.
fn upper(record: storage::RecordId, values: &[Value]) -> u64 {
    let operands: Vec<_> = record.operands().collect();
    9 + 9 * operands.len() as u64
        + operands
            .iter()
            .map(|v| match v.source() {
                OperandSource::Column { ordinal } => match &values[usize::from(ordinal)] {
                    Value::Null => 0,
                    Value::Integer(_) | Value::Real(_) => 8,
                    Value::Text(v) => v.len() as u64,
                    Value::Blob(v) => v.len() as u64,
                },
                OperandSource::Rowid | OperandSource::IntegerOne { .. } => 8,
                OperandSource::NodeFileSlice {
                    ordinal,
                    maximum_bytes,
                } => match &values[usize::from(ordinal)] {
                    Value::Blob(v) => v.len().saturating_sub(1).min(maximum_bytes as usize) as u64,
                    Value::Null => 0,
                    _ => panic!("expression type"),
                },
                OperandSource::ScopedHostOrEmpty { ordinal, .. } => {
                    match &values[usize::from(ordinal)] {
                        Value::Null => 0,
                        Value::Blob(v) => v.len() as u64,
                        _ => panic!("expression type"),
                    }
                }
            })
            .sum::<u64>()
}
fn reference() -> (tempfile::TempDir, Connection) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("reference.sqlite");
    let db = foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap();
    drop(db);
    let connection =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    (temp, connection)
}
fn compare_row(reference: &Connection, t: TableId, mut values: Vec<Value>) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("row.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA page_size=4096; PRAGMA encoding='UTF-8'; PRAGMA foreign_keys=OFF;")
        .unwrap();
    let ddl: String = reference
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?",
            [t.name()],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch(&ddl).unwrap();
    let mut statement=reference.prepare("SELECT sql FROM sqlite_schema WHERE type='index' AND tbl_name=? AND sql IS NOT NULL ORDER BY name").unwrap();
    let indexes = statement
        .query_map([t.name()], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    drop(statement);
    for index in indexes {
        db.execute_batch(&index).unwrap();
    }
    // Physical-layout oracle only: exact table/index DDL, no triggers or data
    // admission claim. CHECKs would reject intentionally boundary-sized IDs.
    db.execute_batch("PRAGMA ignore_check_constraints=ON")
        .unwrap();
    let columns = t
        .columns()
        .map(|c| identifier(c.name()))
        .collect::<Vec<_>>()
        .join(",");
    let parameters = std::iter::repeat_n("?", values.len())
        .collect::<Vec<_>>()
        .join(",");
    db.execute(
        &format!(
            "INSERT INTO {}({columns}) VALUES({parameters})",
            identifier(t.name())
        ),
        rusqlite::params_from_iter(&values),
    )
    .unwrap();
    db.execute_batch("PRAGMA ignore_check_constraints=OFF")
        .unwrap();
    let off: i64 = db
        .query_row("PRAGMA ignore_check_constraints", [], |r| r.get(0))
        .unwrap();
    assert_eq!(off, 0);
    // Read completed post-affinity classes, not the raw bind assumption.
    values = db
        .query_row(
            &format!("SELECT {columns} FROM {}", identifier(t.name())),
            [],
            |row| (0..values.len()).map(|n| row.get(n)).collect(),
        )
        .unwrap();
    let dimensions: Vec<_> = values.iter().map(dimension).collect();
    let checked = storage::check_utf8_row_dimensions(t, &dimensions).unwrap();
    assert_eq!(checked.table_record_bytes, upper(t.record(), &values));
    let mut max_index = 0;
    let mut roots = vec![(
        t.record(),
        db.query_row(
            "SELECT rootpage FROM sqlite_schema WHERE type='table' AND name=?",
            [t.name()],
            |r| r.get::<_, u32>(0),
        )
        .unwrap(),
        false,
    )];
    for index in t.indexes().filter(|i| !i.table_btree_alias()) {
        max_index = max_index.max(upper(index.record(), &values));
        let root = db
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE type='index' AND name=?",
                [index.name()],
                |r| r.get::<_, u32>(0),
            )
            .unwrap();
        roots.push((index.record(), root, index.partial()));
    }
    assert_eq!(checked.maximum_index_bytes, max_index);
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(db);
    for (record, root, partial) in roots {
        let actual = payload(&path, root);
        if record.name() == "foks/index/kv_dirent_file_reference" {
            let ordinal = t.columns().position(|c| c.name() == "node_id").unwrap();
            assert_eq!(
                actual.is_some(),
                matches!(&values[ordinal], Value::Blob(v) if v.first() == Some(&2))
            );
        }
        if record.name() == "foks/index/capability_key_one_active" {
            let ordinal = t.columns().position(|c| c.name() == "state").unwrap();
            assert_eq!(actual.is_some(), values[ordinal] == Value::Integer(1));
        }
        match actual {
            Some(actual) => assert!(
                actual <= upper(record, &values),
                "{} actual{actual} > upper{}",
                record.name(),
                upper(record, &values)
            ),
            None => assert!(partial, "only predicate-filtered index may be empty"),
        }
    }
}
fn values(t: TableId) -> Vec<Value> {
    t.columns()
        .map(|c| {
            if c.nullable() {
                Value::Null
            } else {
                match c.storage_type() {
                    RuntimeType::Integer => Value::Integer(1),
                    RuntimeType::Real => Value::Real(1.5),
                    RuntimeType::Blob => Value::Blob(vec![1]),
                    RuntimeType::Text => Value::Text("é\0z".into()),
                }
            }
        })
        .collect()
}
#[test]
fn actual_table_index_record_payloads_fit_compiled_complete_bounds() {
    let (_keep, reference) = reference();
    for name in [
        "kv_nodes",
        "kv_directories",
        "kv_dirents",
        "team_members",
        "sso_policy",
        "kv_dirent_current",
        "user_local_view_permissions",
        "capability_key_generations",
        "chunk_storage_binding",
    ] {
        let t = HOSTED_SMALL_V1.table(RuntimeComponent::Foks, name).unwrap();
        compare_row(&reference, t, values(t));
    }
    let t = HOSTED_SMALL_V1
        .table(RuntimeComponent::Foks, "kv_dirents")
        .unwrap();
    let mut row = values(t);
    let node = t.columns().position(|c| c.name() == "node_id").unwrap();
    // Native partial file-reference predicate: first byte2, length17.
    row[node] = Value::Blob([vec![2], vec![7; 16]].concat());
    compare_row(&reference, t, row);
    let t = HOSTED_SMALL_V1
        .table(RuntimeComponent::Foks, "team_members")
        .unwrap();
    let mut row = values(t);
    row[t
        .columns()
        .position(|c| c.name() == "scoped_host_id")
        .unwrap()] = Value::Blob(vec![2; 33]);
    compare_row(&reference, t, row);
    let t = HOSTED_SMALL_V1
        .table(RuntimeComponent::Foks, "capability_key_generations")
        .unwrap();
    let mut row = values(t);
    row[t.columns().position(|c| c.name() == "state").unwrap()] = Value::Integer(0);
    compare_row(&reference, t, row);
}
#[test]
fn large_utf8_blob_and_integer_extremes_use_logical_overflow_payload_length() {
    let (_keep, reference) = reference();
    let t = HOSTED_SMALL_V1
        .table(RuntimeComponent::Foks, "kv_nodes")
        .unwrap();
    for bytes in [0, 65_535, 65_536] {
        let mut row = values(t);
        row[3] = Value::Blob(vec![8; bytes]);
        row[2] = Value::Integer(i64::MAX);
        compare_row(&reference, t, row);
    }
    let t = HOSTED_SMALL_V1
        .table(RuntimeComponent::Foks, "kv_directories")
        .unwrap();
    for bytes in [1_048_575, 1_048_576] {
        let mut row = values(t);
        row[7] = Value::Blob(vec![8; bytes]);
        row[2] = Value::Integer(i64::MIN);
        compare_row(&reference, t, row);
    }
    let t = HOSTED_SMALL_V1
        .table(RuntimeComponent::Foks, "capability_key_generations")
        .unwrap();
    let mut row = values(t);
    row[1] = Value::Text("é\0z".repeat(262_144));
    compare_row(&reference, t, row);
}
#[test]
fn decoder_oracle_distinguishes_rowid_alias_desc_and_utf16_storage() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("aliases.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE alias(id INTEGER PRIMARY KEY,b BLOB); CREATE TABLE descending(id INTEGER PRIMARY KEY DESC,b BLOB); INSERT INTO alias VALUES(9223372036854775807,X''); INSERT INTO descending VALUES(9223372036854775807,X'');").unwrap();
    let roots: Vec<u32> = ["alias", "descending"]
        .into_iter()
        .map(|name| {
            db.query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name=?",
                [name],
                |r| r.get(0),
            )
            .unwrap()
        })
        .collect();
    drop(db);
    assert_eq!(payload(&path, roots[0]), Some(3));
    assert_eq!(payload(&path, roots[1]), Some(11));
    let path = temp.path().join("utf16.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA encoding='UTF-16le'; CREATE TABLE text_value(v TEXT); INSERT INTO text_value VALUES('abcdefghij');").unwrap();
    let encoding: String = db.query_row("PRAGMA encoding", [], |r| r.get(0)).unwrap();
    assert_ne!(encoding, HOSTED_SMALL_V1.required_encoding());
    let root: u32 = db
        .query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name='text_value'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    // Stored payload2(header)+20, not2+10 UTF8 bytes. A scanner must reject this
    // encoding before measuring text as UTF8; native SQLite accepts the file.
    assert_eq!(payload(&path, root), Some(22));
}
