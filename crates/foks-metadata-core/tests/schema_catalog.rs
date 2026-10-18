#![cfg(feature = "native")]
//! Private schema SQL transports values; it never grants a schema/owner proof.
#[path = "../../foks-server-db/tests/common/mod.rs"]
mod common;

use foks_metadata_core::{
    catalog::{READ_DEPENDENCY_SQL, READ_SCHEMA_COOKIE_SQL},
    native::NativeSql,
    page::Cell,
    page_catalog::{schema_objects_parameters, PageStatement, SCHEMA_OBJECTS_SQL},
    Error, Sql, Statement, Value,
};
use rusqlite::{params, Connection, StatementStatus};

fn scalar(c: &Connection, statement: Statement) -> Option<Vec<Value>> {
    let tx = c.unchecked_transaction().unwrap();
    let result = NativeSql::new(&tx).unwrap().query(statement, &[]).unwrap();
    tx.rollback().unwrap();
    result
}
fn page(c: &Connection, after: Option<(&str, &str)>, limit: u16) -> Vec<Vec<Value>> {
    let parameters = schema_objects_parameters(after, limit).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let page = NativeSql::new(&tx)
        .unwrap()
        .query_page(PageStatement::SchemaObjects, &parameters)
        .unwrap();
    let result = page
        .rows()
        .map(|row| {
            row.cells()
                .map(|cell| match cell {
                    Cell::Null => Value::Null,
                    Cell::Integer(n) => Value::Integer(n),
                    Cell::Blob(b) => Value::Blob(b.to_vec()),
                    Cell::Text(t) => Value::Text(t.to_owned()),
                })
                .collect()
        })
        .collect();
    drop(page);
    tx.rollback().unwrap();
    result
}
fn text(row: &[Value], index: usize) -> &str {
    let Value::Text(text) = &row[index] else {
        panic!("not text at {index}")
    };
    text
}
fn object(row: &[Value]) -> [String; 4] {
    std::array::from_fn(|field| {
        let offset = field * 3;
        assert_eq!(text(row, offset), "text");
        let value = text(row, offset + 2);
        assert_eq!(row[offset + 1], Value::Integer(value.len() as i64));
        value.to_owned()
    })
}
fn all(c: &Connection, limit: u16) -> Vec<[String; 4]> {
    let mut result = Vec::new();
    let mut after: Option<(String, String)> = None;
    loop {
        let rows = page(
            c,
            after
                .as_ref()
                .map(|(kind, name)| (kind.as_str(), name.as_str())),
            limit,
        );
        let short = rows.len() < usize::from(limit);
        for row in rows {
            let object = object(&row);
            let tuple = (object[0].clone(), object[1].clone());
            if let Some(prior) = &after {
                assert!(tuple > *prior);
            }
            after = Some(tuple);
            result.push(object);
        }
        if short {
            break;
        }
        assert!(result.len() <= 1024);
    }
    result
}
fn oracle(c: &Connection) -> Vec<[String; 4]> {
    c.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE substr(name,1,7) != 'sqlite_' COLLATE NOCASE ORDER BY type COLLATE BINARY,name COLLATE BINARY")
        .unwrap().query_map([], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

#[test]
fn stable_catalog_ids_and_exact_bounded_descriptors() {
    assert_eq!(Statement::PublicUsage as u32, 42);
    assert_eq!(Statement::ReadDependency as u32, 43);
    assert_eq!(Statement::ReadSchemaCookie as u32, 44);
    assert_eq!(PageStatement::DirectorySlice as u32, 8);
    assert_eq!(PageStatement::SchemaObjects as u32, 9);
    assert_eq!(Statement::ReadDependency.spec().sql, READ_DEPENDENCY_SQL);
    assert_eq!(
        Statement::ReadSchemaCookie.spec().sql,
        READ_SCHEMA_COOKIE_SQL
    );
    assert_eq!(
        READ_SCHEMA_COOKIE_SQL,
        "SELECT CAST(schema_version AS TEXT) FROM pragma_schema_version"
    );
    let spec = PageStatement::SchemaObjects.spec();
    assert_eq!(spec.sql, SCHEMA_OBJECTS_SQL);
    assert_eq!(spec.columns.len(), 12);
    assert_eq!(spec.maximum_row_bytes().unwrap(), 66_160);
    assert_eq!(spec.effective_rows().unwrap(), 15);
    assert_eq!(spec.maximum_bytes, 1_048_576);
    assert_eq!(
        schema_objects_parameters(None, 15).unwrap(),
        [
            Value::Integer(1),
            Value::Text(String::new()),
            Value::Text(String::new()),
            Value::Integer(15)
        ]
    );
    for bad in [
        Some(("", "x")),
        Some(("TABLE", "x")),
        Some(("other", "x")),
        Some(("table", "")),
    ] {
        assert!(matches!(
            schema_objects_parameters(bad, 1),
            Err(Error::Bounds)
        ));
    }
    assert!(schema_objects_parameters(Some(("table", &"é".repeat(129))), 1).is_err());
    assert!(schema_objects_parameters(None, 0).is_err());
    assert!(schema_objects_parameters(None, 16).is_err());
    for kind in ["table", "index", "trigger", "view"] {
        assert_eq!(
            schema_objects_parameters(Some((kind, "x")), 1).unwrap()[0],
            Value::Integer(0)
        );
    }
}

#[test]
fn dependency_point_matches_admitted_native_coordinate_and_does_not_copy_bad_blobs() {
    let fixture = common::TestDatabase::new();
    let c = Connection::open(&fixture.path).unwrap();
    let expected = fixture.database.read_dependency().unwrap();
    assert_eq!(
        scalar(&c, Statement::ReadDependency).unwrap(),
        vec![
            Value::Text("blob".into()),
            Value::Integer(32),
            Value::Blob(expected.incarnation.to_vec()),
            Value::Text("integer".into()),
            Value::Integer(i64::try_from(expected.revision).unwrap())
        ]
    );
    // Intentionally non-STRICT fixture isolates malformed legacy projection;
    // ordinary admitted schema59 cannot contain these invalid rows.
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE metadata_read_dependency(singleton,incarnation,revision)")
        .unwrap();
    assert_eq!(scalar(&c, Statement::ReadDependency), None);
    for (blob, revision, kind, length, value, revision_kind, revision_value) in [
        (
            rusqlite::types::Value::Blob(vec![0; 32]),
            rusqlite::types::Value::Integer(0),
            "blob",
            32,
            Value::Blob(vec![0; 32]),
            "integer",
            0,
        ),
        (
            rusqlite::types::Value::Blob(vec![1; 1024 * 1024]),
            rusqlite::types::Value::Integer(i64::MAX),
            "blob",
            1024 * 1024,
            Value::Null,
            "integer",
            i64::MAX,
        ),
        (
            rusqlite::types::Value::Text("wrong".into()),
            rusqlite::types::Value::Text("1".into()),
            "text",
            -1,
            Value::Null,
            "text",
            0,
        ),
        (
            rusqlite::types::Value::Null,
            rusqlite::types::Value::Null,
            "null",
            -1,
            Value::Null,
            "null",
            0,
        ),
        (
            rusqlite::types::Value::Integer(32),
            rusqlite::types::Value::Real(1.5),
            "integer",
            -1,
            Value::Null,
            "real",
            0,
        ),
    ] {
        c.execute("DELETE FROM metadata_read_dependency", [])
            .unwrap();
        c.execute(
            "INSERT INTO metadata_read_dependency VALUES(1,?1,?2)",
            params![blob, revision],
        )
        .unwrap();
        assert_eq!(
            scalar(&c, Statement::ReadDependency).unwrap(),
            vec![
                Value::Text(kind.into()),
                Value::Integer(length),
                value,
                Value::Text(revision_kind.into()),
                Value::Integer(revision_value)
            ]
        );
    }
}

#[test]
fn exact_cookie_statement_observes_commit_and_rollback_without_writes() {
    let c = Connection::open_in_memory().unwrap();
    let before = scalar(&c, Statement::ReadSchemaCookie).unwrap();
    assert_eq!(scalar(&c, Statement::ReadSchemaCookie).unwrap(), before);
    let tx = c.unchecked_transaction().unwrap();
    tx.execute_batch("CREATE TABLE probe(value INTEGER)")
        .unwrap();
    let during = NativeSql::new(&tx)
        .unwrap()
        .query(Statement::ReadSchemaCookie, &[])
        .unwrap()
        .unwrap();
    assert_ne!(during, before);
    tx.rollback().unwrap();
    assert_eq!(scalar(&c, Statement::ReadSchemaCookie).unwrap(), before);
    c.execute_batch("CREATE TABLE probe(value INTEGER)")
        .unwrap();
    assert_eq!(scalar(&c, Statement::ReadSchemaCookie).unwrap(), during);
    c.execute_batch("DROP TABLE probe").unwrap();
    assert_ne!(scalar(&c, Statement::ReadSchemaCookie).unwrap(), during);
}

#[test]
fn schema_keyset_is_complete_for_page_boundaries_and_literal_internal_prefix() {
    for count in [0, 1, 14, 15, 16, 30, 31, 64, 65] {
        let c = Connection::open_in_memory().unwrap();
        for n in 0..count {
            c.execute_batch(&format!("CREATE TABLE item_{n:03}(value TEXT)"))
                .unwrap();
        }
        assert_eq!(all(&c, 15), oracle(&c));
        assert_eq!(all(&c, 1).len(), count);
    }
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE sqliteX_extension(v TEXT DEFAULT 'Case Sensitive'); CREATE TABLE \"\u{feff}é\"(v TEXT); CREATE INDEX index_a ON sqliteX_extension(v); CREATE VIEW view_a AS SELECT v FROM sqliteX_extension; CREATE TRIGGER SQLITEZ_trigger AFTER INSERT ON sqliteX_extension BEGIN SELECT 'A B'; END;").unwrap();
    let actual = all(&c, 2);
    assert_eq!(actual, oracle(&c));
    assert!(actual.iter().any(|row| row[1] == "sqliteX_extension"));
    assert!(actual.iter().any(|row| row[1] == "SQLITEZ_trigger"));
    assert!(actual.iter().any(|row| row[3].contains("'Case Sensitive'")));
    assert!(actual.iter().any(|row| row[1] == "\u{feff}é"));
}

#[test]
fn schema_utf8_and_sql_boundaries_preserve_probes_and_suppress_before_copy() {
    let c = Connection::open_in_memory().unwrap();
    for bytes in [256, 258] {
        let name = "é".repeat(bytes / 2);
        c.execute_batch(&format!("CREATE TABLE \"{name}\"(v TEXT)"))
            .unwrap();
    }
    for bytes in [65536, 65537] {
        let prefix = format!("CREATE TABLE sql_{bytes}(v TEXT DEFAULT '");
        let suffix = "')";
        let ddl = prefix.clone() + &"x".repeat(bytes - prefix.len() - suffix.len()) + suffix;
        assert_eq!(ddl.len(), bytes);
        c.execute_batch(&ddl).unwrap();
    }
    let rows = page(&c, None, 15);
    assert_eq!(rows.len(), 4);
    let maximum = rows.iter().find(|r| r[4] == Value::Integer(256)).unwrap();
    assert_eq!(text(maximum, 5).len(), 256);
    let oversized = rows.iter().find(|r| r[4] == Value::Integer(258)).unwrap();
    assert_eq!(oversized[5], Value::Null);
    assert_eq!(oversized[8], Value::Null); // tbl_name is independently guarded.
    let maximum = rows
        .iter()
        .find(|r| r[10] == Value::Integer(65536))
        .unwrap();
    assert_eq!(text(maximum, 11).len(), 65536);
    let oversized = rows
        .iter()
        .find(|r| r[10] == Value::Integer(65537))
        .unwrap();
    assert_eq!(oversized[11], Value::Null);
}

#[test]
fn unexpected_null_or_blob_sql_is_visible_not_filtered_as_eof() {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE a(v TEXT); CREATE TABLE b(v TEXT); PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=NULL WHERE name='a'; UPDATE sqlite_schema SET sql=X'01' WHERE name='b';").unwrap();
    let rows = page(&c, None, 15);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0][9..],
        [Value::Text("null".into()), Value::Integer(-1), Value::Null]
    );
    assert_eq!(
        rows[1][9..],
        [Value::Text("blob".into()), Value::Integer(-1), Value::Null]
    );
}

#[test]
fn malformed_null_and_reserved_looking_blob_names_remain_visible() {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE a(v TEXT); CREATE TABLE b(v TEXT); CREATE TABLE c(v TEXT); PRAGMA writable_schema=ON; UPDATE sqlite_schema SET name=NULL WHERE name='a'; UPDATE sqlite_schema SET name=CAST('sqlite_hidden' AS BLOB) WHERE name='b'; UPDATE sqlite_schema SET name=X'0102' WHERE name='c';").unwrap();
    let rows = page(&c, None, 15);
    assert_eq!(
        rows.len(),
        3,
        "malformed names must not be mistaken for EOF"
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r[3] == Value::Text("null".into()))
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r[3] == Value::Text("blob".into()))
            .count(),
        2
    );
    for row in rows {
        assert_eq!(row[4], Value::Integer(-1));
        assert_eq!(row[5], Value::Null);
        assert_eq!(text(&row, 0), "text");
        assert_eq!(text(&row, 2), "table");
    }
}

#[test]
fn full_native_schema_enumerates_exactly_and_reports_activation_cost() {
    let fixture = common::TestDatabase::new();
    let c = Connection::open(&fixture.path).unwrap();
    let actual = all(&c, 15);
    assert_eq!(actual, oracle(&c));
    assert!(actual.len() <= 1024);
    assert!(actual.iter().map(|row| row[3].len()).sum::<usize>() <= 2 * 1024 * 1024);
    let mut statement = c.prepare(SCHEMA_OBJECTS_SQL).unwrap();
    let mut rows = statement.query(params!["1", "", "", "15"]).unwrap();
    while rows.next().unwrap().is_some() {}
    drop(rows);
    let first = statement.get_status(StatementStatus::VmStep);
    statement.reset_status(StatementStatus::VmStep);
    let last = actual.last().unwrap();
    let mut rows = statement
        .query(params!["0", last[0], last[1], "15"])
        .unwrap();
    assert!(rows.next().unwrap().is_none());
    drop(rows);
    let last = statement.get_status(StatementStatus::VmStep);
    eprintln!(
        "schema activation objects={} first_page_vm_steps={first} after_last_vm_steps={last}",
        actual.len()
    );
    // sqlite_schema scanning/sorting is activation cost, not a constant-work claim.
}
