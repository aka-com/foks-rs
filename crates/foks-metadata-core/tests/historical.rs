#![cfg(feature = "native")]
use foks_metadata_core::{
    catalog::{Statement, Value, HISTORICAL_NAME_SELECTION_SQL},
    native::NativeSql,
    Sql,
};
use foks_server_db::{Config, Database};
use rusqlite::{params, Connection, StatementStatus};
#[test]
fn shared_historical_selection_and_bounded_projection_preserve_native_lookup() {
    assert_eq!(
        HISTORICAL_NAME_SELECTION_SQL,
        foks_server_db::KV_HISTORICAL_NAME_SELECTION_SQL
    );
    let mut costs = Vec::new();
    for count in [1i64, 100, 10_000] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let database = Database::open(&path, Config::default()).unwrap();
        drop(database);
        let mut c = Connection::open(path).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        c.execute_batch("INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33)); INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'0102');").unwrap();
        let tx = c.transaction().unwrap();
        for version in 1..=count {
            tx.execute("INSERT INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),zeroblob(16),?1,1,zeroblob(17),zeroblob(32),1,?2)",params![version,version.to_be_bytes()]).unwrap();
        }
        tx.commit().unwrap();
        let tx = c.transaction().unwrap();
        let mut sql = NativeSql::new(&tx).unwrap();
        let params = [
            Value::Blob(vec![0; 33]),
            Value::Blob(vec![0; 16]),
            Value::Blob(vec![0; 32]),
            Value::Integer(1),
        ];
        let row = sql
            .query(Statement::HistoricalName, &params)
            .unwrap()
            .unwrap();
        assert_eq!(
            row,
            vec![
                Value::Integer(8),
                Value::Blob(count.to_be_bytes().to_vec()),
                Value::Blob(vec![0; 17]),
                Value::Integer(2),
                Value::Blob(vec![1, 2])
            ]
        );
        let mut q = tx.prepare(Statement::HistoricalName.spec().sql).unwrap();
        {
            let mut rows = q
                .query(params![[0u8; 33], [0u8; 16], [0u8; 32], "1"])
                .unwrap();
            assert!(rows.next().unwrap().is_some());
            assert!(rows.next().unwrap().is_none());
        }
        let positive_steps = q.get_status(StatementStatus::VmStep);
        drop(q);
        let mut current_costs = vec![positive_steps];
        for (mac, directory) in [(0u8, "2"), (1u8, "1")] {
            let mut q = tx.prepare(Statement::HistoricalName.spec().sql).unwrap();
            {
                let mut rows = q
                    .query(params![[0u8; 33], [0u8; 16], [mac; 32], directory])
                    .unwrap();
                assert!(rows.next().unwrap().is_none());
            }
            current_costs.push(q.get_status(StatementStatus::VmStep));
        }
        costs.push(current_costs);
        let plan = tx
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {}",
                Statement::HistoricalName.spec().sql
            ))
            .unwrap()
            .query_map(params![[0u8; 33], [0u8; 16], [0u8; 32], "1"], |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(
            !plan.iter().any(|p| p.contains("USE TEMP B-TREE")),
            "{plan:?}"
        );
        tx.execute(
            "UPDATE kv_directories SET exact_directory=zeroblob(8192)",
            [],
        )
        .unwrap();
        let row = sql
            .query(Statement::HistoricalName, &params)
            .unwrap()
            .unwrap();
        assert_eq!(row[3], Value::Integer(8192));
        assert_eq!(
            row[4],
            Value::Null,
            "oversized payload must not cross private host boundary"
        );
    }
    assert!(costs.iter().all(|c| c == &costs[0]), "{costs:?}");
}
