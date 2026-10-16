#![cfg(feature = "native")]
//! Qualification of private list SQL against the unchanged native repository.
//! This is not a personal read route or a permission/bundle-proof implementation.
#[path = "../../foks-server-db/tests/common/mod.rs"]
mod common;
use foks_metadata_core::{
    native::NativeSql,
    page::{Cell, PageKind},
    page_catalog::PageStatement,
    Sql, Statement, Value,
};
use foks_proto::{
    KvDirectory, KvDirectoryStatus, KvDirent, KvNodeId, KvSmallFileBox, Role, RoleAndGeneration,
    SecretBox,
};
use foks_server_db::{KvDirectoryMutation, KvListCursor, KvNodeMutation, ReadDatabase};
use rusqlite::{
    params,
    types::{Value as SqlValue, ValueRef},
    Connection, StatementStatus,
};
use serde::Serialize;

const UID: [u8; 33] = [1; 33];
const ROOT: [u8; 16] = [0x51; 16];
const BASE_TIME: u64 = 9_007_199_254_740_993;
fn id(n: u64) -> [u8; 16] {
    let mut id = [0; 16];
    id[8..].copy_from_slice(&n.to_be_bytes());
    id
}
fn mac(n: u64) -> [u8; 32] {
    let mut m = [0; 32];
    m[24..].copy_from_slice(&n.to_be_bytes());
    m
}
fn boxed(size: usize) -> Vec<u8> {
    KvSmallFileBox {
        key: RoleAndGeneration {
            role: Role::OWNER,
            generation: 1,
        },
        ciphertext: vec![7; size],
    }
    .encode()
    .unwrap()
}
fn maximum_box() -> Vec<u8> {
    (65536 - 128..65536)
        .map(boxed)
        .find(|b| b.len() == 65536)
        .expect("exact maximum-size real small-file box")
}
fn maximum_dirent() -> KvDirent {
    (4096 - 256..4096)
        .map(|size| {
            KvDirent::new(
                ROOT,
                id(1),
                KvNodeId([3; 17]),
                10_001,
                1,
                Role::ADMIN,
                mac(7),
                SecretBox {
                    nonce: [7; 16],
                    ciphertext: vec![8; size],
                },
                KvDirectoryStatus::Active,
                [9; 32],
                BASE_TIME + 1,
            )
            .unwrap()
        })
        .find(|d| d.encode().unwrap().len() == 4096)
        .expect("exact maximum-size real dirent")
}
fn dirent(
    parent: [u8; 16],
    n: u64,
    version: u64,
    node: [u8; 17],
    name: [u8; 32],
    time: u64,
) -> KvDirent {
    KvDirent::new(
        parent,
        id(n),
        KvNodeId(node),
        version,
        1,
        Role::ADMIN,
        name,
        SecretBox {
            nonce: [7; 16],
            ciphertext: vec![8; 16],
        },
        KvDirectoryStatus::Active,
        [9; 32],
        time,
    )
    .unwrap()
}
fn insert(c: &Connection, d: &KvDirent, head: bool) {
    c.execute("INSERT INTO kv_dirents(uid,parent_id,dirent_id,version,directory_version,node_id,name_mac,creation_time,exact_dirent) VALUES(?1,?2,?3,?4,1,?5,?6,?7,?8)",params![UID,d.parent,d.id,i64::try_from(d.version).unwrap(),d.value.0,d.name_mac,i64::try_from(d.creation_time).unwrap(),d.encode().unwrap()]).unwrap();
    if head {
        c.execute(
            "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,?4)",
            params![UID, d.parent, d.id, i64::try_from(d.version).unwrap()],
        )
        .unwrap();
    }
}
fn fixture(count: u64) -> common::TestDatabase {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let exact = KvDirectory {
        id: ROOT,
        version: 1,
        key: RoleAndGeneration {
            role: Role::OWNER,
            generation: 1,
        },
        seed_ciphertext: vec![1; 48],
        write_role: Role::ADMIN,
        status: KvDirectoryStatus::Active,
    }
    .encode()
    .unwrap();
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &ROOT,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: &exact,
        })
        .unwrap();
    for kind in [3u8, 4] {
        let node = [kind; 17];
        let exact = maximum_box();
        test.database
            .put_kv_node(&KvNodeMutation {
                uid: &UID,
                id: &node,
                node_type: u64::from(kind),
                exact: &exact,
            })
            .unwrap();
    }
    let mut c = Connection::open(&test.path).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    let tx = c.transaction().unwrap();
    for n in 1..=count {
        let node = match n % 4 {
            0 => {
                let mut n = [1; 17];
                n[1..].copy_from_slice(&ROOT);
                n
            }
            1 => [3; 17],
            2 => {
                let mut n = [99; 17];
                n[0] = 3;
                n
            }
            _ => [4; 17],
        };
        if n == 1 {
            insert(&tx, &maximum_dirent(), true);
        } else {
            insert(
                &tx,
                &dirent(ROOT, n, 10_001, node, mac(n * 7 % 17), BASE_TIME + n % 5),
                true,
            );
        }
    }
    tx.commit().unwrap();
    test
}
fn bindings(values: &[Value]) -> Vec<SqlValue> {
    values
        .iter()
        .map(|v| match v {
            Value::Null => SqlValue::Null,
            Value::Integer(n) => SqlValue::Text(n.to_string()),
            Value::Blob(v) => SqlValue::Blob(v.clone()),
            Value::Text(v) => SqlValue::Text(v.clone()),
        })
        .collect()
}
fn query(c: &Connection, statement: PageStatement, values: &[Value]) -> Vec<Vec<Value>> {
    let spec = statement.spec();
    spec.validate_parameters(values).unwrap();
    let mut s = c.prepare(spec.sql).unwrap();
    let result = s
        .query_map(rusqlite::params_from_iter(bindings(values)), |row| {
            Ok(spec
                .columns
                .iter()
                .enumerate()
                .map(|(i, col)| match (row.get_ref(i).unwrap(), col.kind) {
                    (ValueRef::Null, _) if col.nullable => Value::Null,
                    (ValueRef::Text(t), PageKind::Integer) => {
                        let text = std::str::from_utf8(t).unwrap();
                        let n = text.parse::<i64>().unwrap();
                        assert_eq!(text, n.to_string());
                        Value::Integer(n)
                    }
                    (ValueRef::Text(t), PageKind::Text) => {
                        assert!(t.len() <= col.maximum_bytes as usize);
                        Value::Text(std::str::from_utf8(t).unwrap().to_owned())
                    }
                    (ValueRef::Blob(b), PageKind::Blob) => {
                        assert!(b.len() <= col.maximum_bytes as usize);
                        Value::Blob(b.to_vec())
                    }
                    (other, _) => panic!("unexpected SQL result {other:?}"),
                })
                .collect())
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(result.len() <= usize::from(spec.effective_rows().unwrap()));
    // Compare the complete real NativeSql page path with an independent SQL
    // type/length oracle. Owned row mirrors exist only in this test.
    let tx = c.unchecked_transaction().unwrap();
    let mut host = NativeSql::new(&tx).unwrap();
    let page = host.query_page(statement, values).unwrap();
    assert_eq!(host.calls(), 1);
    let decoded: Vec<Vec<Value>> = page
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
    assert_eq!(decoded, result);
    tx.rollback().unwrap();
    result
}
fn blob(value: &Value) -> &[u8] {
    let Value::Blob(b) = value else {
        panic!("expected blob")
    };
    b
}
fn integer(value: &Value) -> i64 {
    let Value::Integer(n) = value else {
        panic!("expected integer")
    };
    *n
}
fn start(cursor: Option<KvListCursor>, include: bool, limit: i64) -> (PageStatement, Vec<Value>) {
    let mut p = vec![Value::Blob(UID.to_vec()), Value::Blob(ROOT.to_vec())];
    let s = match cursor {
        None => PageStatement::ListInitial,
        Some(KvListCursor::Mac(m)) => {
            p.push(Value::Blob(m.to_vec()));
            PageStatement::ListMacStart
        }
        Some(KvListCursor::Time(t)) => {
            p.push(Value::Integer(i64::try_from(t).unwrap()));
            PageStatement::ListTimeStart
        }
    };
    p.extend([Value::Integer(i64::from(include)), Value::Integer(limit)]);
    (s, p)
}
fn continuation(
    row: &[Value],
    time: bool,
    include: bool,
    limit: i64,
) -> (PageStatement, Vec<Value>) {
    (
        if time {
            PageStatement::ListTimeContinue
        } else {
            PageStatement::ListMacContinue
        },
        vec![
            Value::Blob(UID.to_vec()),
            Value::Blob(ROOT.to_vec()),
            row[if time { 2 } else { 1 }].clone(),
            row[0].clone(),
            Value::Integer(i64::from(include)),
            Value::Integer(limit),
        ],
    )
}

#[test]
fn catalog_contract_limits_and_exact_integer_type_probe() {
    for statement in PageStatement::ALL {
        statement.spec().validate().unwrap();
        assert_eq!(PageStatement::from_id(statement as u32), Some(statement));
    }
    assert!(PageStatement::from_id(0).is_none());
    assert!(PageStatement::from_id(7).is_none());
    assert_eq!(
        PageStatement::ListInitial.spec().effective_rows().unwrap(),
        15
    );
    let (s, mut p) = start(None, false, 15);
    assert!(s.spec().validate_parameters(&p).is_ok());
    p[3] = Value::Integer(16);
    assert!(s.spec().validate_parameters(&p).is_err());
    p[3] = Value::Integer(0);
    assert!(s.spec().validate_parameters(&p).is_err());
    p[3] = Value::Integer(15);
    p[2] = Value::Integer(2);
    assert!(s.spec().validate_parameters(&p).is_err());
    let probe = PageStatement::ProbeTypes.spec();
    let mut boundary = vec![
        Value::Integer(64),
        Value::Integer(i64::MAX),
        Value::Blob(vec![0; 64]),
        Value::Text("é".repeat(32)),
        Value::Null,
        Value::Integer(64),
    ];
    assert!(probe.validate_parameters(&boundary).is_ok());
    boundary[3] = Value::Text(format!("{}x", "é".repeat(32)));
    assert!(probe.validate_parameters(&boundary).is_err());
    boundary[3] = Value::Text(String::new());
    boundary[0] = Value::Integer(65);
    assert!(probe.validate_parameters(&boundary).is_err());
    let c = Connection::open_in_memory().unwrap();
    for count in [0, 1, 63, 64] {
        for n in [i64::MIN, -1, 0, 9_007_199_254_740_993, i64::MAX] {
            let p = vec![
                Value::Integer(count),
                Value::Integer(n),
                Value::Blob(vec![]),
                Value::Text("é🙂".into()),
                Value::Null,
                Value::Integer(64),
            ];
            let rows = query(&c, PageStatement::ProbeTypes, &p);
            assert_eq!(rows.len(), count as usize);
            for (index, row) in rows.into_iter().enumerate() {
                assert_eq!(
                    row,
                    vec![
                        Value::Integer(index as i64),
                        Value::Integer(n),
                        Value::Blob(vec![]),
                        Value::Text("é🙂".into()),
                        Value::Null
                    ]
                );
            }
        }
    }
}

#[test]
fn private_pages_preserve_native_mac_time_ties_exact_nodes_and_missing_optional_bodies() {
    for count in [0, 1, 15, 16, 64, 65] {
        let test = fixture(count);
        let c = Connection::open(&test.path).unwrap();
        let reader = ReadDatabase::open(&test.path, foks_server_db::Config::default()).unwrap();
        for include in [false, true] {
            for cursor in [
                None,
                Some(KvListCursor::Mac(mac(3))),
                Some(KvListCursor::Time(BASE_TIME + 2)),
            ] {
                let time = matches!(cursor, Some(KvListCursor::Time(_)));
                let native = reader.kv_list(&UID, &ROOT, cursor, 4096).unwrap();
                let limit = i64::from(PageStatement::ListInitial.spec().effective_rows().unwrap());
                let (mut statement, mut parameters) = start(cursor, include, limit);
                let mut all = Vec::new();
                loop {
                    let rows = query(&c, statement, &parameters);
                    let count = rows.len();
                    if let Some(last) = rows.last() {
                        (statement, parameters) = continuation(last, time, include, limit);
                    }
                    all.extend(rows);
                    if count < limit as usize {
                        break;
                    }
                }
                assert_eq!(all.len(), native.len());
                for (row, native) in all.iter().zip(native) {
                    assert_eq!(blob(&row[6]), native.exact);
                    assert_eq!(blob(&row[3]), native.node_id);
                    let decoded = KvDirent::decode(&native.exact).unwrap();
                    assert_eq!(blob(&row[0]), decoded.id);
                    assert_eq!(blob(&row[1]), decoded.name_mac);
                    assert_eq!(
                        integer(&row[2]),
                        i64::try_from(decoded.creation_time).unwrap()
                    );
                    assert_eq!(row[4], Value::Text("blob".into()));
                    assert_eq!(integer(&row[5]), native.exact.len() as i64);
                    let body = if include && native.node_id[0] == 3 {
                        reader.kv_node(&UID, &native.node_id).unwrap()
                    } else {
                        None
                    };
                    if let Some(body) = body {
                        assert_eq!(row[7], Value::Integer(1));
                        assert_eq!(row[8], Value::Text("blob".into()));
                        assert_eq!(integer(&row[9]), body.exact.len() as i64);
                        assert_eq!(blob(&row[10]), body.exact);
                        assert!(
                            KvSmallFileBox::decode(blob(&row[10]))
                                .unwrap()
                                .ciphertext
                                .len()
                                > 2064
                        );
                    } else {
                        assert_eq!(
                            &row[7..],
                            &[
                                Value::Integer(0),
                                Value::Text("null".into()),
                                Value::Integer(-1),
                                Value::Null
                            ]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn projections_report_oversized_payload_without_materializing_it_or_hiding_presence() {
    let test = fixture(1);
    let c = Connection::open(&test.path).unwrap();
    // The schema itself does not impose repository byte ceilings. These raw
    // rows model a rejected legacy/corrupt profile, not an admitted writer.
    let (s, p) = start(None, true, 1);
    let boundary = query(&c, s, &p);
    assert_eq!(blob(&boundary[0][6]).len(), 4096);
    assert_eq!(blob(&boundary[0][10]).len(), 65536);
    // Wrong SQL types cannot be represented by supported writes to these STRICT
    // tables. Host codec corruption tests cover a host returning the wrong type.
    assert!(c
        .execute("UPDATE kv_dirents SET exact_dirent='not a blob'", [])
        .is_err());
    assert!(c
        .execute("UPDATE kv_nodes SET exact_node='not a blob'", [])
        .is_err());
    c.execute("UPDATE kv_dirents SET exact_dirent=zeroblob(4097)", [])
        .unwrap();
    c.execute(
        "UPDATE kv_nodes SET exact_node=zeroblob(65537),ciphertext_bytes=NULL WHERE node_id=?1",
        [[3u8; 17]],
    )
    .unwrap();
    let (s, p) = start(None, true, 1);
    let rows = query(&c, s, &p);
    let row = &rows[0];
    assert_eq!(row[4], Value::Text("blob".into()));
    assert_eq!(row[5], Value::Integer(4097));
    assert_eq!(row[6], Value::Null);
    assert_eq!(row[7], Value::Integer(1));
    assert_eq!(row[8], Value::Text("blob".into()));
    assert_eq!(row[9], Value::Integer(65537));
    assert_eq!(row[10], Value::Null);
    let (s, p) = start(None, false, 1);
    assert_eq!(query(&c, s, &p)[0][7], Value::Integer(0));
}

fn measure(c: &Connection, sql: &str, p: &[Value]) -> (i32, Vec<String>) {
    let bindings = bindings(p);
    let mut q = c.prepare(sql).unwrap();
    {
        let mut rows = q.query(rusqlite::params_from_iter(&bindings)).unwrap();
        while rows.next().unwrap().is_some() {}
    }
    let steps = q.get_status(StatementStatus::VmStep);
    let plan = c
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(rusqlite::params_from_iter(&bindings), |r| r.get(3))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    (steps, plan)
}
#[test]
fn actual_native_and_page_queries_bound_work_with_retained_history_tombstones_and_unrelated_heads()
{
    let mut expected = None;
    for extra in [0, 100, 10_000] {
        let test = fixture(64);
        let mut c = Connection::open(&test.path).unwrap();
        let tx = c.transaction().unwrap();
        for n in 1..=extra {
            insert(
                &tx,
                &dirent(ROOT, 1, n, [3; 17], mac(7), BASE_TIME + 1),
                false,
            );
            insert(
                &tx,
                &dirent(ROOT, 100_000 + n, 1, [0; 17], mac(99), BASE_TIME),
                true,
            );
            insert(
                &tx,
                &dirent(ROOT, 200_000 + n, 1, [3; 17], mac(99), BASE_TIME + 99),
                true,
            );
        }
        tx.commit().unwrap();
        let mut work = Vec::new();
        for cursor in [
            None,
            Some(KvListCursor::Mac(mac(3))),
            Some(KvListCursor::Time(BASE_TIME + 2)),
        ] {
            let (s, p) = start(cursor, true, 2);
            let result = query(&c, s, &p);
            let m = measure(&c, s.spec().sql, &p);
            assert!(m
                .1
                .iter()
                .any(|x| x.contains("kv_dirent_current_name")
                    || x.contains("kv_dirent_current_time")));
            assert!(!m.1.iter().any(|x| x.contains("TEMP B-TREE")));
            work.push(m.0);
            let (s, p) = continuation(
                result.last().unwrap(),
                matches!(cursor, Some(KvListCursor::Time(_))),
                true,
                2,
            );
            let m = measure(&c, s.spec().sql, &p);
            assert!(!m.1.iter().any(|x| x.contains("TEMP B-TREE")));
            work.push(m.0);
            let mut p = vec![Value::Blob(UID.to_vec()), Value::Blob(ROOT.to_vec())];
            let sql = match cursor {
                None => foks_server_db::KV_LIST_INITIAL_SQL,
                Some(KvListCursor::Mac(m)) => {
                    p.push(Value::Blob(m.to_vec()));
                    foks_server_db::KV_LIST_MAC_SQL
                }
                Some(KvListCursor::Time(t)) => {
                    p.push(Value::Integer(t as i64));
                    foks_server_db::KV_LIST_TIME_SQL
                }
            };
            p.push(Value::Integer(2));
            work.push(measure(&c, sql, &p).0);
        }
        eprintln!("page/native current-list extra_history_heads_tombstones={extra} VM={work:?}");
        if let Some(expected) = &expected {
            assert_eq!(&work, expected);
        } else {
            expected = Some(work);
        }
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
#[derive(Clone, Serialize)]
struct Table {
    name: String,
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}
#[derive(Clone, Serialize)]
struct Object {
    kind: String,
    sql: String,
}
fn schema(c: &Connection) -> Vec<Object> {
    c.prepare("SELECT type,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY type,name").unwrap().query_map([],|r|Ok(Object{kind:r.get(0)?,sql:r.get(1)?})).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}
fn snapshot(c: &Connection) -> Vec<Table> {
    let names:Vec<String>=c.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut query = c
                .prepare(&format!("SELECT * FROM {}", quote(&name)))
                .unwrap();
            let columns = query
                .column_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let count = columns.len();
            let mut rows = query
                .query_map([], |row| {
                    (0..count)
                        .map(|i| {
                            Ok(match row.get_ref(i)? {
                                ValueRef::Null => Value::Null,
                                ValueRef::Integer(n) => Value::Integer(n),
                                ValueRef::Text(t) => {
                                    Value::Text(std::str::from_utf8(t).unwrap().to_owned())
                                }
                                ValueRef::Blob(b) => Value::Blob(b.to_vec()),
                                ValueRef::Real(_) => panic!("unexpected native REAL"),
                            })
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort_by_cached_key(|row| serde_json::to_string(row).unwrap());
            Table {
                name,
                columns,
                rows,
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[derive(Serialize)]
struct PageVector {
    name: String,
    page: bool,
    command: String,
    before: Vec<Table>,
    after: Vec<Table>,
    outcome: String,
    reply: Option<String>,
    fail_after: usize,
    trap_after: usize,
    deferred_failure: bool,
}
fn vector(
    c: &Connection,
    name: &str,
    statement: PageStatement,
    parameters: &[Value],
) -> PageVector {
    let before = snapshot(c);
    let command =
        hex(&foks_metadata_core::page::encode_parameters(&statement.spec(), parameters).unwrap());
    let tx = c.unchecked_transaction().unwrap();
    let mut host = NativeSql::new(&tx).unwrap();
    let result = host.query_page(statement, parameters);
    assert_eq!(host.calls(), 1);
    tx.rollback().unwrap();
    let after = snapshot(c);
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    let (outcome, reply) = match result {
        Ok(page) => ("ok".into(), Some(hex(page.as_bytes()))),
        Err(e) => (e.code().into(), None),
    };
    PageVector {
        name: name.into(),
        page: true,
        command,
        before,
        after,
        outcome,
        reply,
        fail_after: 0,
        trap_after: 0,
        deferred_failure: false,
    }
}
#[test]
fn export_real_schema_binary_pages_for_native_wasm_and_celld_qualification() {
    let test = fixture(65);
    let c = Connection::open(&test.path).unwrap();
    let objects = schema(&c);
    let mut vectors = Vec::new();
    for count in [0, 1, 63, 64] {
        for (label, integer, optional) in [
            ("minimum", i64::MIN, Value::Null),
            ("precise", 9_007_199_254_740_993, Value::Blob(vec![])),
            ("maximum", i64::MAX, Value::Blob(vec![0, 255])),
        ] {
            let p = vec![
                Value::Integer(count),
                Value::Integer(integer),
                Value::Blob(vec![]),
                Value::Text(format!("\u{feff}{}x", "é".repeat(30))),
                optional,
                Value::Integer(64),
            ];
            vectors.push(vector(
                &c,
                &format!("type_probe_{count}_{label}"),
                PageStatement::ProbeTypes,
                &p,
            ));
        }
    }
    for include in [false, true] {
        for (label, cursor) in [
            ("initial", None),
            ("mac", Some(KvListCursor::Mac(mac(3)))),
            ("time", Some(KvListCursor::Time(BASE_TIME + 2))),
        ] {
            let (s, p) = start(cursor, include, 15);
            let rows = query(&c, s, &p);
            vectors.push(vector(
                &c,
                &format!("list_{label}_include_{include}"),
                s,
                &p,
            ));
            let (s, p) = continuation(
                rows.last().unwrap(),
                matches!(cursor, Some(KvListCursor::Time(_))),
                include,
                15,
            );
            vectors.push(vector(
                &c,
                &format!("list_{label}_continue_include_{include}"),
                s,
                &p,
            ));
        }
    }
    let (s, mut p) = start(None, true, 15);
    p[0] = Value::Blob(vec![99; 33]);
    vectors.push(vector(&c, "list_empty_namespace", s, &p));
    // Preserve the probe distinction on oversized legacy data; these vectors
    // qualify SQL transport, not admission or public List decoding.
    c.execute(
        "UPDATE kv_dirents SET exact_dirent=zeroblob(4097) WHERE dirent_id=?1",
        [id(1)],
    )
    .unwrap();
    c.execute(
        "UPDATE kv_nodes SET exact_node=zeroblob(65537),ciphertext_bytes=NULL WHERE node_id=?1",
        [[3u8; 17]],
    )
    .unwrap();
    let (s, p) = start(Some(KvListCursor::Mac(mac(6))), true, 15);
    vectors.push(vector(&c, "oversized_payload_probes", s, &p));
    if let Ok(directory) = std::env::var("FOKS_PAGE_VECTORS") {
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join("vectors.json"),serde_json::to_vec(&serde_json::json!({"schema_version":foks_server_db::SCHEMA_VERSION,"objects":objects,"vectors":vectors})).unwrap()).unwrap();
        std::fs::write(
            directory.join("catalog.json"),
            serde_json::to_vec(Statement::ALL.map(Statement::spec).as_slice()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            directory.join("page-catalog.json"),
            foks_metadata_core::page::catalog_artifact().unwrap(),
        )
        .unwrap();
    }
}
