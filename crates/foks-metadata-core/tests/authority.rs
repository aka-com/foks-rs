#![cfg(feature = "native")]

#[path = "../../foks-server-db/tests/common/mod.rs"]
mod common;

use foks_metadata_core::{
    catalog::Statement, native::NativeSql, wire::Command, Error, Limits, Mutation, Sql, Value,
};
use foks_proto::{KvDirectoryStatus, KvDirent, KvNodeId, Role, SecretBox};
use foks_server_db::{Config, KvDirectoryMutation, KvNodeMutation, KvRootMutation};
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
use foks_metadata_core::authority::{
    self as a, Activation, Caller, Fence, Identity, Intent, Invocation, Number, Principal, Reply,
    Request,
};

const PRINCIPAL: [u8; 32] = [0x21; 32];
fn identity() -> Identity {
    Identity {
        installation: [1; 16],
        tenant: [2; 16],
        host: vec![3; 33],
        cell: [4; 32],
    }
}
fn activation() -> Activation {
    Activation {
        version: Number(1),
        schema: Number(foks_server_db::SCHEMA_VERSION),
        schema_hash: [5; 32],
        kernel_hash: [6; 32],
        catalog_hash: [10; 32],
        key_generation: Number(1),
        key_digest: [7; 32],
        storage_namespace: [8; 16],
        storage_placement: [9; 32],
        limits: limits(&Config::default()),
    }
}
fn fence(epoch: i64) -> Fence {
    Fence {
        identity: identity(),
        activation: activation(),
        owner: [epoch as u8; 16],
        epoch: Number(epoch),
    }
}
fn principal(enabled: bool) -> Principal {
    Principal {
        id: PRINCIPAL,
        namespace: UID.to_vec(),
        role: Role::OWNER.encoded().unwrap(),
        enabled,
    }
}
fn command(id: u8, version: u64) -> String {
    serde_json::to_string(&Command::PutDirents {
        uid: UID.to_vec(),
        precondition: None,
        role: Role::OWNER.encoded().unwrap(),
        mutations: vec![mutation(id, version)],
        limits: limits(&Config::default()),
        normalize_at: None,
    })
    .unwrap()
}
fn call(request: Request, control: bool, now: i64) -> Invocation {
    Invocation {
        caller: if control {
            Caller::Controller {}
        } else {
            Caller::Frontend {
                principal: PRINCIPAL,
            }
        },
        now: Number(now),
        deadline: Number(now + 1000),
        request,
    }
}
fn intent(epoch: i64, seq: i64, rev: i64, id: u8) -> Intent {
    Intent {
        epoch: Number(epoch),
        sequence: Number(seq),
        revision: Number(rev),
        expires: Number(1000),
        command: command(id, 1),
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
    fail_statement: u32,
    trap_statement: u32,
    deferred_failure: bool,
    authority: bool,
}
struct Harness {
    test: common::TestDatabase,
    connection: Connection,
    vectors: Vec<Vector>,
}
impl Harness {
    fn new() -> Self {
        let test = fixture();
        let c = Connection::open(&test.path).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        c.execute_batch(a::SCHEMA).unwrap();
        Self {
            test,
            connection: c,
            vectors: vec![],
        }
    }
    fn invoke(&mut self, name: &str, invocation: Invocation) -> Result<Reply, Error> {
        let before = snapshot(&self.connection);
        let command = serde_json::to_string(&invocation).unwrap();
        let tx = self.connection.transaction().unwrap();
        let result = a::run(&mut NativeSql::new(&tx).unwrap(), invocation);
        if result.is_ok() {
            tx.commit().unwrap();
        } else {
            tx.rollback().unwrap();
        }
        let after = snapshot(&self.connection);
        if result.is_err() {
            assert_eq!(json(&before), json(&after), "{name}");
        }
        self.vectors.push(Vector {
            name: name.into(),
            command,
            before,
            after,
            outcome: result.as_ref().err().map_or("ok", Error::code).into(),
            reply: result.as_ref().ok().cloned(),
            fail_after: 0,
            trap_after: 0,
            fail_statement: 0,
            trap_statement: 0,
            deferred_failure: false,
            authority: true,
        });
        result
    }
    fn init(&mut self, rows: i64, bytes: i64) {
        self.invoke(
            "initialize",
            call(
                Request::Initialize {
                    identity: identity(),
                    activation: activation(),
                    owner: [1; 16],
                    max_rows: Number(rows),
                    max_bytes: Number(bytes),
                },
                true,
                1,
            ),
        )
        .unwrap();
        self.invoke(
            "bind_principal",
            call(
                Request::SetPrincipal {
                    fence: fence(1),
                    revision: Number(1),
                    principal: principal(true),
                },
                true,
                2,
            ),
        )
        .unwrap();
    }
    fn register(&mut self, name: &str, f: Fence, i: Intent) -> [u8; 32] {
        match self
            .invoke(
                name,
                call(
                    Request::Register {
                        fence: f,
                        intent: i,
                    },
                    false,
                    3,
                ),
            )
            .unwrap()
        {
            Reply::Registered { digest } => digest,
            other => panic!("{other:?}"),
        }
    }
    fn revision(&self) -> i64 {
        self.connection
            .query_row("SELECT revision FROM metadata_authority", [], |r| r.get(0))
            .unwrap()
    }
    fn reopen(&mut self) {
        // Drop the active SQL connection, retaining only durable DB/fixture keys.
        self.connection = Connection::open(&self.test.path).unwrap();
        self.connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
    }
    fn export(&self) {
        self.export_suite("");
    }
    fn export_suite(&self, suite: &str) {
        if let Ok(directory) = std::env::var("FOKS_AUTHORITY_VECTORS") {
            let directory = std::path::Path::new(&directory).join(suite);
            std::fs::create_dir_all(&directory).unwrap();
            let data = serde_json::json!({"schema_version":foks_server_db::SCHEMA_VERSION,"objects":schema(&self.connection),"vectors":self.vectors});
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
}
#[test]
fn authority_exact_outcomes_fences_principal_state_retirement_and_exports() {
    let mut h = Harness::new();
    h.init(8, 8 * a::INTENT_CHARGE);
    let initial = intent(1, 1, 2, 0x61);
    let digest = h.register("register_before_source", fence(1), initial.clone());
    assert_eq!(
        h.revision(),
        2,
        "registration must not invalidate preparation"
    );
    assert_eq!(
        digest,
        h.register(
            "lost_registration_reply_exact_retry",
            fence(1),
            initial.clone()
        )
    );
    let mut changed = initial.clone();
    changed.command = command(0x62, 1);
    assert_eq!(
        h.invoke(
            "sequence_input_collision",
            call(
                Request::Register {
                    fence: fence(1),
                    intent: changed
                },
                false,
                3
            )
        ),
        Err(Error::Conflict)
    );
    let completed = h
        .invoke(
            "commit_domain_and_outcome",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: initial.clone(),
                },
                false,
                4,
            ),
        )
        .unwrap();
    assert!(
        matches!(&completed,Reply::Outcome {outcome} if outcome.reply.is_some() && outcome.error.is_none())
    );
    h.reopen();
    assert_eq!(
        h.invoke(
            "lost_reply_reopen_exact_execute",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: initial.clone()
                },
                false,
                5
            )
        )
        .unwrap(),
        completed
    );
    assert_eq!(
        h.invoke(
            "resolution_after_original_expiry",
            call(
                Request::Resolve {
                    fence: fence(1),
                    epoch: Number(1),
                    sequence: Number(1),
                    digest
                },
                false,
                1001
            )
        )
        .unwrap(),
        completed
    );
    // Clock floor is durable, and a bad observation cannot move it backwards.
    assert_eq!(
        h.invoke(
            "clock_rollback",
            call(Request::Inspect { fence: fence(1) }, true, 5)
        ),
        Err(Error::Expired)
    );
    let mut later = intent(1, 2, 3, 0x62);
    later.expires = Number(1500);
    let later_digest = match h
        .invoke(
            "register_pending_before_promotion",
            call(
                Request::Register {
                    fence: fence(1),
                    intent: later.clone(),
                },
                false,
                1002,
            ),
        )
        .unwrap()
    {
        Reply::Registered { digest } => digest,
        _ => unreachable!(),
    };
    assert_eq!(
        h.invoke(
            "frontend_cannot_promote",
            call(
                Request::Promote {
                    fence: fence(1),
                    new_owner: [2; 16]
                },
                false,
                1003
            )
        ),
        Err(Error::Permission)
    );
    h.invoke(
        "promote",
        call(
            Request::Promote {
                fence: fence(1),
                new_owner: [2; 16],
            },
            true,
            1003,
        ),
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "lost_promotion_reply_does_not_increment_again",
            call(
                Request::Promote {
                    fence: fence(1),
                    new_owner: [2; 16]
                },
                true,
                1003
            )
        ),
        Err(Error::Permission)
    );
    assert!(matches!(
        h.invoke(
            "resolve_promotion_by_exact_requested_tuple",
            call(Request::Inspect { fence: fence(2) }, true, 1003)
        )
        .unwrap(),
        Reply::State {
            epoch: Number(2),
            revision: Number(4),
            ..
        }
    ));
    assert_eq!(
        h.invoke(
            "old_owner_cannot_execute",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: later
                },
                false,
                1004
            )
        ),
        Err(Error::Permission)
    );
    assert_eq!(
        h.invoke(
            "old_observation_proof_fenced",
            call(
                Request::Prove {
                    fence: fence(1),
                    revision: Number(3)
                },
                false,
                1004
            )
        ),
        Err(Error::Permission)
    );
    assert_eq!(
        h.invoke(
            "successor_resolves_acknowledged_result",
            call(
                Request::Resolve {
                    fence: fence(2),
                    epoch: Number(1),
                    sequence: Number(1),
                    digest
                },
                false,
                1004
            )
        )
        .unwrap(),
        completed
    );
    let resolved = h
        .invoke(
            "closed_pending_is_definitely_unexecuted",
            call(
                Request::Resolve {
                    fence: fence(2),
                    epoch: Number(1),
                    sequence: Number(2),
                    digest: later_digest,
                },
                false,
                1004,
            ),
        )
        .unwrap();
    assert!(
        matches!(resolved,Reply::Outcome {outcome} if outcome.error.as_deref()==Some("not_executed"))
    );
    h.invoke(
        "revoke_original_principal",
        call(
            Request::SetPrincipal {
                fence: fence(2),
                revision: Number(4),
                principal: principal(false),
            },
            true,
            1005,
        ),
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "successor_resolution_reauthorizes",
            call(
                Request::Resolve {
                    fence: fence(2),
                    epoch: Number(1),
                    sequence: Number(1),
                    digest
                },
                false,
                1006
            )
        ),
        Err(Error::Permission)
    );
    assert_eq!(
        h.invoke(
            "bounded_retirement",
            call(
                Request::Prune {
                    fence: fence(2),
                    limit: Number(1)
                },
                true,
                1006
            )
        )
        .unwrap(),
        Reply::Pruned { count: Number(1) }
    );
    h.invoke(
        "enable_for_retired_lookup",
        call(
            Request::SetPrincipal {
                fence: fence(2),
                revision: Number(5),
                principal: principal(true),
            },
            true,
            1007,
        ),
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "missing_closed_outcome_never_executes",
            call(
                Request::Resolve {
                    fence: fence(2),
                    epoch: Number(1),
                    sequence: Number(1),
                    digest
                },
                false,
                1008
            )
        )
        .unwrap(),
        Reply::RetiredUnknown
    );
    let mut wrong = fence(2);
    wrong.identity.cell[0] ^= 1;
    assert_eq!(
        h.invoke(
            "wrong_cell_identity",
            call(Request::Inspect { fence: wrong }, false, 1008)
        ),
        Err(Error::Permission)
    );
    let mut wrong = fence(2);
    wrong.activation.kernel_hash[0] ^= 1;
    assert_eq!(
        h.invoke(
            "wrong_kernel_activation",
            call(Request::Inspect { fence: wrong }, false, 1008)
        ),
        Err(Error::Permission)
    );
    assert_eq!(
        h.invoke(
            "revision_bound_read_rejects_old",
            call(
                Request::ObserveUsage {
                    fence: fence(2),
                    revision: Number(5)
                },
                false,
                1008
            )
        ),
        Err(Error::Conflict)
    );
    h.invoke(
        "revision_bound_read",
        call(
            Request::ObserveUsage {
                fence: fence(2),
                revision: Number(6),
            },
            false,
            1008,
        ),
    )
    .unwrap();
    h.invoke(
        "final_observation_proof",
        call(
            Request::Prove {
                fence: fence(2),
                revision: Number(6),
            },
            false,
            1008,
        ),
    )
    .unwrap();
    h.export();
}

#[test]
fn domain_rejection_rolls_back_savepoint_and_reserves_exact_error_outcome() {
    let mut h = Harness::new();
    h.init(3, 3 * a::INTENT_CHARGE);
    let mut i = intent(1, 1, 2, 0x70);
    // First mutation inserts; second conflicts on the same current encrypted name.
    let first = mutation(0x70, 1);
    let mut second = mutation(0x71, 1);
    second.name_mac = first.name_mac;
    let mut exact = KvDirent::decode(&second.exact).unwrap();
    exact.name_mac = first.name_mac;
    second.exact = exact.encode().unwrap();
    i.command = serde_json::to_string(&Command::PutDirents {
        uid: UID.to_vec(),
        precondition: None,
        role: Role::OWNER.encoded().unwrap(),
        mutations: vec![first, second],
        limits: limits(&Config::default()),
        normalize_at: None,
    })
    .unwrap();
    let digest = h.register("late_error_registration", fence(1), i.clone());
    let before: i64 = h
        .connection
        .query_row("SELECT count(*) FROM kv_dirents", [], |r| r.get(0))
        .unwrap();
    let result = h
        .invoke(
            "late_domain_error_commits_only_error_outcome",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: i,
                },
                false,
                4,
            ),
        )
        .unwrap();
    assert!(
        matches!(&result,Reply::Outcome {outcome} if outcome.error.as_deref()==Some("conflict") && outcome.reply.is_none())
    );
    assert_eq!(
        h.connection
            .query_row("SELECT count(*) FROM kv_dirents", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        h.invoke(
            "exact_error_resolution",
            call(
                Request::Resolve {
                    fence: fence(1),
                    epoch: Number(1),
                    sequence: Number(1),
                    digest
                },
                false,
                1001
            )
        )
        .unwrap(),
        result
    );
    h.export_suite("errors");
}

#[test]
fn row_byte_budgets_current_high_water_and_many_closed_epochs_stay_bounded() {
    for (rows, bytes) in [(1, 2 * a::INTENT_CHARGE), (2, a::INTENT_CHARGE)] {
        let mut h = Harness::new();
        h.init(rows, bytes);
        let first = intent(1, 1, 2, 0x60);
        h.register("reserve", fence(1), first.clone());
        assert_eq!(
            h.invoke(
                "capacity_before_execution",
                call(
                    Request::Register {
                        fence: fence(1),
                        intent: intent(1, 2, 2, 0x61)
                    },
                    false,
                    3
                )
            ),
            Err(Error::Quota)
        );
        h.invoke(
            "finish",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: first.clone(),
                },
                false,
                4,
            ),
        )
        .unwrap();
        h.invoke(
            "retire",
            call(
                Request::Prune {
                    fence: fence(1),
                    limit: Number(64),
                },
                true,
                5,
            ),
        )
        .unwrap();
        h.reopen();
        assert_eq!(
            h.invoke(
                "retired_sequence_cannot_register",
                call(
                    Request::Register {
                        fence: fence(1),
                        intent: first
                    },
                    false,
                    6
                )
            )
            .unwrap(),
            Reply::RetiredUnknown
        );
        for epoch in 1..=70 {
            h.invoke(
                "promote_cycle",
                call(
                    Request::Promote {
                        fence: fence(epoch),
                        new_owner: [(epoch + 1) as u8; 16],
                    },
                    true,
                    7 + epoch,
                ),
            )
            .unwrap();
        }
        assert_eq!(
            h.connection
                .query_row("SELECT count(*) FROM metadata_authority", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            h.connection
                .query_row("SELECT count(*) FROM metadata_intents", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            h.connection
                .query_row("SELECT epoch FROM metadata_authority", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            71
        );
    }
}

#[test]
fn revoked_pending_work_can_be_settled_without_execution_and_expiry_cannot_refresh() {
    let mut h = Harness::new();
    h.init(4, 4 * a::INTENT_CHARGE);
    h.register("pending", fence(1), intent(1, 1, 2, 0x60));
    h.invoke(
        "revoke_changes_revision",
        call(
            Request::SetPrincipal {
                fence: fence(1),
                revision: Number(2),
                principal: principal(false),
            },
            true,
            4,
        ),
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "controller_settles_revoked_revision",
            call(
                Request::Settle {
                    fence: fence(1),
                    limit: Number(1)
                },
                true,
                5
            )
        )
        .unwrap(),
        Reply::Settled { count: Number(1) }
    );
    assert_eq!(
        h.connection
            .query_row("SELECT count(*) FROM kv_dirents", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    h.invoke(
        "enable",
        call(
            Request::SetPrincipal {
                fence: fence(1),
                revision: Number(3),
                principal: principal(true),
            },
            true,
            6,
        ),
    )
    .unwrap();
    let next = intent(1, 2, 4, 0x61);
    h.invoke(
        "register_fresh",
        call(
            Request::Register {
                fence: fence(1),
                intent: next.clone(),
            },
            false,
            7,
        ),
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "cannot_execute_expired",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: next.clone()
                },
                false,
                1001
            )
        ),
        Err(Error::Expired)
    );
    let mut changed = next;
    changed.expires = Number(2000);
    assert_eq!(
        h.invoke(
            "cannot_refresh_registered_expiry",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: changed
                },
                false,
                1002
            )
        ),
        Err(Error::Conflict)
    );
    assert_eq!(
        h.invoke(
            "settle_expired_same_epoch",
            call(
                Request::Settle {
                    fence: fence(1),
                    limit: Number(64)
                },
                true,
                1002
            )
        )
        .unwrap(),
        Reply::Settled { count: Number(1) }
    );
    assert_eq!(
        h.connection
            .query_row("SELECT count(*) FROM kv_dirents", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    h.export_suite("settlement");
}

#[test]
fn faults_after_domain_or_receipt_and_commit_leave_no_effect_or_outcome() {
    struct Fault<'a> {
        host: NativeSql<'a>,
        target: Statement,
        calls: usize,
        hit: usize,
    }
    impl Sql for Fault<'_> {
        fn query(&mut self, s: Statement, p: &[Value]) -> Result<Option<Vec<Value>>, Error> {
            self.calls += 1;
            self.host.query(s, p)
        }
        fn execute(&mut self, s: Statement, p: &[Value]) -> Result<u64, Error> {
            self.calls += 1;
            let result = self.host.execute(s, p)?;
            if s == self.target {
                self.hit = self.calls;
                return Err(Error::Storage);
            }
            Ok(result)
        }
    }
    let mut h = Harness::new();
    h.init(2, 2 * a::INTENT_CHARGE);
    let i = intent(1, 1, 2, 0x63);
    h.register("fault_registration", fence(1), i.clone());
    let invocation = call(
        Request::Execute {
            fence: fence(1),
            intent: i,
        },
        false,
        4,
    );
    for (name, target, trap) in [
        ("after_real_domain_insert", Statement::InsertDirent, false),
        (
            "after_real_outcome_insert",
            Statement::AuthorityOutcome,
            false,
        ),
        (
            "wasm_trap_after_real_domain_insert",
            Statement::InsertDirent,
            true,
        ),
    ] {
        let before = snapshot(&h.connection);
        let tx = h.connection.transaction().unwrap();
        let mut host = Fault {
            host: NativeSql::new(&tx).unwrap(),
            target,
            calls: 0,
            hit: 0,
        };
        assert_eq!(a::run(&mut host, invocation.clone()), Err(Error::Storage));
        let position = host.hit;
        assert!(position > 0);
        tx.rollback().unwrap();
        assert_eq!(json(&before), json(&snapshot(&h.connection)));
        h.vectors.push(Vector {
            name: name.into(),
            command: serde_json::to_string(&invocation).unwrap(),
            before: before.clone(),
            after: before,
            outcome: "host_failure".into(),
            reply: None,
            fail_after: 0,
            trap_after: 0,
            fail_statement: if trap { 0 } else { target as u32 },
            trap_statement: if trap { target as u32 } else { 0 },
            deferred_failure: false,
            authority: true,
        });
    }
    let before = snapshot(&h.connection);
    let tx = h.connection.transaction().unwrap();
    tx.execute_batch("PRAGMA defer_foreign_keys=ON").unwrap();
    assert!(a::run(&mut NativeSql::new(&tx).unwrap(), invocation.clone()).is_ok());
    tx.execute(
        "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,1)",
        rusqlite::params![UID, ROOT, [0x99_u8; 16]],
    )
    .unwrap();
    assert!(tx.commit().is_err());
    assert_eq!(json(&before), json(&snapshot(&h.connection)));
    h.vectors.push(Vector {
        name: "deferred_commit_failure_after_outcome".into(),
        command: serde_json::to_string(&invocation).unwrap(),
        before: before.clone(),
        after: before,
        outcome: "host_failure".into(),
        reply: None,
        fail_after: 0,
        trap_after: 0,
        fail_statement: 0,
        trap_statement: 0,
        deferred_failure: true,
        authority: true,
    });
    h.export_suite("faults");
}

#[test]
fn canonical_command_binding_rejects_policy_and_principal_substitution() {
    let mut h = Harness::new();
    h.init(4, 4 * a::INTENT_CHARGE);
    let original = intent(1, 1, 2, 0x65);
    let digest = h.register("canonical", fence(1), original.clone());
    let mut equivalent = original.clone();
    equivalent.command = serde_json::to_string_pretty(
        &serde_json::from_str::<serde_json::Value>(&original.command).unwrap(),
    )
    .unwrap();
    assert_eq!(
        h.register(
            "canonical_object_order_and_whitespace",
            fence(1),
            equivalent
        ),
        digest
    );
    let mut policy = original.clone();
    let mut parsed: serde_json::Value = serde_json::from_str(&policy.command).unwrap();
    parsed["limits"]["namespace_bytes"] = serde_json::json!(i64::MAX);
    policy.command = serde_json::to_string(&parsed).unwrap();
    assert_eq!(
        h.invoke(
            "cannot_increase_authoritative_policy",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: policy
                },
                false,
                4
            )
        ),
        Err(Error::Permission)
    );
    let mut changed = call(
        Request::Execute {
            fence: fence(1),
            intent: original.clone(),
        },
        false,
        4,
    );
    changed.caller = Caller::Frontend {
        principal: [0x22; 32],
    };
    assert_eq!(
        h.invoke("different_authenticated_principal", changed),
        Err(Error::Permission)
    );
    let mut active = activation();
    active.version = Number(2);
    active.key_generation = Number(2);
    active.key_digest = [0xff; 32];
    h.invoke(
        "controller_key_activation_changes_revision",
        call(
            Request::Activate {
                fence: fence(1),
                revision: Number(2),
                activation: active.clone(),
            },
            true,
            4,
        ),
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "prepared_old_key_cannot_execute",
            call(
                Request::Execute {
                    fence: fence(1),
                    intent: original
                },
                false,
                5
            )
        ),
        Err(Error::Permission)
    );
    let mut new_fence = fence(1);
    new_fence.activation = active;
    assert_eq!(
        h.invoke(
            "settle_invalidated_activation",
            call(
                Request::Settle {
                    fence: new_fence,
                    limit: Number(64)
                },
                true,
                5
            )
        )
        .unwrap(),
        Reply::Settled { count: Number(1) }
    );
    h.export_suite("bindings");
}

#[test]
fn infrastructure_limits_after_a_domain_effect_abort_instead_of_becoming_receipts() {
    struct Limit<'a> {
        host: NativeSql<'a>,
        error: Error,
    }
    impl Sql for Limit<'_> {
        fn query(&mut self, s: Statement, p: &[Value]) -> Result<Option<Vec<Value>>, Error> {
            self.host.query(s, p)
        }
        fn execute(&mut self, s: Statement, p: &[Value]) -> Result<u64, Error> {
            let result = self.host.execute(s, p)?;
            if s == Statement::InsertDirent {
                return Err(self.error.clone());
            }
            Ok(result)
        }
    }
    let mut h = Harness::new();
    h.init(2, 2 * a::INTENT_CHARGE);
    let i = intent(1, 1, 2, 0x65);
    h.register("limit_registration", fence(1), i.clone());
    for error in [
        Error::Bounds,
        Error::Expired,
        Error::IntegerRange,
        Error::Invalid("corrupt stored value"),
    ] {
        let before = snapshot(&h.connection);
        let tx = h.connection.transaction().unwrap();
        let mut host = Limit {
            host: NativeSql::new(&tx).unwrap(),
            error: error.clone(),
        };
        assert_eq!(
            a::run(
                &mut host,
                call(
                    Request::Execute {
                        fence: fence(1),
                        intent: i.clone()
                    },
                    false,
                    4
                )
            ),
            Err(error)
        );
        tx.rollback().unwrap();
        assert_eq!(json(&before), json(&snapshot(&h.connection)));
        assert_eq!(
            h.connection
                .query_row("SELECT terminal FROM metadata_intents", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn malformed_numbers_unknown_fields_and_epoch_exhaustion_fail_closed() {
    for raw in [r#""01""#, r#"1"#, r#""9223372036854775808""#] {
        assert!(serde_json::from_str::<Number>(raw).is_err());
    }
    let mut h = Harness::new();
    h.init(2, 2 * a::INTENT_CHARGE);
    for controller in [true, false] {
        for extra in ["untrusted_override", "identity", "owner"] {
            let mut unknown =
                serde_json::to_value(call(Request::Inspect { fence: fence(1) }, controller, 3))
                    .unwrap();
            unknown["caller"][extra] = serde_json::json!(true);
            assert!(serde_json::from_value::<Invocation>(unknown).is_err());
        }
    }
    h.connection
        .execute("UPDATE metadata_authority SET epoch=?1", [i64::MAX])
        .unwrap();
    let mut exhausted = fence(1);
    exhausted.epoch = Number(i64::MAX);
    assert_eq!(
        h.invoke(
            "epoch_exhaustion",
            call(
                Request::Promote {
                    fence: exhausted,
                    new_owner: [2; 16]
                },
                true,
                3
            )
        ),
        Err(Error::IntegerRange)
    );
    assert_eq!(
        h.connection
            .query_row("SELECT epoch FROM metadata_authority", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        i64::MAX
    );
}

#[test]
fn normalization_binds_trusted_microseconds_and_exact_retry_keeps_original_time() {
    fn normalized(mut i: Intent, micros: u64) -> Intent {
        let mut command: Command = serde_json::from_str(&i.command).unwrap();
        let Command::PutDirents { normalize_at, .. } = &mut command else {
            unreachable!()
        };
        *normalize_at = Some(micros);
        i.command = serde_json::to_string(&command).unwrap();
        i
    }
    let mut h = Harness::new();
    h.init(3, 3 * a::INTENT_CHARGE);
    let i = normalized(intent(1, 1, 2, 0x68), 3000);
    let digest = h.register("normalized_time_ms_to_micros", fence(1), i.clone());
    assert_eq!(
        h.invoke(
            "retry_does_not_resample_creation_time",
            call(
                Request::Register {
                    fence: fence(1),
                    intent: i.clone()
                },
                false,
                4
            )
        )
        .unwrap(),
        Reply::Registered { digest }
    );
    let changed = normalized(i.clone(), 4000);
    assert_eq!(
        h.invoke(
            "retry_cannot_rebind_creation_time",
            call(
                Request::Register {
                    fence: fence(1),
                    intent: changed
                },
                false,
                4
            )
        ),
        Err(Error::Conflict)
    );
    h.invoke(
        "execute_original_normalization",
        call(
            Request::Execute {
                fence: fence(1),
                intent: i.clone(),
            },
            false,
            5,
        ),
    )
    .unwrap();
    assert_eq!(
        h.connection
            .query_row("SELECT creation_time FROM kv_dirents", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        3000
    );
    h.invoke(
        "retire_normalized_intent",
        call(
            Request::Prune {
                fence: fence(1),
                limit: Number(1),
            },
            true,
            6,
        ),
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "missing_retired_time_never_refreshes",
            call(
                Request::Register {
                    fence: fence(1),
                    intent: i
                },
                false,
                7
            )
        )
        .unwrap(),
        Reply::RetiredUnknown
    );
    let mut next = normalized(intent(1, 2, 3, 0x69), 8);
    next.expires = Number(1000);
    assert_eq!(
        h.invoke(
            "milliseconds_are_not_microseconds",
            call(
                Request::Register {
                    fence: fence(1),
                    intent: next.clone()
                },
                false,
                8
            )
        ),
        Err(Error::Invalid("normalization time"))
    );
    let now = i64::MAX / 1000 + 1;
    next.expires = Number(now + 500);
    assert_eq!(
        h.invoke(
            "normalization_time_overflow",
            call(
                Request::Register {
                    fence: fence(1),
                    intent: next
                },
                false,
                now
            )
        ),
        Err(Error::IntegerRange)
    );
    assert_eq!(
        h.connection
            .query_row("SELECT high_water FROM metadata_authority", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    h.export_suite("normalization");
}

#[test]
fn retirement_pass_is_bounded_and_does_not_evict_live_intents() {
    let mut h = Harness::new();
    h.init(72, 72 * a::INTENT_CHARGE);
    for sequence in 1..=72 {
        let i = intent(1, sequence, sequence.max(2), 0x66);
        h.invoke(
            "many_registration",
            call(
                Request::Register {
                    fence: fence(1),
                    intent: i.clone(),
                },
                false,
                10,
            ),
        )
        .unwrap();
        if (2..=71).contains(&sequence) {
            h.invoke(
                "many_terminal",
                call(
                    Request::Execute {
                        fence: fence(1),
                        intent: i,
                    },
                    false,
                    10,
                ),
            )
            .unwrap();
        }
    }
    assert_eq!(
        h.invoke(
            "invalid_prune_bound",
            call(
                Request::Prune {
                    fence: fence(1),
                    limit: Number(65)
                },
                true,
                11
            )
        ),
        Err(Error::Bounds)
    );
    assert_eq!(
        h.invoke(
            "first_bounded_pass",
            call(
                Request::Prune {
                    fence: fence(1),
                    limit: Number(64)
                },
                true,
                11
            )
        )
        .unwrap(),
        Reply::Pruned { count: Number(64) }
    );
    assert_eq!(
        h.connection
            .query_row("SELECT retained_rows FROM metadata_authority", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        8
    );
    assert_eq!(
        h.invoke(
            "second_bounded_pass",
            call(
                Request::Prune {
                    fence: fence(1),
                    limit: Number(64)
                },
                true,
                11
            )
        )
        .unwrap(),
        Reply::Pruned { count: Number(6) }
    );
    assert_eq!(
        h.connection
            .query_row(
                "SELECT count(*) FROM metadata_intents WHERE terminal=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    let remaining = h
        .connection
        .prepare("SELECT sequence FROM metadata_intents ORDER BY sequence")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        remaining,
        vec![1, 72],
        "old live sequences must survive out-of-order retirement"
    );
    assert_eq!(
        h.connection
            .query_row("SELECT high_water FROM metadata_authority", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        72
    );
    assert_eq!(
        h.connection
            .query_row("SELECT reserved_bytes FROM metadata_authority", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        2 * a::INTENT_CHARGE
    );
}

#[test]
fn nested_domain_controls_share_exact_call_budget_on_success_and_rejection() {
    use foks_metadata_core::MAX_SQL_CALLS;
    let probe = [
        Value::Integer(0),
        Value::Blob(vec![]),
        Value::Null,
        Value::Text(String::new()),
    ];
    for reject in [false, true] {
        let h = Harness::new();
        let first = mutation(0x72, 1);
        let mut second = mutation(0x73, 1);
        second.name_mac = first.name_mac;
        let mut exact = KvDirent::decode(&second.exact).unwrap();
        exact.name_mac = first.name_mac;
        second.exact = exact.encode().unwrap();
        let command = serde_json::to_vec(&Command::PutDirents {
            uid: UID.to_vec(),
            precondition: None,
            role: Role::OWNER.encoded().unwrap(),
            mutations: if reject {
                vec![first, second]
            } else {
                vec![first]
            },
            limits: limits(&Config::default()),
            normalize_at: None,
        })
        .unwrap();
        let before = snapshot(&h.connection);
        let tx = h.connection.unchecked_transaction().unwrap();
        let mut host = NativeSql::new(&tx).unwrap();
        let expected = host.domain(&command);
        assert_eq!(expected.is_err(), reject);
        if reject {
            assert_eq!(expected, Err(Error::Conflict));
        }
        let cost = host.calls();
        assert!(cost > 3 && cost < MAX_SQL_CALLS);
        tx.rollback().unwrap();
        for over in [0, 1] {
            let tx = h.connection.unchecked_transaction().unwrap();
            let mut host = NativeSql::new(&tx).unwrap();
            for _ in 0..MAX_SQL_CALLS - cost + over {
                host.query(Statement::ProbeTypes, &probe).unwrap();
            }
            let observed = host.domain(&command);
            if over == 0 {
                assert_eq!(observed, expected);
                assert_eq!(host.calls(), MAX_SQL_CALLS);
            } else {
                assert_eq!(observed, Err(Error::Bounds));
                assert_eq!(host.calls(), MAX_SQL_CALLS + 1);
            }
            tx.rollback().unwrap();
            assert_eq!(json(&snapshot(&h.connection)), json(&before));
        }
    }
}
