#![cfg(feature = "native")]
//! Fixed page9 test scripts, not an activation/profile constructor.
#[path = "../../foks-server-db/tests/common/mod.rs"]
mod common;
use foks_metadata_core::{
    native::NativeSql,
    page::Cell,
    page_catalog::{schema_objects_parameters, PageStatement},
    Sql, Statement, Value,
};
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

const PLATFORM: &str =
    include_str!("../../../tools/foks-metadata-spike/celld-directory-schema.json");
const NAMES: &[&str] = &[
    "full_15",
    "full_1",
    "suffix_0",
    "suffix_1",
    "suffix_15",
    "suffix_16",
    "unicode",
    "name_256",
    "name_257",
    "sql_65536",
    "sql_65537",
    "sqliteX",
    "missing_foks",
    "changed_foks",
    "missing_authority",
    "changed_authority",
    "platform_missing",
    "platform_changed",
    "platform_extra",
];
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct Object {
    kind: String,
    name: String,
    table: String,
    sql: String,
}
fn objects(c: &Connection) -> Vec<Object> {
    c.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE typeof(name)!='text' OR substr(name,1,7)!='sqlite_' COLLATE NOCASE ORDER BY type COLLATE BINARY,name COLLATE BINARY").unwrap().query_map([], |r| Ok(Object{kind:r.get(0)?,name:r.get(1)?,table:r.get(2)?,sql:r.get(3)?})).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn long_view(n: usize) -> String {
    let first = "CREATE VIEW schema_long AS SELECT '";
    let last = "' AS v";
    format!("{first}{}{last}", "x".repeat(n - first.len() - last.len()))
}
fn mutation(name: &str) -> Option<String> {
    Some(match name {
        "unicode" => "CREATE TABLE \"\u{feff}𝄞schema\"(v TEXT DEFAULT 'A B')".into(),
        "name_256" => format!("CREATE TABLE \"{}\"(v)", "é".repeat(128)),
        "name_257" => format!("CREATE TABLE \"{}x\"(v)", "é".repeat(128)),
        "sql_65536" => long_view(65536), "sql_65537" => long_view(65537),
        "sqliteX" => "CREATE TABLE sqliteX_fixture(v)".into(),
        "missing_foks" => "DROP INDEX kv_dirent_current_name".into(),
        "changed_foks" => "DROP TRIGGER kv_current_head_insert; CREATE TRIGGER kv_current_head_insert AFTER INSERT ON kv_dirent_heads BEGIN SELECT 1; END".into(),
        "missing_authority" => "DROP INDEX metadata_terminal".into(),
        "changed_authority" => "DROP TRIGGER metadata_intent_reserve; CREATE TRIGGER metadata_intent_reserve AFTER INSERT ON metadata_intents BEGIN SELECT 1; END".into(),
        _ => return None,
    })
}
#[derive(Serialize)]
struct Page {
    command: String,
    reply: String,
    rows: usize,
    last: Option<[String; 2]>,
    valid: bool,
    sql_bytes: usize,
}
#[derive(Serialize)]
struct Case {
    name: String,
    limit: u16,
    start: Option<[String; 2]>,
    pages: Option<Vec<Page>>,
    observed: Option<Vec<Object>>,
    union_matches: bool,
    complete: bool,
}
fn valid_object(row: &[Value]) -> Option<Object> {
    let caps = [8, 256, 256, 65536];
    let mut fields = Vec::new();
    for (field, cap) in caps.into_iter().enumerate() {
        let i = field * 3;
        let (Value::Text(kind), Value::Integer(length), Value::Text(value)) =
            (&row[i], &row[i + 1], &row[i + 2])
        else {
            return None;
        };
        if kind != "text" || *length < 0 || *length as usize != value.len() || value.len() > cap {
            return None;
        }
        fields.push(value.clone());
    }
    if !matches!(fields[0].as_str(), "table" | "index" | "trigger" | "view") || fields[1].is_empty()
    {
        return None;
    }
    Some(Object {
        kind: fields[0].clone(),
        name: fields[1].clone(),
        table: fields[2].clone(),
        sql: fields[3].clone(),
    })
}
fn pages(c: &Connection, start: Option<[String; 2]>, limit: u16) -> (Vec<Page>, bool) {
    let mut after = start;
    let mut result = Vec::new();
    let mut total = 0;
    let mut sql_total = 0;
    loop {
        assert!(result.len() < 1025);
        let params = schema_objects_parameters(
            after.as_ref().map(|x| (x[0].as_str(), x[1].as_str())),
            limit,
        )
        .unwrap();
        let page = NativeSql::new(c)
            .unwrap()
            .query_page(PageStatement::SchemaObjects, &params)
            .unwrap();
        let rows: Vec<Vec<Value>> = page
            .rows()
            .map(|r| {
                r.cells()
                    .map(|cell| match cell {
                        Cell::Null => Value::Null,
                        Cell::Integer(n) => Value::Integer(n),
                        Cell::Blob(b) => Value::Blob(b.to_vec()),
                        Cell::Text(s) => Value::Text(s.to_owned()),
                    })
                    .collect()
            })
            .collect();
        let parsed: Option<Vec<Object>> = rows.iter().map(|r| valid_object(r)).collect();
        let valid = parsed.is_some();
        let last = parsed
            .as_ref()
            .and_then(|r| r.last())
            .map(|o| [o.kind.clone(), o.name.clone()]);
        let sql_bytes = rows
            .iter()
            .map(|r| match r[10] {
                Value::Integer(n) if n >= 0 => n as usize,
                _ => 0,
            })
            .sum();
        total += rows.len();
        sql_total += sql_bytes;
        assert!(total <= 1024 && sql_total <= 2 * 1024 * 1024);
        let short = rows.len() < usize::from(limit);
        result.push(Page {
            command: hex(&foks_metadata_core::page::encode_parameters(
                &PageStatement::SchemaObjects.spec(),
                &params,
            )
            .unwrap()),
            reply: hex(page.as_bytes()),
            rows: rows.len(),
            last: last.clone(),
            valid,
            sql_bytes,
        });
        if !valid {
            return (result, false);
        }
        if short {
            return (result, true);
        }
        assert!(last.is_some());
        after = last;
    }
}
#[test]
fn export_pinned_schema_union_and_native_page_scripts() {
    let db = common::TestDatabase::new();
    assert_eq!(
        hex(db.database.read_dependency_schema().contract_hash()),
        "3111af3c9542eb8845461009f55042925c0fa7d056b4d417f591042ae4074c64"
    );
    let c = Connection::open(&db.path).unwrap();
    c.execute_batch(foks_metadata_core::authority::SCHEMA)
        .unwrap();
    let seed = objects(&c);
    let platform: Vec<serde_json::Value> = serde_json::from_str(PLATFORM).unwrap();
    assert_eq!(platform.len(), 10);
    for o in &platform {
        c.execute_batch(o["sql"].as_str().unwrap()).unwrap();
    }
    let baseline = objects(&c);
    assert_eq!(baseline.len(), seed.len() + 10);
    let platform_objects: Vec<_> = baseline
        .iter()
        .filter(|o| !seed.contains(o))
        .cloned()
        .collect();
    let coordinate: (Vec<u8>, i64) = c
        .query_row(
            "SELECT incarnation,revision FROM metadata_read_dependency WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let mut cases = Vec::new();
    for name in NAMES {
        let tx = c.unchecked_transaction().unwrap();
        if let Some(sql) = mutation(name) {
            tx.execute_batch(&sql).unwrap();
        }
        let observed = objects(&tx);
        let mut expected = baseline.clone();
        match *name {
            "platform_missing" => expected.retain(|o| o.name != "_cf_WAKE_update"),
            "platform_changed" => {
                let o = expected
                    .iter_mut()
                    .find(|o| o.name == "_cf_WAKE_update")
                    .unwrap();
                o.sql = o.sql.replace("sequence=sequence+1", "sequence=sequence+2");
            }
            "platform_extra" => {
                expected.push(Object {
                    kind: "table".into(),
                    name: "_cf_fixture_unknown".into(),
                    table: "_cf_fixture_unknown".into(),
                    sql: "CREATE TABLE _cf_fixture_unknown(v)".into(),
                });
                expected.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
            }
            _ => {}
        }
        let limit = if *name == "full_1" { 1 } else { 15 };
        let start = name.strip_prefix("suffix_").map(|n| {
            let n: usize = n.parse().unwrap();
            let o = &observed[observed.len() - n - 1];
            [o.kind.clone(), o.name.clone()]
        });
        let (script, complete) = pages(&tx, start.clone(), limit);
        assert_eq!(complete, !matches!(*name, "name_257" | "sql_65537"));
        let union_matches = observed == expected;
        assert_eq!(
            union_matches,
            name.starts_with("full_") || name.starts_with("suffix_")
        );
        cases.push(Case {
            name: (*name).into(),
            limit,
            start,
            pages: if name.starts_with("platform_") {
                None
            } else {
                Some(script)
            },
            observed: (observed != baseline).then_some(observed),
            union_matches,
            complete,
        });
        tx.rollback().unwrap();
        assert_eq!(objects(&c), baseline);
    }
    let catalog = serde_json::to_vec(Statement::ALL.map(Statement::spec).as_slice()).unwrap();
    let page_catalog = foks_metadata_core::page::catalog_artifact().unwrap();
    let manifest=serde_json::to_vec(&serde_json::json!({"schema_version":foks_server_db::SCHEMA_VERSION,"platform_sha256":digest(PLATFORM.as_bytes()),"catalog_sha256":digest(&catalog),"page_catalog_sha256":digest(&page_catalog),"seed":seed,"platform":platform_objects,"baseline":baseline,"coordinate":{"incarnation":coordinate.0,"revision":coordinate.1.to_string()},"vectors":cases})).unwrap();
    assert!(
        manifest.len() <= 8 * 1024 * 1024,
        "fixture manifest {}",
        manifest.len()
    );
    if let Ok(directory) = std::env::var("FOKS_SCHEMA_ENUMERATION_VECTORS") {
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join("vectors.json"), manifest).unwrap();
        std::fs::write(directory.join("catalog.json"), catalog).unwrap();
        std::fs::write(directory.join("page-catalog.json"), page_catalog).unwrap();
    }
}
