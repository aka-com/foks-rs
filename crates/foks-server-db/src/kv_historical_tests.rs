//! Schema56 lookup is an oracle, not a protocol tie-order guarantee.
use super::*;
use rusqlite::{Connection, StatementStatus};
const UID: [u8; 33] = [0; 33];
const PARENT: [u8; 16] = [0; 16];
pub(super) const LEGACY_SQL: &str = "SELECT e.exact_dirent,e.node_id,d.exact_directory FROM kv_dirents e JOIN kv_directories d ON d.uid=e.uid AND d.directory_id=e.parent_id AND d.version=e.directory_version WHERE e.uid=?1 AND e.parent_id=?2 AND e.name_mac=?3 AND e.directory_version=?4 ORDER BY e.version DESC LIMIT 1";
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
fn id(n: u64) -> [u8; 16] {
    let mut id = [0; 16];
    id[8..].copy_from_slice(&n.to_be_bytes());
    id
}
fn insert(c: &Connection, n: u64, version: i64, directory: i64, name: u8, tombstone: bool) {
    let mut node = [0u8; 17];
    if !tombstone {
        node[0] = 3;
        node[1..].copy_from_slice(&id(n));
    }
    c.execute(
        "INSERT INTO kv_dirents VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            UID,
            PARENT,
            id(n),
            version,
            directory,
            node,
            [name; 32],
            version,
            format!("{n}:{version}:{directory}:{tombstone}").as_bytes()
        ],
    )
    .unwrap();
}
pub(super) fn legacy(
    c: &Connection,
    uid: &[u8],
    parent: &[u8; 16],
    directory: u64,
    name: &[u8; 32],
) -> StoredKvDirent {
    c.query_row(
        LEGACY_SQL,
        params![uid, parent, name, i64::try_from(directory).unwrap()],
        |r| {
            let node: Vec<u8> = r.get(1)?;
            Ok(StoredKvDirent {
                exact: r.get(0)?,
                node_id: node.try_into().unwrap(),
                exact_directory: Some(r.get(2)?),
            })
        },
    )
    .unwrap()
}
fn downgrade(c: &Connection) {
    c.execute_batch(concat!(
        include_str!("../tests/fixtures/remove_kv_usage.sql"),
        include_str!("../tests/fixtures/remove_kv_historical.sql")
    ))
    .unwrap();
    c.pragma_update(None, "user_version", 56).unwrap();
}
fn schema(c: &Connection) -> Vec<(String, String, Option<String>)> {
    c.prepare("SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_stat%' ORDER BY type,name").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}
fn data(c: &Connection) -> Vec<(String, Vec<String>)> {
    let names:Vec<String>=c.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let columns: Vec<String> = c.prepare("SELECT name FROM pragma_table_info(?1) WHERE name NOT LIKE 'usage_%' AND name!='ciphertext_bytes' ORDER BY cid").unwrap()
                .query_map([name.as_str()],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            let selected=columns.into_iter().map(|v|format!("\"{}\"",v.replace('"',"\"\""))).collect::<Vec<_>>().join(",");
            let mut q=c.prepare(&format!("SELECT {selected} FROM \"{}\"",name.replace('"',"\"\""))).unwrap();
            let cols = q.column_count();
            let mut rows: Vec<String> = q
                .query_map([], |r| {
                    let values: Vec<rusqlite::types::Value> = (0..cols)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<_>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            rows.sort();
            (name, rows)
        })
        .collect()
}
#[test]
fn schema56_tie_oracle_and_new_query_preserve_complete_historical_result() {
    for reverse in [false, true] {
        for analyze in [false, true] {
            for tombstone_first in [false, true] {
                let mut c = connection();
                downgrade(&c);
                let order = if reverse { [2, 1] } else { [1, 2] };
                for n in order {
                    insert(&c, n, 7, 1, 3, (n == 1) == tombstone_first);
                }
                // Newer versions with another name/directory version cannot win.
                insert(&c, 1, 8, 2, 3, false);
                insert(&c, 2, 9, 1, 4, false);
                // Exercise planner choices with other names/history and live heads.
                for n in 10..110 {
                    insert(&c, n, 1, 1, 9, false);
                }
                c.execute(
                    "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,9)",
                    params![UID, PARENT, id(2)],
                )
                .unwrap();
                if analyze {
                    c.execute_batch("ANALYZE").unwrap();
                }
                let before = legacy(&c, &UID, &PARENT, 1, &[3; 32]);
                assert_eq!(
                    before.exact,
                    format!("1:7:1:{tombstone_first}").as_bytes(),
                    "STOP: supported native baseline tie differs from proposed ascending-ID choice"
                );
                let before_data = data(&c);
                crate::schema::initialize(&mut c).unwrap();
                let after = kv_dirent_at_name(&c, &UID, &PARENT, 1, &[3; 32])
                    .unwrap()
                    .unwrap();
                assert_eq!(after, before);
                assert_eq!(data(&c), before_data);
                assert_eq!(after.exact_directory, Some(vec![1]));
                assert!(kv_dirent_at_name(&c, &UID, &PARENT, 2, &[4; 32])
                    .unwrap()
                    .is_none());
                assert_eq!(schema(&c), schema(&connection()));
            }
        }
    }
}
#[test]
fn historical_index_upgrade_rolls_back_on_collision_and_reopens_canonically() {
    let mut c = connection();
    insert(&c, 1, 1, 1, 1, false);
    downgrade(&c);
    c.execute_batch("CREATE TABLE kv_dirent_historical_name(sentinel TEXT)")
        .unwrap();
    let old_schema = schema(&c);
    let old_data = data(&c);
    assert!(crate::schema::initialize(&mut c).is_err());
    assert_eq!(schema(&c), old_schema);
    assert_eq!(data(&c), old_data);
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        56
    );
    c.execute_batch("DROP TABLE kv_dirent_historical_name")
        .unwrap();
    crate::schema::initialize(&mut c).unwrap();
    crate::schema::initialize(&mut c).unwrap();
    assert_eq!(schema(&c), schema(&connection()));
    assert_eq!(
        data(&c),
        old_data
            .into_iter()
            .filter(|(name, _)| name != "kv_dirent_historical_name")
            .collect::<Vec<_>>()
    );
    assert!(!c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
}
fn cost(c: &Connection, sql: &str, name: u8, directory: i64) -> (i32, Vec<String>) {
    let mut q = c.prepare(sql).unwrap();
    {
        let mut rows = q
            .query(params![UID, PARENT, [name; 32], directory])
            .unwrap();
        while rows.next().unwrap().is_some() {}
    }
    let steps = q.get_status(StatementStatus::VmStep);
    let plan = c
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(params![UID, PARENT, [name; 32], directory], |r| r.get(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    (steps, plan)
}
fn write_cost(c: &Connection) -> Vec<i32> {
    let mut result = Vec::new();
    let mut q = c
        .prepare("INSERT INTO kv_dirents VALUES(?1,?2,?3,1,1,?4,?5,1,X'01')")
        .unwrap();
    q.execute(params![UID, PARENT, id(99_999), [0u8; 17], [9u8; 32]])
        .unwrap();
    result.push(q.get_status(StatementStatus::VmStep));
    let mut q=c.prepare("UPDATE kv_dirents SET name_mac=?4,creation_time=2 WHERE uid=?1 AND parent_id=?2 AND dirent_id=?3").unwrap();
    q.execute(params![UID, PARENT, id(99_999), [8u8; 32]])
        .unwrap();
    result.push(q.get_status(StatementStatus::VmStep));
    let mut q = c
        .prepare("DELETE FROM kv_dirents WHERE uid=?1 AND parent_id=?2 AND dirent_id=?3")
        .unwrap();
    q.execute(params![UID, PARENT, id(99_999)]).unwrap();
    result.push(q.get_status(StatementStatus::VmStep));
    result
}
#[test]
fn production_historical_query_is_history_independent_and_preserves_large_native_payloads() {
    let mut costs = Vec::new();
    for count in [1i64, 100, 10_000] {
        let mut c = connection();
        downgrade(&c);
        let tx = c.transaction().unwrap();
        for version in 1..=count {
            insert(&tx, 1, version, 1, 1, false);
            insert(&tx, 2, version, 2, 1, false);
            insert(&tx, u64::try_from(version + 10).unwrap(), 1, 1, 3, false);
        }
        tx.commit().unwrap();
        let old_cost = cost(&c, LEGACY_SQL, 1, 1).0;
        let old_write_cost = write_cost(&c);
        let before_pages: i64 = c
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        crate::schema::initialize(&mut c).unwrap();
        let after_pages: i64 = c
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        let mut current = Vec::new();
        for (name, dir) in [(1, 1), (1, 2), (1, 3), (2, 1)] {
            let (steps, plan) = cost(&c, KV_HISTORICAL_NAME_SQL.as_str(), name, dir);
            assert!(
                plan.iter().any(|p| p.contains("kv_dirent_historical_name")),
                "{plan:?}"
            );
            assert!(
                !plan.iter().any(|p| p.contains("USE TEMP B-TREE")),
                "{plan:?}"
            );
            current.push(steps);
        }
        let new_write_cost = write_cost(&c);
        eprintln!("history={count} old_steps={old_cost} new_steps={current:?} old_write={old_write_cost:?} new_write={new_write_cost:?} added_index_pages={}",after_pages-before_pages);
        costs.push(current);
        c.execute(
            "UPDATE kv_directories SET exact_directory=zeroblob(8192) WHERE version=1",
            [],
        )
        .unwrap();
        assert_eq!(
            kv_dirent_at_name(&c, &UID, &PARENT, 1, &[1; 32])
                .unwrap()
                .unwrap()
                .exact_directory
                .unwrap()
                .len(),
            8192
        );
    }
    assert!(costs.iter().all(|c| c == &costs[0]), "{costs:?}");
}
