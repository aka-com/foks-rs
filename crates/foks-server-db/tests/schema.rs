mod common;

use foks_server_db::{Config, Database, Error, APPLICATION_ID, SCHEMA_VERSION};
use rusqlite::Connection;

#[test]
fn every_connection_has_authoritative_pragmas() {
    let database = common::TestDatabase::new();
    let pragmas = database.database.pragmas().unwrap();
    assert!(pragmas.foreign_keys);
    assert_eq!(pragmas.journal_mode, "wal");
    assert_eq!(pragmas.synchronous, 2);
    assert!(!pragmas.trusted_schema);
    assert_eq!(pragmas.busy_timeout_millis, 5000);
    assert!(database.database.integrity_check().unwrap());

    let reader = database.database.open_reader().unwrap();
    assert!(reader
        .pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))
        .unwrap());
    assert_eq!(
        reader
            .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
            .unwrap(),
        APPLICATION_ID
    );
    assert_eq!(
        reader
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
}

#[test]
fn foreign_and_future_databases_are_refused() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("foreign.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "application_id", 0x1234_5678)
        .unwrap();
    drop(connection);
    assert!(matches!(
        Database::open(&path, Config::default()),
        Err(Error::ApplicationId { .. })
    ));

    let path = directory.path().join("future.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    connection
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    drop(connection);
    assert!(matches!(
        Database::open(&path, Config::default()),
        Err(Error::SchemaVersion { .. })
    ));
}

#[test]
fn schema_constraints_reject_bad_lengths_and_negative_values() {
    let database = common::TestDatabase::new();
    let path = database.path.clone();
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    assert!(connection
        .execute(
            "INSERT INTO merkle_nodes(node_hash, exact_node) VALUES (?1, ?2)",
            rusqlite::params![vec![0_u8; 31], b"node"],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO hostchain_links(seqno, link_hash, exact_link) VALUES (-1, ?1, ?2)",
            rusqlite::params![vec![0_u8; 32], b"link"],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO devices(device_id, uid, active, hepk_fingerprint, exact_hepk)
             VALUES (?1, ?2, 1, ?3, ?4)",
            rusqlite::params![vec![4_u8; 33], vec![1_u8; 33], vec![0_u8; 32], b"hepk"],
        )
        .is_err());
}
