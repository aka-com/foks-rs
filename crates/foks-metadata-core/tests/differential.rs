#![cfg(feature = "native")]

#[path = "../../foks-server-db/tests/common/mod.rs"]
mod common;

use foks_metadata_core::{
    catalog::Statement,
    native::NativeSql,
    wire::{Command, Reply},
    Error, Limits, Mutation, Sql, Value,
};
use foks_proto::{KvDirectoryStatus, KvDirent, KvNodeId, Role, SecretBox};
use foks_server_db::{
    Config, Database, KvDirectoryMutation, KvDirentMutation, KvNodeMutation, KvRootMutation,
};
use rusqlite::{types::ValueRef, Connection};
use serde::Serialize;

const UID: [u8; 33] = [1; 33];
const ROOT: [u8; 16] = [0x51; 16];
fn limits(config: &Config) -> Limits {
    Limits {
        blob_bytes: config.maximum_blob_bytes,
        dirent_bytes: config.maximum_kv_dirent_bytes,
        namespace_bytes: config.maximum_kv_namespace_bytes,
        namespace_objects: config.maximum_kv_namespace_objects,
        directories: config.maximum_kv_directories,
        dirents: config.maximum_kv_dirents,
    }
}
fn mutation(id: u8, version: u64) -> Mutation {
    let d = KvDirent::new(
        ROOT,
        [id; 16],
        KvNodeId([3; 17]),
        version,
        1,
        Role::ADMIN,
        [id; 32],
        SecretBox {
            nonce: [7; 16],
            ciphertext: vec![8; 16],
        },
        KvDirectoryStatus::Active,
        [9; 32],
        42,
    )
    .unwrap();
    Mutation {
        parent: d.parent,
        id: d.id,
        version: d.version,
        directory_version: d.directory_version,
        node_id: d.value.0,
        name_mac: d.name_mac,
        creation_time: d.creation_time,
        exact: d.encode().unwrap(),
    }
}
fn native_mutation(m: &Mutation) -> KvDirentMutation<'_> {
    KvDirentMutation {
        parent: &m.parent,
        id: &m.id,
        version: m.version,
        directory_version: m.directory_version,
        node_id: &m.node_id,
        name_mac: &m.name_mac,
        creation_time: m.creation_time,
        exact: &m.exact,
    }
}
fn fixture() -> common::TestDatabase {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &ROOT,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: b"directory",
        })
        .unwrap();
    test.database
        .put_kv_root(&KvRootMutation {
            uid: &UID,
            version: 1,
            directory_id: &ROOT,
            directory_version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            exact: b"root",
        })
        .unwrap();
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[3; 17],
            node_type: 3,
            exact: b"small file",
        })
        .unwrap();
    test
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
fn json(value: &impl Serialize) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}
fn code(error: &foks_server_db::Error) -> &'static str {
    match error {
        foks_server_db::Error::KvConflict => "conflict",
        foks_server_db::Error::KvPermission => "permission",
        foks_server_db::Error::QuotaExceeded => "quota",
        foks_server_db::Error::IntegerRange => "integer_range",
        foks_server_db::Error::Invalid(_) => "invalid",
        _ => "storage",
    }
}
#[derive(Serialize)]
struct Vector {
    name: String,
    command: String,
    before: Vec<Table>,
    after: Vec<Table>,
    outcome: String,
    reply: Option<Reply>,
    fail_after: usize,
    trap_after: usize,
    deferred_failure: bool,
}

#[test]
fn actual_initialized_schema_put_dirents_matches_existing_repository_and_exports_vectors() {
    let mut baseline = fixture();
    let mut vectors = Vec::new();
    let mut scenarios = vec![
        ("insert", vec![mutation(0x61, 1)], Role::OWNER, false, None),
        (
            "exact_replay_before_quota",
            vec![mutation(0x61, 1)],
            Role::OWNER,
            false,
            Some(1),
        ),
        (
            "overwrite",
            vec![mutation(0x61, 2)],
            Role::OWNER,
            false,
            None,
        ),
        (
            "late_batch_conflict",
            vec![mutation(0x62, 1), mutation(0x63, 9)],
            Role::OWNER,
            false,
            None,
        ),
        (
            "stored_role_denied",
            vec![mutation(0x61, 3)],
            Role::member(0),
            false,
            None,
        ),
        (
            "derived_creation_time",
            vec![mutation(0x64, 1)],
            Role::OWNER,
            true,
            None,
        ),
        (
            "derived_creation_replay",
            vec![mutation(0x64, 1)],
            Role::OWNER,
            true,
            Some(1),
        ),
        (
            "precondition_before_quota",
            vec![mutation(0x67, 1)],
            Role::OWNER,
            false,
            Some(1),
        ),
    ];
    // A conflicting live name is a negative predicate, not a point lookup.
    let mut collision = mutation(0x65, 1);
    collision.name_mac = [0x61; 32];
    scenarios.push(("duplicate_name", vec![collision], Role::OWNER, false, None));
    for (name, mut mutations, role, normalize, quota) in scenarios {
        let temp = tempfile::tempdir().unwrap();
        let copy = temp.path().join("kernel.sqlite");
        baseline.database.online_backup(&copy).unwrap();
        let mut c = Connection::open(&copy).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        let before = snapshot(&c);
        let config = Config {
            maximum_kv_namespace_bytes: quota
                .unwrap_or(Config::default().maximum_kv_namespace_bytes),
            ..Config::default()
        };
        let precondition = matches!(
            name,
            "exact_replay_before_quota" | "precondition_before_quota"
        )
        .then_some(foks_proto::KvPathVersionVector {
            root_version: 99,
            directories: vec![],
        });
        let command = serde_json::to_string(&Command::PutDirents {
            uid: UID.to_vec(),
            precondition: precondition.as_ref().map(|v| v.encode().unwrap()),
            role: role.encoded().unwrap(),
            mutations: mutations.clone(),
            limits: limits(&config),
            normalize_at: normalize.then_some(777),
        })
        .unwrap();
        let tx = c.transaction().unwrap();
        let result =
            foks_metadata_core::wire::run(&mut NativeSql::new(&tx).unwrap(), command.as_bytes());
        let outcome = result.as_ref().err().map_or("ok", Error::code).to_owned();
        let reply = match result {
            Ok(reply) => {
                tx.commit().unwrap();
                Some(reply)
            }
            Err(_) => {
                tx.rollback().unwrap();
                None
            }
        };
        if normalize {
            for m in &mut mutations {
                let mut d = KvDirent::decode(&m.exact).unwrap();
                let prior = baseline
                    .database
                    .kv_dirent(&UID, &m.parent, &m.id)
                    .unwrap()
                    .map(|v| KvDirent::decode(&v.exact).unwrap())
                    .filter(|d| d.version == m.version);
                let time = prior.map_or(777, |d| d.creation_time);
                d.set_creation_time(time).unwrap();
                m.exact = d.encode().unwrap();
                m.creation_time = time;
            }
        }
        // Existing production method is the independent oracle. Reopen with
        // pressure configuration without changing its source implementation.
        let baseline_copy = temp.path().join("oracle.sqlite");
        baseline.database.online_backup(&baseline_copy).unwrap();
        let mut oracle = Database::open(&baseline_copy, config).unwrap();
        let expected = oracle.put_kv_dirents(
            &UID,
            precondition.as_ref(),
            role,
            &mutations.iter().map(native_mutation).collect::<Vec<_>>(),
        );
        assert_eq!(
            outcome,
            expected.as_ref().err().map_or("ok", |e| code(e)),
            "{name}"
        );
        let oracle_connection = Connection::open(&baseline_copy).unwrap();
        assert_eq!(
            json(&snapshot(&c)),
            json(&snapshot(&oracle_connection)),
            "{name}"
        );
        vectors.push(Vector {
            name: name.into(),
            command,
            before,
            after: snapshot(&c),
            outcome,
            reply,
            fail_after: 0,
            trap_after: 0,
            deferred_failure: false,
        });
        if expected.is_ok() {
            baseline
                .database
                .put_kv_dirents(
                    &UID,
                    None,
                    role,
                    &mutations.iter().map(native_mutation).collect::<Vec<_>>(),
                )
                .unwrap();
        }
    }
    let c = Connection::open(&baseline.path).unwrap();
    let objects = schema(&c);
    let base = snapshot(&c);
    for n in [i64::MIN, i64::MAX, 9_007_199_254_740_993] {
        let command = serde_json::to_string(&Command::ProbeTypes {
            integer: Value::Integer(n),
            blob: vec![0, 255, 1],
            text: "bounded text".into(),
        })
        .unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let reply =
            foks_metadata_core::wire::run(&mut NativeSql::new(&tx).unwrap(), command.as_bytes())
                .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            reply.values,
            vec![
                Value::Integer(n),
                Value::Blob(vec![0, 255, 1]),
                Value::Null,
                Value::Text("bounded text".into())
            ]
        );
        vectors.push(Vector {
            name: format!("exact_i64_{n}"),
            command,
            before: base.clone(),
            after: base.clone(),
            outcome: "ok".into(),
            reply: Some(reply),
            fail_after: 0,
            trap_after: 0,
            deferred_failure: false,
        });
    }
    // Inject after the first actual dirent INSERT (and its accounting triggers).
    let command = serde_json::to_string(&Command::PutDirents {
        uid: UID.to_vec(),
        precondition: None,
        role: Role::OWNER.encoded().unwrap(),
        mutations: vec![mutation(0x66, 1)],
        limits: limits(&Config::default()),
        normalize_at: None,
    })
    .unwrap();
    for (name, fail_after, trap_after, deferred_failure) in [
        ("host_failure_after_insert", 8, 0, false),
        ("wasm_trap_after_insert", 0, 8, false),
        ("deferred_commit_failure", 0, 0, true),
    ] {
        vectors.push(Vector {
            name: name.into(),
            command: command.clone(),
            before: base.clone(),
            after: base.clone(),
            outcome: "host_failure".into(),
            reply: None,
            fail_after,
            trap_after,
            deferred_failure,
        });
    }
    // Corrupt/oversized stored data must be rejected before a payload is copied
    // into V8/Wasm; the guarded SELECT returns its length plus NULL instead.
    let mut oversized = base.clone();
    let table = oversized
        .iter_mut()
        .find(|t| t.name == "kv_dirents")
        .unwrap();
    let column = table
        .columns
        .iter()
        .position(|c| c == "exact_dirent")
        .unwrap();
    for row in &mut table.rows {
        row[column] = Value::Blob(vec![0; 4097]);
    }
    let command = serde_json::to_string(&Command::PutDirents {
        uid: UID.to_vec(),
        precondition: None,
        role: Role::OWNER.encoded().unwrap(),
        mutations: vec![mutation(0x61, 3)],
        limits: limits(&Config::default()),
        normalize_at: None,
    })
    .unwrap();
    vectors.push(Vector {
        name: "oversized_stored_blob".into(),
        command,
        before: oversized.clone(),
        after: oversized,
        outcome: "bounds".into(),
        reply: None,
        fail_after: 0,
        trap_after: 0,
        deferred_failure: false,
    });
    // Expose the deliberate atomicity difference instead of claiming broader
    // equivalence: old ensure_kv_namespace commits before its inner transaction.
    let mut fresh = common::TestDatabase::new();
    fresh.reserve(1_000_000);
    fresh.commit(None).unwrap();
    let mut fresh_connection = Connection::open(&fresh.path).unwrap();
    fresh_connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let initial = snapshot(&fresh_connection);
    assert_eq!(
        fresh_connection
            .query_row("SELECT count(*) FROM kv_namespaces", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let m = mutation(0x71, 1);
    let command = serde_json::to_string(&Command::PutDirents {
        uid: UID.to_vec(),
        precondition: None,
        role: Role::OWNER.encoded().unwrap(),
        mutations: vec![m.clone()],
        limits: limits(&Config::default()),
        normalize_at: None,
    })
    .unwrap();
    let tx = fresh_connection.transaction().unwrap();
    assert_eq!(
        foks_metadata_core::wire::run(&mut NativeSql::new(&tx).unwrap(), command.as_bytes()),
        Err(Error::Conflict)
    );
    tx.rollback().unwrap();
    assert_eq!(json(&initial), json(&snapshot(&fresh_connection)));
    assert!(matches!(
        fresh
            .database
            .put_kv_dirents(&UID, None, Role::OWNER, &[native_mutation(&m)]),
        Err(foks_server_db::Error::KvConflict)
    ));
    assert_eq!(
        fresh_connection
            .query_row("SELECT count(*) FROM kv_namespaces", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    vectors.push(Vector {
        name: "new_namespace_error_is_atomic_intentional_difference".into(),
        command,
        before: initial.clone(),
        after: initial,
        outcome: "conflict".into(),
        reply: None,
        fail_after: 0,
        trap_after: 0,
        deferred_failure: false,
    });
    if let Ok(directory) = std::env::var("FOKS_METADATA_VECTORS") {
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory).unwrap();
        let data = serde_json::json!({"schema_version":foks_server_db::SCHEMA_VERSION,"objects":objects,"vectors":vectors});
        std::fs::write(
            directory.join("vectors.json"),
            serde_json::to_vec(&data).unwrap(),
        )
        .unwrap();
        std::fs::write(
            directory.join("catalog.json"),
            serde_json::to_vec(&Statement::ALL.map(Statement::spec)).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn errors_after_sql_effects_roll_back_and_hosts_require_outer_transaction() {
    let test = fixture();
    let mut c = Connection::open(&test.path).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    assert!(NativeSql::new(&c).is_err());
    let before = json(&snapshot(&c));
    struct Fault<'a> {
        host: NativeSql<'a>,
    }
    impl Sql for Fault<'_> {
        fn query(
            &mut self,
            s: Statement,
            p: &[Value],
        ) -> foks_metadata_core::Result<Option<Vec<Value>>> {
            self.host.query(s, p)
        }
        fn execute(&mut self, s: Statement, p: &[Value]) -> foks_metadata_core::Result<u64> {
            let n = self.host.execute(s, p)?;
            if s == Statement::InsertDirent {
                Err(Error::Storage)
            } else {
                Ok(n)
            }
        }
    }
    let tx = c.transaction().unwrap();
    let mut fault = Fault {
        host: NativeSql::new(&tx).unwrap(),
    };
    assert_eq!(
        foks_metadata_core::put_dirents(
            &mut fault,
            &UID,
            None,
            Role::OWNER,
            &[mutation(0x66, 1)],
            &limits(&Config::default())
        ),
        Err(Error::Storage)
    );
    tx.rollback().unwrap();
    assert_eq!(before, json(&snapshot(&c)));
    c.execute_batch("CREATE TABLE probe_parent(id INTEGER PRIMARY KEY); CREATE TABLE probe_child(id INTEGER REFERENCES probe_parent(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
    let before = json(&snapshot(&c));
    let tx = c.transaction().unwrap();
    foks_metadata_core::put_dirents(
        &mut NativeSql::new(&tx).unwrap(),
        &UID,
        None,
        Role::OWNER,
        &[mutation(0x66, 1)],
        &limits(&Config::default()),
    )
    .unwrap();
    tx.execute("INSERT INTO probe_child VALUES(1)", []).unwrap();
    assert!(tx.commit().is_err());
    assert_eq!(before, json(&snapshot(&c)));
}

#[test]
fn name_conflict_query_cost_records_retained_history_and_unrelated_current_heads() {
    assert_eq!(
        Statement::NameConflict.spec().sql,
        foks_server_db::KV_NAME_CONFLICT_SQL
    );
    let mut measurements = Vec::new();
    for (retained, unrelated_heads) in [
        (0u32, 0u32),
        (100, 0),
        (1000, 0),
        (0, 100),
        (0, 1000),
        (1000, 1000),
        (10000, 10000),
    ] {
        let test = fixture();
        let mut c = Connection::open(&test.path).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        let tx = c.transaction().unwrap();
        let insert = |id: &[u8; 16], version: u32, node: &[u8; 17], name: &[u8; 32]| {
            tx.execute("INSERT INTO kv_dirents(uid,parent_id,dirent_id,version,directory_version,node_id,name_mac,creation_time,exact_dirent) VALUES(?1,?2,?3,?4,1,?5,?6,0,X'00')",rusqlite::params![UID.as_slice(),ROOT.as_slice(),id.as_slice(),version,node.as_slice(),name.as_slice()]).unwrap();
        };
        // Retained versions share the searched name, but the current head is a
        // tombstone. The fixed-key negative lookup must still return no row.
        for version in 1..=retained {
            insert(&[0x41; 16], version, &[3; 17], &[0x55; 32]);
        }
        insert(&[0x41; 16], retained + 1, &[0; 17], &[0x55; 32]);
        tx.execute(
            "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,?4)",
            rusqlite::params![
                UID.as_slice(),
                ROOT.as_slice(),
                [0x41u8; 16].as_slice(),
                retained + 1
            ],
        )
        .unwrap();
        for index in 0..unrelated_heads {
            let mut id = [0u8; 16];
            id[..4].copy_from_slice(&index.to_be_bytes());
            let mut name = [0u8; 32];
            name[..4].copy_from_slice(&index.to_be_bytes());
            insert(&id, 1, &[3; 17], &name);
            tx.execute(
                "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,1)",
                rusqlite::params![UID.as_slice(), ROOT.as_slice(), id.as_slice()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let spec = Statement::NameConflict.spec();
        let target_id = [0xf0u8; 16];
        let target_name = [0x55u8; 32];
        let parameters = rusqlite::params![
            UID.as_slice(),
            ROOT.as_slice(),
            target_id.as_slice(),
            target_name.as_slice()
        ];
        let mut statement = c.prepare(spec.sql).unwrap();
        {
            let mut rows = statement.query(parameters).unwrap();
            assert!(rows.next().unwrap().is_none());
        }
        let steps = statement.get_status(rusqlite::StatementStatus::VmStep);
        assert!(steps > 0);
        let plan = c
            .prepare(&format!("EXPLAIN QUERY PLAN {}", spec.sql))
            .unwrap()
            .query_map(parameters, |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        measurements.push(serde_json::json!({"retained_versions":retained,"unrelated_current_heads":unrelated_heads,"total_current_heads":unrelated_heads+1,"vm_steps":steps,"query_plan":plan}));
    }
    if let Ok(directory) = std::env::var("FOKS_METADATA_VECTORS") {
        let path = std::path::Path::new(&directory);
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(
            path.join("query-cost.json"),
            serde_json::to_vec_pretty(&measurements).unwrap(),
        )
        .unwrap();
    }
    // Negative indexed lookup work is independent of both retained versions
    // and unrelated live heads; SQLite b-tree depth is outside VM opcode count.
    let baseline = measurements[0]["vm_steps"].as_i64().unwrap();
    assert!(measurements
        .iter()
        .all(|m| m["vm_steps"].as_i64().unwrap() == baseline));
    eprintln!(
        "NameConflict SQLite VM measurements: {}",
        serde_json::to_string(&measurements).unwrap()
    );
}
