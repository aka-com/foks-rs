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

#[test]
fn older_than_supported_predecessor_is_refused_without_upgrading() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("older.sqlite");
    let c = Connection::open(&path).unwrap();
    c.pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    c.pragma_update(None, "user_version", 42).unwrap();
    assert!(matches!(
        Database::open(&path, Config::default()),
        Err(Error::SchemaVersion { found: 42 })
    ));
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        42
    );
}

const EXPIRY_INDEXES: [&str; 7] = [
    "names_reservation_expiry",
    "team_names_reservation_expiry",
    "recovery_challenges_cleanup",
    "team_view_tokens_expiry",
    "team_view_challenges_expiry",
    "team_admin_tokens_expiry",
    "log_sends_created_at",
];

fn schema_objects(connection: &Connection) -> Vec<(String, String, Option<String>)> {
    connection
        .prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY type,name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn predecessor(connection: &Connection, version: i64) {
    assert!(matches!(version, 43..=47));
    if version <= 44 {
        for name in EXPIRY_INDEXES {
            connection
                .execute_batch(&format!("DROP INDEX {name}"))
                .unwrap();
        }
    }
    if version == 43 {
        connection
            .execute_batch(
                "DROP TRIGGER rt_membership_insert;
             DROP TRIGGER rt_membership_delete;
             DROP TRIGGER rt_membership_update;
             DROP TRIGGER rt_team_access_update;
             ALTER TABLE rt_user_inboxes DROP COLUMN reconcile_dirty;",
            )
            .unwrap();
    }
    if version <= 45 {
        connection
            .execute_batch("ALTER TABLE sso_policy RENAME COLUMN blocked_reason TO fence;")
            .unwrap();
    }
    if version <= 46 {
        connection
            .execute_batch(
                "DROP INDEX sso_session_source;
                 ALTER TABLE sso_sessions RENAME COLUMN source_hash TO admission_hash;
                 CREATE INDEX sso_session_admission
                 ON sso_sessions(host, admission_hash, expires_at_ms);
                 ALTER TABLE sso_identity_challenges
                 RENAME COLUMN source_hash TO admission_hash;",
            )
            .unwrap();
    }
    connection
        .pragma_update(None, "user_version", version)
        .unwrap();
}

#[test]
fn writer_upgrades_supported_predecessors_and_preserves_data_and_index_definitions() {
    let fresh = common::TestDatabase::new();
    let reference = Connection::open(&fresh.path).unwrap();
    let indexes = |connection: &Connection| {
        EXPIRY_INDEXES.map(|name| {
            connection
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='index' AND name=?1",
                    [name],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        })
    };
    let expected = indexes(&reference);
    for version in [43, 44, 45, 46, 47] {
        let test = common::TestDatabase::new();
        let path = test.path.clone();
        drop(test.database);
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        predecessor(&connection, version);
        connection.execute_batch(
            "INSERT INTO recovery_challenges VALUES(zeroblob(32),zeroblob(33),zeroblob(33),zeroblob(16),123,1);",
        ).unwrap();
        let before = schema_objects(&connection);
        assert!(matches!(
            Database::open_existing(&path, Config::default()),
            Err(Error::SchemaVersion { found }) if found == version
        ));
        assert!(matches!(
            foks_server_db::ReadDatabase::open(&path, Config::default()),
            Err(Error::SchemaVersion { found }) if found == version
        ));
        assert_eq!(schema_objects(&connection), before);
        let upgraded = Database::open(&path, Config::default()).unwrap();
        assert_eq!(indexes(&connection), expected);
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT expires_at,consumed FROM recovery_challenges",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                )
                .unwrap(),
            (123, 1)
        );
        assert!(connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none());
        drop(upgraded);
        drop(Database::open(&path, Config::default()).unwrap());
        drop(Database::open_existing(&path, Config::default()).unwrap());
        drop(foks_server_db::ReadDatabase::open(&path, Config::default()).unwrap());
    }
}

#[test]
fn version_48_upgrade_expires_old_sso_envelopes_and_requires_relinking() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let path = test.path.clone();
    drop(test.database);
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection.execute_batch(
        "INSERT INTO sso_policy(host,config_hash,rollout_id,issuer,mode,revision,authorization_epoch)
         VALUES(zeroblob(33),zeroblob(32),zeroblob(16),'https://issuer.test',1,1,1);
         INSERT INTO sso_sessions(host,session_hash,config_hash,source_hash,uid,state,revision,
             authorization_epoch,expires_at_ms,ciphertext)
         VALUES(zeroblob(33),zeroblob(32),zeroblob(32),zeroblob(32),NULL,0,1,1,1000,zeroblob(57));
         INSERT INTO sso_access(host,uid,issuer,subject,config_hash,revision,authorization_epoch,
             authorization_generation,state,expires_at_ms,ciphertext)
         VALUES(zeroblob(33),X'010101010101010101010101010101010101010101010101010101010101010101',
             'https://issuer.test','subject',zeroblob(32),1,1,1,0,1000,zeroblob(57));
         INSERT INTO sso_identity_challenges(challenge,claim_hash,source_hash,expires_at_ms)
         VALUES(zeroblob(32),zeroblob(32),zeroblob(32),1000);
         INSERT INTO sso_binding_receipts(host,uid,commitment,purpose,authorization_epoch,
             authorization_generation,expires_at_ms)
         VALUES(zeroblob(33),X'010101010101010101010101010101010101010101010101010101010101010101',
             zeroblob(32),0,1,1,1000);",
    ).unwrap();
    connection.pragma_update(None, "user_version", 47).unwrap();

    drop(Database::open(&path, Config::default()).unwrap());
    for table in [
        "sso_sessions",
        "sso_access",
        "sso_identity_challenges",
        "sso_binding_receipts",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} survived the SSO format cutover");
    }
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM sso_policy", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn failed_index_upgrade_rolls_back_all_ddl_and_preserves_predecessor_version() {
    for version in [43, 44] {
        let mut test = common::TestDatabase::new();
        test.reserve(1_000_000);
        test.commit(None).unwrap();
        let path = test.path.clone();
        drop(test.database);
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        predecessor(&connection, version);
        connection.execute(
            "INSERT INTO rt_user_inboxes(uid,app_id,version,reconcile_memberships,reconcile_after)
             VALUES(?1,1,7,X'0102',zeroblob(16))", [[1_u8; 33]],
        ).unwrap();
        // Fail at the final index after earlier index builds and, for 43, the
        // reconciliation column/trigger changes have already run.
        connection.execute_batch(
            "CREATE INDEX log_sends_created_at ON log_sends(uid);
             INSERT INTO recovery_challenges VALUES(zeroblob(32),zeroblob(33),zeroblob(33),zeroblob(16),123,1);",
        ).unwrap();
        let before = schema_objects(&connection);
        assert!(Database::open(&path, Config::default()).is_err());
        assert_eq!(schema_objects(&connection), before);
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM recovery_challenges", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT version,reconcile_memberships,reconcile_after FROM rt_user_inboxes",
                    [],
                    |row| Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?
                    )),
                )
                .unwrap(),
            (7, vec![1, 2], vec![0; 16])
        );
        // Removing the deliberately conflicting index lets the same file retry.
        connection
            .execute_batch("DROP INDEX log_sends_created_at")
            .unwrap();
        drop(Database::open(&path, Config::default()).unwrap());
    }
}

#[test]
fn fresh_schema_failure_rolls_back_creation_and_version_stamps() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collision.sqlite");
    let connection = Connection::open(&path).unwrap();
    // A preexisting object collides near the end of the fresh schema batch.
    connection
        .execute_batch(
            "CREATE TABLE log_sends(sentinel TEXT); INSERT INTO log_sends VALUES('retained');",
        )
        .unwrap();
    let before = schema_objects(&connection);
    assert!(Database::open(&path, Config::default()).is_err());
    assert_eq!(schema_objects(&connection), before);
    for pragma in ["application_id", "user_version"] {
        assert_eq!(
            connection
                .pragma_query_value(None, pragma, |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        connection
            .query_row("SELECT sentinel FROM log_sends", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "retained"
    );
}

#[test]
fn every_validating_open_rejects_foreign_future_and_unsupported_old_files_without_ddl() {
    for (application_id, version) in [
        (0x1234_5678, 43),
        (APPLICATION_ID, 42),
        (APPLICATION_ID, SCHEMA_VERSION + 1),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("refused.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('retained');",
            )
            .unwrap();
        connection
            .pragma_update(None, "application_id", application_id)
            .unwrap();
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        let before = schema_objects(&connection);
        let results = [
            Database::open(&path, Config::default()).map(|_| ()),
            Database::open_existing(&path, Config::default()).map(|_| ()),
            foks_server_db::ReadDatabase::open(&path, Config::default()).map(|_| ()),
        ];
        for result in results {
            if application_id != APPLICATION_ID {
                assert!(
                    matches!(result, Err(Error::ApplicationId { found }) if found == application_id)
                );
            } else {
                assert!(matches!(result, Err(Error::SchemaVersion { found }) if found == version));
            }
        }
        assert_eq!(schema_objects(&connection), before);
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "application_id", |r| r.get::<_, i64>(0))
                .unwrap(),
            application_id
        );
    }
}
