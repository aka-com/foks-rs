//! Production access-path, trigger and genuine-predecessor regression gates.
use super::*;
use rusqlite::{Connection, StatementStatus};

const UID: [u8; 33] = [0; 33];
const PARENT: [u8; 16] = [0; 16];

fn connection() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    crate::schema::initialize(&mut c).unwrap();
    c.execute_batch("INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33));
        INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'01');
        INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),2,3,0,1,0,X'02');
        INSERT INTO kv_directory_heads VALUES(zeroblob(33),zeroblob(16),2);").unwrap();
    c
}
fn id(number: u64) -> [u8; 16] {
    let mut id = [0; 16];
    id[8..].copy_from_slice(&number.to_be_bytes());
    id
}
fn history(c: &Connection, number: u64, version: i64, mac: u8, live: bool) {
    let mut node = [0_u8; 17];
    node[0] = if live { 2 } else { 0 };
    node[1..].copy_from_slice(&id(number));
    c.execute(
        "INSERT INTO kv_dirents VALUES(?1,?2,?3,?4,1,?5,?6,?7,?8)",
        params![
            UID,
            PARENT,
            id(number),
            version,
            node,
            [mac; 32],
            i64::from(mac),
            format!("{number}:{version}").as_bytes()
        ],
    )
    .unwrap();
}
fn head(c: &Connection, number: u64, version: i64) {
    c.execute(
        "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,?4)
        ON CONFLICT(uid,parent_id,dirent_id) DO UPDATE SET version=excluded.version",
        params![UID, PARENT, id(number), version],
    )
    .unwrap();
}
fn invariant(c: &Connection) {
    let expected = "SELECT d.uid,d.parent_id,d.dirent_id,d.version,d.name_mac,d.creation_time,d.node_id
        FROM kv_dirent_heads h JOIN kv_dirents d ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
        WHERE substr(d.node_id,1,1)!=X'00'";
    for sql in [
        format!("SELECT EXISTS({expected} EXCEPT SELECT * FROM kv_dirent_current)"),
        format!("SELECT EXISTS(SELECT * FROM kv_dirent_current EXCEPT {expected})"),
    ] {
        assert!(!c.query_row(&sql, [], |r| r.get::<_, bool>(0)).unwrap());
    }
    assert!(!c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
}
fn conflict(c: &Connection, own: u64, mac: u8) -> bool {
    c.prepare(KV_NAME_CONFLICT_SQL)
        .unwrap()
        .exists(params![UID, PARENT, id(own), [mac; 32]])
        .unwrap()
}
fn exact(c: &Connection, cursor: Option<KvListCursor>, limit: usize) -> Vec<Vec<u8>> {
    kv_list(c, &UID, &PARENT, cursor, limit)
        .unwrap()
        .into_iter()
        .map(|r| r.exact)
        .collect()
}

#[test]
fn live_projection_tracks_head_history_mutation_cascade_and_rollback() {
    let mut c = connection();
    history(&c, 1, 1, 3, true);
    head(&c, 1, 1);
    history(&c, 1, 2, 4, true);
    head(&c, 1, 2);
    invariant(&c);
    // Editing an obsolete version must never remove the newer current head.
    c.execute(
        "UPDATE kv_dirents SET name_mac=?1,node_id=zeroblob(17) WHERE dirent_id=?2 AND version=1",
        params![[9_u8; 32], id(1)],
    )
    .unwrap();
    invariant(&c);
    assert!(conflict(&c, 0, 4));
    assert!(!conflict(&c, 0, 9));
    c.execute(
        "UPDATE kv_dirents SET name_mac=?1,creation_time=17 WHERE dirent_id=?2 AND version=2",
        params![[8_u8; 32], id(1)],
    )
    .unwrap();
    invariant(&c);
    assert!(conflict(&c, 0, 8));
    c.execute(
        "UPDATE kv_dirents SET node_id=zeroblob(17) WHERE dirent_id=?1 AND version=2",
        [id(1)],
    )
    .unwrap();
    invariant(&c);
    assert!(!conflict(&c, 0, 8));
    c.execute("UPDATE kv_dirents SET node_id=X'0200000000000000000000000000000001' WHERE dirent_id=?1 AND version=2",[id(1)]).unwrap();
    invariant(&c);
    assert!(conflict(&c, 0, 8));
    c.execute(
        "UPDATE kv_dirents SET exact_dirent=X'0708' WHERE dirent_id=?1 AND version=2",
        [id(1)],
    )
    .unwrap();
    assert_eq!(exact(&c, None, 1), vec![vec![7, 8]]);
    head(&c, 1, 1);
    invariant(&c);
    assert!(exact(&c, None, 1).is_empty());
    head(&c, 1, 2);
    invariant(&c);
    let tx = c.transaction().unwrap();
    tx.execute("DELETE FROM kv_dirent_heads", []).unwrap();
    invariant(&tx);
    tx.rollback().unwrap();
    invariant(&c);
    assert!(conflict(&c, 0, 8));
    c.execute(
        "DELETE FROM kv_dirents WHERE dirent_id=?1 AND version=2",
        [id(1)],
    )
    .unwrap();
    invariant(&c);
    assert!(exact(&c, None, 1).is_empty());
    history(&c, 2, 1, 2, true);
    head(&c, 2, 1);
    c.execute("DELETE FROM kv_namespaces", []).unwrap();
    invariant(&c);
    assert!(exact(&c, None, 1).is_empty());
}

#[test]
fn replace_head_with_tombstone_clears_projection_without_recursive_delete_triggers() {
    let c = connection();
    c.pragma_update(None, "recursive_triggers", false).unwrap();
    history(&c, 1, 1, 2, true);
    head(&c, 1, 1);
    history(&c, 1, 2, 2, false);
    c.execute(
        "INSERT OR REPLACE INTO kv_dirent_heads VALUES(?1,?2,?3,2)",
        params![UID, PARENT, id(1)],
    )
    .unwrap();
    invariant(&c);
    assert!(!conflict(&c, 2, 2));
    assert!(exact(&c, None, 1).is_empty());
}

#[test]
fn deferred_head_first_insert_and_primary_key_relocation_work_in_both_orders() {
    for head_first in [false, true] {
        let mut c = connection();
        let tx = c.transaction().unwrap();
        tx.execute_batch("PRAGMA defer_foreign_keys=ON").unwrap();
        if head_first {
            head(&tx, 1, 1);
            history(&tx, 1, 1, 1, true);
        } else {
            history(&tx, 1, 1, 1, true);
            head(&tx, 1, 1);
        }
        invariant(&tx);
        tx.commit().unwrap();
        let tx = c.transaction().unwrap();
        tx.execute_batch("PRAGMA defer_foreign_keys=ON").unwrap();
        let change_head = |c: &Connection| {
            c.execute(
                "UPDATE kv_dirent_heads SET dirent_id=?1,version=2 WHERE dirent_id=?2",
                params![id(2), id(1)],
            )
            .unwrap();
        };
        let change_history = |c: &Connection| {
            c.execute(
                "UPDATE kv_dirents SET dirent_id=?1,version=2 WHERE dirent_id=?2",
                params![id(2), id(1)],
            )
            .unwrap();
        };
        if head_first {
            change_head(&tx);
            change_history(&tx);
        } else {
            change_history(&tx);
            change_head(&tx);
        }
        invariant(&tx);
        tx.commit().unwrap();
        invariant(&c);
        assert!(!conflict(&c, 2, 1));
        assert!(conflict(&c, 1, 1));
        let tx = c.transaction().unwrap();
        tx.execute_batch("PRAGMA defer_foreign_keys=ON").unwrap();
        head(&tx, 9, 1);
        assert!(tx.commit().is_err());
        invariant(&c);
    }
}

#[test]
fn duplicate_names_cursor_boundaries_tombstones_and_historical_results_are_preserved() {
    let c = connection();
    for (n, mac) in [(1, 2), (2, 2), (3, 3), (4, 4)] {
        history(&c, n, 1, mac, true);
        head(&c, n, 1);
    }
    history(&c, 1, 2, 8, false);
    head(&c, 1, 2);
    assert!(conflict(&c, 2, 3));
    assert!(!conflict(&c, 2, 2));
    history(&c, 5, 1, 2, true);
    head(&c, 5, 1);
    assert!(conflict(&c, 2, 2));
    assert_eq!(exact(&c, None, 2), vec![b"2:1".to_vec(), b"5:1".to_vec()]);
    assert_eq!(
        exact(&c, Some(KvListCursor::Mac([2; 32])), 2),
        vec![b"3:1".to_vec(), b"4:1".to_vec()]
    );
    assert_eq!(
        exact(&c, Some(KvListCursor::Time(2)), 3),
        vec![b"2:1".to_vec(), b"5:1".to_vec(), b"3:1".to_vec()]
    );
    let old = kv_dirent_at_name(&c, &UID, &PARENT, 1, &[8; 32])
        .unwrap()
        .unwrap();
    assert_eq!(old.exact, b"1:2");
    assert_eq!(old.exact_directory, Some(vec![1]));
    assert!(kv_dirent_at_name(&c, &UID, &PARENT, 2, &[2; 32])
        .unwrap()
        .is_none());
    invariant(&c);
}

fn schema(c: &Connection) -> Vec<(String, String, Option<String>)> {
    c.prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY type,name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn data(c: &Connection) -> Vec<(String, Vec<String>)> {
    let tables:Vec<String>=c.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name!='metadata_read_dependency' AND name!='kv_dirent_current' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    tables
        .into_iter()
        .map(|table| {
            let columns: Vec<String> = c.prepare("SELECT name FROM pragma_table_info(?1) WHERE name NOT LIKE 'usage_%' AND name!='ciphertext_bytes' ORDER BY cid").unwrap()
                .query_map([table.as_str()],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            let selected=columns.into_iter().map(|v|format!("\"{}\"",v.replace('"',"\"\""))).collect::<Vec<_>>().join(",");
            let mut s=c.prepare(&format!("SELECT {selected} FROM \"{}\"",table.replace('"',"\"\""))).unwrap();
            let columns = s.column_count();
            let mut rows: Vec<String> = s
                .query_map([], |r| {
                    let values: Vec<rusqlite::types::Value> = (0..columns)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<_>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            rows.sort();
            (table, rows)
        })
        .collect()
}
fn downgrade(c: &Connection) {
    c.execute_batch(concat!(
        include_str!("../tests/fixtures/remove_read_dependency.sql"),
        include_str!("../tests/fixtures/remove_kv_usage.sql"),
        include_str!("../tests/fixtures/remove_kv_current.sql")
    ))
    .unwrap();
    c.pragma_update(None, "user_version", 55).unwrap();
}

#[test]
fn genuine_schema55_backfill_preserves_all_base_data_counters_and_history() {
    let mut c = connection();
    for (n, mac, live) in [(1, 3, true), (2, 3, true), (3, 4, false)] {
        history(&c, n, 1, mac, live);
        head(&c, n, 1);
    }
    history(&c, 1, 2, 7, true);
    head(&c, 1, 2);
    c.execute_batch("INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(zeroblob(33),zeroblob(16),X'0102',0,0);
        INSERT INTO chunk_objects(object_id,object_key,digest,encoded_bytes,state) VALUES(zeroblob(32),'legacy',zeroblob(32),12,3);
        INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk,external_digest,external_ciphertext_bytes,external_encoded_bytes,external_object_id)
        VALUES(zeroblob(33),zeroblob(16),0,X'',1,X'',zeroblob(32),10,12,zeroblob(32));
        INSERT INTO sso_identity_challenges(challenge,claim_hash,source_hash,expires_at_ms) VALUES(zeroblob(32),zeroblob(32),zeroblob(32),1000);").unwrap();
    history(&c, 0, 1, 5, true);
    history(&c, 0, 2, 5, false);
    head(&c, 0, 2);
    downgrade(&c);
    assert!(matches!(
        crate::schema::validate_connection(&c),
        Err(Error::SchemaVersion { found: 55 })
    ));
    let before = data(&c);
    // Including a tied historical name: preserve the predecessor's exact bytes,
    // without imposing a new tie-breaker or changing its history index.
    let history_before = super::historical_tests::legacy(&c, &UID, &PARENT, 1, &[3; 32]);
    let legacy_current:Vec<Vec<u8>>=c.prepare("SELECT d.exact_dirent FROM kv_dirent_heads h JOIN kv_dirents d ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version WHERE h.uid=?1 AND h.parent_id=?2 AND substr(d.node_id,1,1)!=X'00' ORDER BY d.name_mac,d.dirent_id LIMIT 10").unwrap().query_map(params![UID,PARENT],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    crate::schema::initialize(&mut c).unwrap();
    invariant(&c);
    assert_eq!(data(&c), before);
    assert_eq!(schema(&c), schema(&connection()));
    assert_eq!(exact(&c, None, 10), legacy_current);
    assert_eq!(
        kv_dirent_at_name(&c, &UID, &PARENT, 1, &[3; 32])
            .unwrap()
            .unwrap(),
        history_before
    );
    crate::schema::initialize(&mut c).unwrap();
    assert_eq!(data(&c), before);
}

#[test]
fn failed_projection_upgrade_rolls_back_schema_version_and_data() {
    for missing_target in [false, true] {
        let mut c = connection();
        history(&c, 1, 1, 1, true);
        head(&c, 1, 1);
        downgrade(&c);
        if missing_target {
            c.pragma_update(None, "foreign_keys", false).unwrap();
            c.execute("DELETE FROM kv_dirents", []).unwrap();
            c.pragma_update(None, "foreign_keys", true).unwrap();
        } else {
            c.execute_batch("CREATE TABLE kv_dirent_current_time(sentinel TEXT)")
                .unwrap();
        }
        let before = data(&c);
        let old_schema = schema(&c);
        assert!(crate::schema::initialize(&mut c).is_err());
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            55
        );
        assert_eq!(schema(&c), old_schema);
        assert_eq!(data(&c), before);
    }
}

fn cost(c: &Connection, sql: &str, parameters: impl rusqlite::Params) -> (i32, Vec<String>) {
    let mut s = c.prepare(sql).unwrap();
    {
        let mut rows = s.query(parameters).unwrap();
        while rows.next().unwrap().is_some() {}
    }
    let steps = s.get_status(StatementStatus::VmStep);
    let explain = c
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(
            rusqlite::params_from_iter(std::iter::repeat_n(
                rusqlite::types::Value::Null,
                s.parameter_count(),
            )),
            |r| r.get(3),
        )
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    (steps, explain)
}
#[test]
fn production_conflict_list_and_update_work_is_independent_of_history_heads_and_tombstones() {
    let mut baseline = None;
    for (history_size, heads, tombstones) in [
        (0, 0, 0),
        (100, 0, 0),
        (1000, 0, 0),
        (10000, 0, 0),
        (0, 100, 0),
        (0, 1000, 0),
        (0, 10000, 0),
        (0, 0, 10000),
        (10000, 10000, 10000),
    ] {
        let mut c = connection();
        let tx = c.transaction().unwrap();
        for version in 1..=history_size {
            history(&tx, 1, version, 1, true);
        }
        history(&tx, 1, history_size + 1, 1, false);
        head(&tx, 1, history_size + 1);
        for n in 10..10 + heads {
            history(&tx, n, 1, 100, true);
            head(&tx, n, 1);
        }
        for n in 20000..20000 + tombstones {
            history(&tx, n, 1, 0, false);
            head(&tx, n, 1);
        }
        for n in 2..=4 {
            history(&tx, n, 1, n as u8, true);
            head(&tx, n, 1);
        }
        history(&tx, 2, 2, 2, true);
        tx.commit().unwrap();
        let measures = [
            cost(
                &c,
                KV_NAME_CONFLICT_SQL,
                params![UID, PARENT, id(9), [1_u8; 32]],
            ),
            cost(
                &c,
                KV_NAME_CONFLICT_SQL,
                params![UID, PARENT, id(9), [2_u8; 32]],
            ),
            cost(&c, KV_LIST_INITIAL_SQL, params![UID, PARENT, 1]),
            cost(&c, KV_LIST_MAC_SQL, params![UID, PARENT, [2_u8; 32], 1]),
            cost(&c, KV_LIST_TIME_SQL, params![UID, PARENT, 3, 1]),
        ];
        assert!(!conflict(&c, 9, 1));
        assert!(conflict(&c, 9, 2));
        assert_eq!(exact(&c, None, 1), vec![b"2:1".to_vec()]);
        let mut update=c.prepare("UPDATE kv_dirent_heads SET version=2 WHERE uid=?1 AND parent_id=?2 AND dirent_id=?3").unwrap();
        update.execute(params![UID, PARENT, id(2)]).unwrap();
        let mut work: Vec<i32> = measures.iter().map(|(steps, _)| *steps).collect();
        work.push(update.get_status(StatementStatus::VmStep));
        let mut old_version = c.prepare("UPDATE kv_dirents SET name_mac=?1 WHERE uid=?2 AND parent_id=?3 AND dirent_id=?4 AND version=1").unwrap();
        old_version
            .execute(params![[9_u8; 32], UID, PARENT, id(2)])
            .unwrap();
        work.push(old_version.get_status(StatementStatus::VmStep));
        eprintln!("current projection history={history_size} heads={heads} tombstones={tombstones}: {measures:?}; update={}",work.last().unwrap());
        for (_, plan) in &measures {
            assert!(
                plan.iter()
                    .any(|line| line.contains("kv_dirent_current_name")
                        || line.contains("kv_dirent_current_time")),
                "{plan:?}"
            );
            assert!(
                !plan.iter().any(|line| line.contains("TEMP B-TREE")),
                "{plan:?}"
            );
        }
        if let Some(expected) = &baseline {
            assert_eq!(&work, expected);
        } else {
            baseline = Some(work);
        }
        invariant(&c);
    }
}
