use super::*;
use crate::schema::read_dependency::{READ_DEPENDENCY_TABLES, READ_DEPENDENCY_TRIGGERS};

fn connection() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    crate::schema::initialize(&mut connection).unwrap();
    connection
}
fn coordinate(c: &Connection) -> ReadDependency {
    read(c).unwrap()
}
fn seed(c: &Connection) {
    c.execute_batch("INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33));
        INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'01');
        INSERT INTO kv_directory_heads VALUES(zeroblob(33),zeroblob(16),1);
        INSERT INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),zeroblob(16),1,1,X'0300000000000000000000000000000000',zeroblob(32),0,X'01');
        INSERT INTO kv_dirent_heads VALUES(zeroblob(33),zeroblob(16),zeroblob(16),1);").unwrap();
}

#[test]
fn schema_has_exact_event_hooks_and_checks_existing_projection_contract() {
    let c = connection();
    verify(&c).unwrap();
    for table in READ_DEPENDENCY_TABLES {
        for event in ["insert", "update", "delete"] {
            let name = format!("metadata_read_{table}_{event}");
            let body: String = c.query_row("SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1 AND tbl_name=?2",params![name,table], |r| r.get(0)).unwrap();
            assert!(
                body.contains("RAISE(ABORT,'read dependency row missing, invalid or exhausted')")
            );
        }
    }
    c.execute_batch(
        "DROP TRIGGER kv_current_head_insert;
        CREATE TRIGGER kv_current_head_insert AFTER INSERT ON kv_dirent_heads BEGIN SELECT 1; END;",
    )
    .unwrap();
    assert!(matches!(verify(&c), Err(Error::ReadDependencySchema(_))));
    let c = connection();
    c.execute_batch(
        "CREATE TRIGGER extra_reader_hole AFTER UPDATE ON kv_dirents BEGIN SELECT 1; END",
    )
    .unwrap();
    assert!(matches!(verify(&c), Err(Error::ReadDependencySchema(_))));
}

#[test]
fn reserved_prefix_filter_does_not_hide_authoritative_extension_triggers() {
    let c = connection();
    let expected_hash = *verify(&c).unwrap().contract_hash();
    // The underscore in SQLite's internal prefix is literal. LIKE 'sqlite_%'
    // would also hide these valid, non-internal object names.
    c.execute_batch("CREATE TABLE sqliteX_extension(value INTEGER)")
        .unwrap();
    assert_eq!(*verify(&c).unwrap().contract_hash(), expected_hash);
    for (name, table) in [
        ("sqliteX_reader_hole", "kv_dirents"),
        ("SQLITEZ_reader_hole", "kv_dirents"),
        ("mixed_case_target", "KV_DIRENTS"),
    ] {
        c.execute_batch(&format!(
            "CREATE TRIGGER {name} AFTER UPDATE ON {table} BEGIN SELECT 1; END"
        ))
        .unwrap();
        assert!(matches!(verify(&c), Err(Error::ReadDependencySchema(_))));
        c.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
        assert_eq!(*verify(&c).unwrap().contract_hash(), expected_hash);
    }
    c.execute_batch("CREATE TABLE METADATA_READ_extra(value INTEGER)")
        .unwrap();
    assert!(matches!(verify(&c), Err(Error::ReadDependencySchema(_))));
}

#[test]
fn schema_enumeration_rejects_malformed_public_rows_but_keeps_valid_autoindexes() {
    for (column, expression, expected_type) in [
        ("name", "NULL", "null"),
        ("name", "CAST('extension' AS BLOB)", "blob"),
        ("name", "CAST('sqlite_hidden' AS BLOB)", "blob"),
        ("sql", "NULL", "null"),
        (
            "sql",
            "CAST('CREATE TABLE extension(v UNIQUE)' AS BLOB)",
            "blob",
        ),
    ] {
        let c = connection();
        let expected_hash = *verify(&c).unwrap().contract_hash();
        c.execute_batch("CREATE TABLE extension(v UNIQUE)").unwrap();
        let internal: (String, String) = c.query_row(
            "SELECT typeof(name),typeof(sql) FROM sqlite_schema WHERE name='sqlite_autoindex_extension_1'",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(internal, ("text".into(), "null".into()));
        assert_eq!(*verify(&c).unwrap().contract_hash(), expected_hash);
        c.execute_batch("PRAGMA writable_schema=ON").unwrap();
        c.execute(
            &format!("UPDATE sqlite_schema SET {column}={expression} WHERE type='table' AND tbl_name='extension'"),
            [],
        ).unwrap();
        c.execute_batch("PRAGMA writable_schema=OFF").unwrap();
        let actual_type: String = c.query_row(
            &format!("SELECT typeof({column}) FROM sqlite_schema WHERE type='table' AND tbl_name='extension'"),
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(actual_type, expected_type, "{column}={expression}");
        assert!(verify(&c).is_err(), "{column}={expression}");
    }
}

#[test]
fn exact_payload_edit_and_projection_cascade_advance_atomically() {
    let mut c = connection();
    seed(&c);
    let before = coordinate(&c);
    c.execute("UPDATE kv_dirents SET exact_dirent=X'02'", [])
        .unwrap();
    assert!(coordinate(&c).revision > before.revision);
    let before = coordinate(&c);
    {
        let tx = c.transaction().unwrap();
        tx.execute("DELETE FROM kv_namespaces", []).unwrap();
        assert!(coordinate(&tx).revision > before.revision);
        assert_eq!(
            tx.query_row("SELECT count(*) FROM kv_dirent_current", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        // Dropping the outer transaction rolls source, projection, counters and
        // every individual dependency increment back together.
    }
    assert_eq!(coordinate(&c), before);
    assert_eq!(
        c.query_row("SELECT count(*) FROM kv_dirent_current", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    c.execute("DELETE FROM kv_namespaces", []).unwrap();
    assert!(coordinate(&c).revision > before.revision);
}

#[test]
fn overflow_and_missing_coordinate_abort_source_mutation_not_just_counter() {
    for exhausted in [false, true] {
        let c = connection();
        seed(&c);
        if exhausted {
            c.execute(
                "UPDATE metadata_read_dependency SET revision=?1",
                [i64::MAX],
            )
            .unwrap();
        } else {
            c.execute_batch(
                "DROP TRIGGER metadata_read_no_delete; DELETE FROM metadata_read_dependency;",
            )
            .unwrap();
        }
        assert!(c
            .execute("UPDATE kv_dirents SET exact_dirent=X'ff'", [])
            .is_err());
        assert_eq!(
            c.query_row("SELECT exact_dirent FROM kv_dirents", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            [1]
        );
        if exhausted {
            assert_eq!(coordinate(&c).revision, i64::MAX as u64);
        } else {
            assert!(read(&c).is_err());
        }
    }
    let c = connection();
    seed(&c);
    c.execute(
        "UPDATE metadata_read_dependency SET revision=?1",
        [i64::MAX - 1],
    )
    .unwrap();
    // Cascading accounting/projection work requires more than one increment.
    assert!(c.execute("DELETE FROM kv_namespaces", []).is_err());
    assert_eq!(coordinate(&c).revision, (i64::MAX - 1) as u64);
    assert_eq!(
        c.query_row("SELECT count(*) FROM kv_dirent_current", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn dependency_table_cannot_disappear_rewind_or_accept_invalid_coordinates() {
    let c = connection();
    seed(&c);
    assert!(c
        .execute("DELETE FROM metadata_read_dependency", [])
        .is_err());
    assert!(c
        .execute("UPDATE metadata_read_dependency SET revision=1", [])
        .is_err());
    for sql in [
        "INSERT OR REPLACE INTO metadata_read_dependency VALUES(1,randomblob(32),1)",
        "UPDATE metadata_read_dependency SET revision=1.5",
        "UPDATE metadata_read_dependency SET revision=0",
        "UPDATE metadata_read_dependency SET incarnation=zeroblob(32)",
        "UPDATE metadata_read_dependency SET incarnation=zeroblob(31)",
        "UPDATE metadata_read_dependency SET singleton=2",
    ] {
        assert!(c.execute(sql, []).is_err(), "{sql}");
    }
}

#[test]
fn genuine_58_migration_preserves_data_and_atomic_failure_does_not_publish_59() {
    let mut c = connection();
    seed(&c);
    c.execute_batch(include_str!(
        "../../tests/fixtures/remove_read_dependency.sql"
    ))
    .unwrap();
    c.pragma_update(None, "user_version", 58).unwrap();
    let exact: Vec<u8> = c
        .query_row("SELECT exact_dirent FROM kv_dirents", [], |r| r.get(0))
        .unwrap();
    c.execute_batch("CREATE TABLE metadata_read_dependency(collision INTEGER)")
        .unwrap();
    let before = schema_objects(&c).unwrap();
    assert!(crate::schema::initialize(&mut c).is_err());
    assert_eq!(schema_objects(&c).unwrap(), before);
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        58
    );
    c.execute_batch("DROP TABLE metadata_read_dependency")
        .unwrap();
    crate::schema::initialize(&mut c).unwrap();
    verify(&c).unwrap();
    let coordinate = coordinate(&c);
    assert_ne!(coordinate.incarnation, [0; 32]);
    assert_eq!(coordinate.revision, 1);
    assert_eq!(
        c.query_row("SELECT exact_dirent FROM kv_dirents", [], |r| r
            .get::<_, Vec<u8>>(0))
            .unwrap(),
        exact
    );
    crate::schema::initialize(&mut c).unwrap();
    assert_eq!(read(&c).unwrap(), coordinate);
}

#[test]
fn every_generated_event_body_refuses_without_singleton_and_rolls_back() {
    // Exercise the actual generated bodies independently of each domain table's
    // data constraints; real domain mutation/cascade tests cover integration.
    let mut c = Connection::open_in_memory().unwrap();
    c.execute_batch(crate::READ_DEPENDENCY_SCHEMA).unwrap();
    for table in READ_DEPENDENCY_TABLES {
        c.execute_batch(&format!("CREATE TABLE {table}(v INTEGER)"))
            .unwrap();
    }
    c.execute_batch(READ_DEPENDENCY_TRIGGERS).unwrap();
    for table in READ_DEPENDENCY_TABLES {
        assert!(c
            .execute(&format!("INSERT INTO {table} VALUES(1)"), [])
            .is_err());
    }
    c.execute(
        "INSERT INTO metadata_read_dependency VALUES(1,?1,1)",
        [[1u8; 32].as_slice()],
    )
    .unwrap();
    for table in READ_DEPENDENCY_TABLES {
        let tx = c.transaction().unwrap();
        let prior = coordinate(&tx).revision;
        for (expected, sql) in (prior + 1..).zip([
            format!("INSERT INTO {table} VALUES(1)"),
            format!("UPDATE {table} SET v=2"),
            format!("DELETE FROM {table}"),
        ]) {
            tx.execute(&sql, []).unwrap();
            assert_eq!(coordinate(&tx).revision, expected);
        }
        tx.rollback().unwrap();
        assert_eq!(coordinate(&c).revision, 1);
    }
}

#[test]
fn migration_validates_existing_hooks_before_publishing_any_new_coordinate() {
    let mut c = connection();
    seed(&c);
    c.execute_batch(include_str!(
        "../../tests/fixtures/remove_read_dependency.sql"
    ))
    .unwrap();
    c.pragma_update(None, "user_version", 58).unwrap();
    c.execute_batch("DROP TRIGGER kv_current_head_delete")
        .unwrap();
    let prior = schema_objects(&c).unwrap();
    assert!(matches!(
        crate::schema::initialize(&mut c),
        Err(Error::ReadDependencySchema(_))
    ));
    assert_eq!(schema_objects(&c).unwrap(), prior);
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        58
    );
    assert!(!c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='metadata_read_dependency')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
}

#[test]
fn row_revision_overhead_does_not_scan_retained_history() {
    use rusqlite::StatementStatus;
    let mut with_hooks = Vec::new();
    let mut baseline = Vec::new();
    let mut increments = Vec::new();
    for count in [1i64, 100, 10_000] {
        let c = connection();
        seed(&c);
        let tx = c.unchecked_transaction().unwrap();
        for version in 2..=count {
            tx.execute("INSERT INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),zeroblob(16),?1,1,X'0300000000000000000000000000000000',zeroblob(32),0,X'01')",[version]).unwrap();
        }
        tx.commit().unwrap();
        let before = coordinate(&c).revision;
        let mut statement=c.prepare("UPDATE kv_dirents SET exact_dirent=X'02' WHERE uid=zeroblob(33) AND parent_id=zeroblob(16) AND dirent_id=zeroblob(16) AND version=1").unwrap();
        statement.execute([]).unwrap();
        with_hooks.push(statement.get_status(StatementStatus::VmStep));
        drop(statement);
        increments.push(coordinate(&c).revision - before);
        c.execute_batch(include_str!(
            "../../tests/fixtures/remove_read_dependency.sql"
        ))
        .unwrap();
        let mut statement=c.prepare("UPDATE kv_dirents SET exact_dirent=X'01' WHERE uid=zeroblob(33) AND parent_id=zeroblob(16) AND dirent_id=zeroblob(16) AND version=1").unwrap();
        statement.execute([]).unwrap();
        baseline.push(statement.get_status(StatementStatus::VmStep));
    }
    eprintln!(
        "read dependency exact-payload update: history=[1,100,10000], vm_with_hooks={with_hooks:?}, vm_baseline={baseline:?}, revision_increments={increments:?}"
    );
    assert!(increments.iter().all(|n| *n > 0));
    assert!(with_hooks.iter().max().unwrap() - with_hooks.iter().min().unwrap() < 100);
    assert!(baseline.iter().max().unwrap() - baseline.iter().min().unwrap() < 100);
    assert!(with_hooks
        .iter()
        .zip(baseline)
        .all(|(with, without)| *with > without));
}

#[test]
fn shared_streaming_contract_preserves_full_schema59_legacy_hash_and_tokens() {
    let c = connection();
    let mut statement = c.prepare(SCHEMA_OBJECTS_SQL).unwrap();
    let all = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let old = test_oracle::hash(all.iter().map(|(kind, name, table, sql)| {
        (kind.as_str(), name.as_str(), table.as_str(), sql.as_str())
    }))
    .unwrap();
    let actual = verify(&c).unwrap();
    assert_eq!(*actual.contract_hash(), old);
    // Captured from the independent schema59 algorithm before extraction.
    assert_eq!(
        old,
        [
            0x44, 0x4f, 0xc9, 0x34, 0xa1, 0x33, 0x6a, 0x96, 0x7d, 0xb1, 0xd8, 0x9e, 0x07, 0x86,
            0x32, 0x18, 0x3c, 0x2c, 0x5b, 0x00, 0x1a, 0xdb, 0x42, 0x2d, 0xf7, 0x07, 0xa6, 0x43,
            0x9b, 0x5e, 0xdb, 0x3b
        ],
    );
    for (_, _, _, sql) in &all {
        // Exact original spelling and comment/spacing variants are qualified by
        // both the old oracle and the extracted streaming comparator.
        let variant = format!(" /*before*/ {sql} --after");
        assert_eq!(
            test_oracle::tokens(sql).unwrap(),
            test_oracle::tokens(&variant).unwrap()
        );
        assert!(foks_schema_contract::sql_equal(sql, &variant).unwrap());
    }
}
