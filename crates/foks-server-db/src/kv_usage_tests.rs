use super::*;
use rusqlite::{Connection, StatementStatus};

const UID: [u8; 33] = [1; 33];
const OTHER: [u8; 33] = [2; 33];
fn boxed(n: usize) -> Vec<u8> {
    foks_proto::KvSmallFileBox {
        key: foks_proto::RoleAndGeneration {
            role: foks_proto::Role::OWNER,
            generation: 1,
        },
        ciphertext: vec![7; n],
    }
    .encode()
    .unwrap()
}
fn id(n: u64) -> [u8; 17] {
    let mut id = [3; 17];
    id[9..].copy_from_slice(&n.to_be_bytes());
    id
}
fn connection() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    c.pragma_update(None, "recursive_triggers", false).unwrap();
    crate::schema::initialize(&mut c).unwrap();
    for uid in [UID, OTHER] {
        c.execute(
            "INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(?1,1,?1)",
            [uid.as_slice()],
        )
        .unwrap();
    }
    c
}
fn insert(c: &Connection, n: u64, exact: &[u8]) {
    c.execute(
        "INSERT INTO kv_nodes(uid,node_id,node_type,exact_node) VALUES(?1,?2,3,?3)",
        params![UID, id(n), exact],
    )
    .unwrap();
}
fn stamp(c: &Connection, n: u64, exact: &[u8]) {
    stamp_node_usage(
        c,
        &KvNodeMutation {
            uid: &UID,
            id: &id(n),
            node_type: 3,
            exact,
        },
    )
    .unwrap();
}
// Frozen predecessor scan: do not implement this oracle through derived columns.
fn legacy(c: &Connection, uid: &[u8]) -> Result<foks_proto::KvUsage> {
    let mut statement =
        c.prepare("SELECT exact_node FROM kv_nodes WHERE uid=?1 AND node_type IN(3,4)")?;
    let mut small_number = 0u64;
    let mut small_bytes = 0u64;
    for exact in statement.query_map([uid], |r| r.get::<_, Vec<u8>>(0))? {
        let boxed = foks_proto::KvSmallFileBox::decode(&exact?)
            .map_err(|_| Error::Invalid("stored KV small node"))?;
        small_number = small_number.checked_add(1).ok_or(Error::IntegerRange)?;
        small_bytes = small_bytes
            .checked_add(u64::try_from(boxed.ciphertext.len()).map_err(|_| Error::IntegerRange)?)
            .ok_or(Error::IntegerRange)?;
    }
    let (number,bytes,chunks):(i64,i64,i64)=c.query_row("SELECT count(*),coalesce(sum(chunk_bytes),0),coalesce(sum(chunk_count),0) FROM kv_file_uploads WHERE uid=?1",[uid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    Ok(foks_proto::KvUsage {
        small: foks_proto::KvUsageStats {
            number: small_number,
            bytes: small_bytes,
        },
        large: foks_proto::KvChunkedUsageStats {
            base: foks_proto::KvUsageStats {
                number: crate::error::unsigned(number)?,
                bytes: crate::error::unsigned(bytes)?,
            },
            chunks: crate::error::unsigned(chunks)?,
        },
    })
}
fn same(c: &Connection) {
    for uid in [UID, OTHER, [9; 33]] {
        match (legacy(c, &uid), kv_usage(c, &uid)) {
            (Ok(a), Ok(b)) => assert_eq!(a.encode().unwrap(), b.encode().unwrap()),
            (Err(Error::Invalid(a)), Err(Error::Invalid(b))) => assert_eq!(a, b),
            values => panic!("usage differs: {values:?}"),
        }
    }
}
fn schema(c: &Connection) -> Vec<(String, String, Option<String>)> {
    c.prepare(
        "SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
    )
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}
fn downgrade(c: &Connection) {
    c.execute_batch(concat!(
        include_str!("../tests/fixtures/remove_read_dependency.sql"),
        include_str!("../tests/fixtures/remove_kv_usage.sql")
    ))
    .unwrap();
    c.pragma_update(None, "user_version", 57).unwrap();
}
fn upload(c: &Connection, n: u64) {
    let file = n.to_be_bytes().repeat(2);
    c.execute("INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(?1,?2,X'01',0,0)",params![UID,file]).unwrap();
}
fn chunk(c: &Connection, n: u64, offset: i64, bytes: usize) {
    c.execute("INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk) VALUES(?1,?2,?3,?4,0,X'01')",params![UID,n.to_be_bytes().repeat(2),offset,vec![8u8;bytes]]).unwrap();
}

#[test]
fn exact_typed_retries_revalidate_and_commit_without_double_counting() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    let mut db = crate::Database::open(&path, crate::Config::default()).unwrap();
    db.connection
        .execute(
            "INSERT INTO names(normalized_name,reservation_sequence,uid) VALUES(?1,1,?2)",
            params![b"usage-owner".as_slice(), UID.as_slice()],
        )
        .unwrap();
    db.connection
        .execute(
            "INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES(?1,?2,?2,1,zeroblob(16),0)",
            params![UID.as_slice(), b"usage-owner".as_slice()],
        )
        .unwrap();
    let exact = boxed(3000);
    let node = id(1);
    let m = KvNodeMutation {
        uid: &UID,
        id: &node,
        node_type: 3,
        exact: &exact,
    };
    db.put_kv_node(&m).unwrap();
    db.put_kv_node(&m).unwrap();
    same(&db.connection);
    assert_eq!(kv_usage(&db.connection, &UID).unwrap().small.bytes, 3000);
    assert!(matches!(
        db.put_kv_node(&KvNodeMutation {
            exact: &boxed(3001),
            ..m
        }),
        Err(Error::KvConflict)
    ));
    db.connection
        .execute("UPDATE kv_nodes SET ciphertext_bytes=NULL", [])
        .unwrap();
    assert!(matches!(
        kv_usage(&db.connection, &UID),
        Err(Error::Invalid("stored KV small node"))
    ));
    db.put_kv_node(&m).unwrap();
    drop(db);
    let db = crate::Database::open(&path, crate::Config::default()).unwrap();
    same(&db.connection);
    assert_eq!(kv_usage(&db.connection, &UID).unwrap().small.number, 1);
}

#[test]
fn exact_change_invalidation_collision_cascades_and_rollback_preserve_oracle() {
    let mut c = connection();
    same(&c);
    let a = boxed(10);
    insert(&c, 1, &a);
    stamp(&c, 1, &a);
    same(&c);
    let mut changed = foks_proto::KvSmallFileBox::decode(&a).unwrap();
    changed.ciphertext.fill(9);
    let b = changed.encode().unwrap();
    assert!(c
        .execute("UPDATE kv_nodes SET exact_node=?1", [&b])
        .is_err());
    same(&c);
    c.execute(
        "UPDATE kv_nodes SET exact_node=?1,ciphertext_bytes=NULL",
        [&b],
    )
    .unwrap();
    assert!(kv_usage(&c, &UID).is_err());
    stamp(&c, 1, &b);
    same(&c);
    for suffix in [
        "",
        " ON CONFLICT(uid,node_id) DO NOTHING",
        " ON CONFLICT(uid,node_id) DO UPDATE SET exact_node=excluded.exact_node",
    ] {
        assert!(c.execute(&format!("INSERT INTO kv_nodes(uid,node_id,node_type,exact_node) VALUES(?1,?2,3,?3){suffix}"),params![UID,id(1),b]).is_err());
        same(&c);
    }
    assert!(c
        .execute(
            "INSERT OR REPLACE INTO kv_nodes(uid,node_id,node_type,exact_node) VALUES(?1,?2,3,?3)",
            params![UID, id(1), b]
        )
        .is_err());
    same(&c);
    c.execute("UPDATE kv_nodes SET uid=?1", [OTHER.as_slice()])
        .unwrap();
    same(&c);
    upload(&c, 1);
    upload(&c, 2);
    chunk(&c, 1, 0, 9);
    chunk(&c, 1, 20, 7);
    same(&c);
    c.execute(
        "UPDATE kv_file_uploads SET complete=1,reclaiming=1,updated_at=2",
        [],
    )
    .unwrap();
    same(&c);
    assert!(c.execute("INSERT OR REPLACE INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(?1,?2,X'',0,0)",params![UID,1u64.to_be_bytes().repeat(2)]).is_err());
    same(&c);
    let before = kv_usage(&c, &UID).unwrap();
    let tx = c.transaction().unwrap();
    chunk(&tx, 1, 30, 11);
    same(&tx);
    tx.rollback().unwrap();
    assert_eq!(kv_usage(&c, &UID).unwrap(), before);
    c.execute("DELETE FROM kv_file_chunks WHERE clear_offset=0", [])
        .unwrap();
    same(&c);
    c.execute("DELETE FROM kv_file_uploads", []).unwrap();
    same(&c);
    c.execute("DELETE FROM kv_namespaces", []).unwrap();
    same(&c);
    assert!(!c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
}

#[test]
fn genuine57_backfill_preserves_valid_oversized_and_malformed_payloads_and_reopens() {
    let mut c = connection();
    downgrade(&c);
    for (n, len) in [(1, 0), (2, 2065), (3, 60_000)] {
        insert(&c, n, &boxed(len));
    }
    insert(&c, 4, b"invalid");
    upload(&c, 1);
    chunk(&c, 1, 0, 23);
    let exacts: Vec<Vec<u8>> = c
        .prepare("SELECT exact_node FROM kv_nodes ORDER BY node_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    crate::schema::initialize(&mut c).unwrap();
    same(&c);
    assert_eq!(schema(&c), schema(&connection()));
    assert_eq!(
        exacts,
        c.prepare("SELECT exact_node FROM kv_nodes ORDER BY node_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<Vec<u8>>>>()
            .unwrap()
    );
    assert_eq!(
        c.query_row(
            "SELECT usage_unvalidated_small_count FROM kv_namespaces WHERE namespace_id=?1",
            [UID.as_slice()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    c.execute("DELETE FROM kv_nodes WHERE node_id=?1", [id(4)])
        .unwrap();
    same(&c);
    assert_eq!(kv_usage(&c, &UID).unwrap().small.bytes, 62_065);
    crate::schema::initialize(&mut c).unwrap();
    same(&c);
}

#[test]
fn migration_aborts_on_bad_upload_totals_resource_failure_and_late_ddl_collision() {
    for failure in [0, 1, 2, 3] {
        let mut c = connection();
        downgrade(&c);
        insert(&c, 1, &boxed(16));
        upload(&c, 1);
        chunk(&c, 1, 0, 9);
        if failure == 0 {
            c.execute("UPDATE kv_file_uploads SET chunk_bytes=10", [])
                .unwrap();
        }
        if failure == 2 {
            c.execute_batch("CREATE TRIGGER kv_nodes_usage_insert AFTER INSERT ON kv_nodes BEGIN SELECT 1; END;").unwrap();
        }
        if failure == 3 {
            c.execute(
                "UPDATE kv_nodes SET exact_node=zeroblob(?1)",
                [i64::try_from(foks_proto::MAXIMUM_KV_NODE_BYTES + 1).unwrap()],
            )
            .unwrap();
        }
        let before = schema(&c);
        let before_exact: Vec<u8> = c
            .query_row("SELECT exact_node FROM kv_nodes", [], |r| r.get(0))
            .unwrap();
        struct Reject;
        impl foks_snowpack::AllocationBudget for Reject {
            fn reserve(&self, bytes: usize) -> bool {
                bytes == 512
            }
        }
        let result = if failure == 1 {
            foks_snowpack::with_allocation_budget(std::sync::Arc::new(Reject), || {
                crate::schema::initialize(&mut c)
            })
        } else {
            crate::schema::initialize(&mut c)
        };
        assert!(result.is_err());
        if failure == 3 {
            assert!(matches!(result, Err(Error::QuotaExceeded)));
        }
        if failure == 1 {
            assert!(matches!(
                result,
                Err(Error::Capacity("KV usage migration allocation"))
            ));
        }
        assert_eq!(schema(&c), before);
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            57
        );
        assert_eq!(
            c.query_row("SELECT exact_node FROM kv_nodes", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            before_exact
        );
    }
}

#[test]
fn usage_arithmetic_overflow_aborts_whole_mutation() {
    let c = connection();
    let exact = boxed(1);
    insert(&c, 1, &exact);
    stamp(&c, 1, &exact);
    c.execute(
        "UPDATE kv_namespaces SET usage_small_count=?2 WHERE namespace_id=?1",
        params![UID, i64::MAX],
    )
    .unwrap();
    assert!(c
        .execute(
            "INSERT INTO kv_nodes(uid,node_id,node_type,exact_node) VALUES(?1,?2,3,?3)",
            params![UID, id(2), exact]
        )
        .is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM kv_nodes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    c.execute(
        "UPDATE kv_namespaces SET usage_small_count=1,usage_large_bytes=?2 WHERE namespace_id=?1",
        params![UID, i64::MAX],
    )
    .unwrap();
    upload(&c, 1);
    assert!(c.execute("INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk) VALUES(?1,?2,0,X'01',0,X'01')",params![UID,1u64.to_be_bytes().repeat(2)]).is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM kv_file_chunks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT chunk_count FROM kv_file_uploads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn actual_public_usage_query_and_per_row_triggers_have_history_independent_vm_work() {
    let mut all = Vec::new();
    for count in [1u64, 100, 10_000] {
        let mut c = connection();
        let exact = boxed(1);
        let tx = c.transaction().unwrap();
        for n in 1..=count {
            insert(&tx, n, &exact);
            stamp(&tx, n, &exact);
            upload(&tx, n);
            chunk(&tx, n, 0, 1);
        }
        tx.commit().unwrap();
        same(&c);
        downgrade(&c);
        let pages_before: i64 = c
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        let started = std::time::Instant::now();
        crate::schema::initialize(&mut c).unwrap();
        let elapsed = started.elapsed();
        let pages_after: i64 = c
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        same(&c);
        eprintln!(
            "usage migration rows_per_kind={count} elapsed={elapsed:?} added_pages={}",
            pages_after - pages_before
        );
        let mut costs = Vec::new();
        for uid in [UID, [9; 33]] {
            let mut q = c.prepare(KV_PUBLIC_USAGE_SQL).unwrap();
            {
                let mut rows = q.query([uid.as_slice()]).unwrap();
                while rows.next().unwrap().is_some() {}
            }
            costs.push(q.get_status(StatementStatus::VmStep));
        }
        let mut q = c
            .prepare("UPDATE kv_nodes SET ciphertext_bytes=NULL WHERE uid=?1 AND node_id=?2")
            .unwrap();
        q.execute(params![UID, id(1)]).unwrap();
        costs.push(q.get_status(StatementStatus::VmStep));
        stamp(&c, 1, &exact);
        let mut q=c.prepare("UPDATE kv_file_chunks SET ciphertext=X'0102' WHERE uid=?1 AND file_id=?2 AND clear_offset=0").unwrap();
        q.execute(params![UID, 1u64.to_be_bytes().repeat(2)])
            .unwrap();
        costs.push(q.get_status(StatementStatus::VmStep));
        same(&c);
        let plan: Vec<String> = c
            .prepare(&format!("EXPLAIN QUERY PLAN {KV_PUBLIC_USAGE_SQL}"))
            .unwrap()
            .query_map([UID.as_slice()], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            plan.iter().any(|p| p.contains("SEARCH kv_namespaces")),
            "{plan:?}"
        );
        eprintln!("public usage history={count} vm={costs:?} plan={plan:?}");
        all.push(costs);
    }
    assert!(all.windows(2).all(|pair| pair[0] == pair[1]), "{all:?}");
}

#[test]
fn raw_upload_key_relocation_is_refused_without_drift_in_either_order() {
    for child_first in [false, true] {
        let mut c = connection();
        upload(&c, 1);
        chunk(&c, 1, 0, 17);
        same(&c);
        let before = kv_usage(&c, &UID).unwrap();
        let tx = c.transaction().unwrap();
        tx.execute_batch("PRAGMA defer_foreign_keys=ON").unwrap();
        if child_first {
            tx.execute("UPDATE kv_file_chunks SET uid=?1", [OTHER.as_slice()])
                .unwrap();
        }
        assert!(tx
            .execute("UPDATE kv_file_uploads SET uid=?1", [OTHER.as_slice()])
            .is_err());
        tx.rollback().unwrap();
        same(&c);
        assert_eq!(kv_usage(&c, &UID).unwrap(), before);
        c.execute("UPDATE kv_file_uploads SET uid=uid,file_id=file_id", [])
            .unwrap();
        same(&c);
    }
}

#[test]
fn colliding_node_key_updates_fail_closed_without_replacement_delete_drift() {
    let c = connection();
    let exact = boxed(5);
    insert(&c, 1, &exact);
    stamp(&c, 1, &exact);
    insert(&c, 2, &exact);
    stamp(&c, 2, &exact);
    let before = kv_usage(&c, &UID).unwrap();
    assert!(c
        .execute(
            "UPDATE OR REPLACE kv_nodes SET node_id=?2 WHERE node_id=?1",
            params![id(1), id(2)]
        )
        .is_err());
    same(&c);
    assert_eq!(kv_usage(&c, &UID).unwrap(), before);
    c.execute("UPDATE kv_nodes SET node_id=node_id,uid=uid", [])
        .unwrap();
    same(&c);
    c.execute(
        "UPDATE kv_nodes SET uid=?1 WHERE node_id=?2",
        params![OTHER, id(2)],
    )
    .unwrap();
    same(&c);
    assert!(c
        .execute(
            "UPDATE OR REPLACE kv_nodes SET uid=?1,node_id=?2 WHERE uid=?3",
            params![OTHER, id(2), UID]
        )
        .is_err());
    same(&c);
}
