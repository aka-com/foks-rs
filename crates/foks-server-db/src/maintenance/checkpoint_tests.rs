use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::Config;

#[test]
fn reports_validate_sentinels_and_classify_progress() {
    for (busy, log, done, expected) in [
        (0, 0, 0, CheckpointOutcome::Complete),
        (0, 9, 9, CheckpointOutcome::Complete),
        (0, 9, 4, CheckpointOutcome::Deferred),
        (1, 9, 9, CheckpointOutcome::Deferred),
        (0, -1, -1, CheckpointOutcome::Unavailable),
        (1, -1, -1, CheckpointOutcome::Deferred),
    ] {
        let report = CheckpointReport::decode(busy, log, done).unwrap();
        assert_eq!(report.outcome(), expected);
        assert_eq!(report.wal_pages, u64::try_from(log).ok());
        assert_eq!(report.checkpointed_pages, u64::try_from(done).ok());
    }
    for (busy, log, done) in [
        (-1, 0, 0),
        (2, 0, 0),
        (0, -1, 0),
        (0, 0, -1),
        (0, -2, -2),
        (0, 3, 4),
        (1, 2, -2),
    ] {
        assert!(CheckpointReport::decode(busy, log, done).is_err());
    }
}

#[test]
fn passive_makes_partial_progress_without_calling_busy_handler() {
    static BUSY_CALLS: AtomicUsize = AtomicUsize::new(0);
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("host.sqlite"), Config::default()).unwrap();
    database
        .connection
        .execute_batch(
            "CREATE TABLE checkpoint_fixture(value); INSERT INTO checkpoint_fixture VALUES(1)",
        )
        .unwrap();
    database.checkpoint().unwrap();
    database
        .connection
        .execute("INSERT INTO checkpoint_fixture VALUES(2)", [])
        .unwrap();
    let reader = database.open_reader().unwrap();
    reader
        .execute_batch("BEGIN; SELECT * FROM checkpoint_fixture;")
        .unwrap();
    database
        .connection
        .execute("INSERT INTO checkpoint_fixture VALUES(3)", [])
        .unwrap();
    database
        .connection
        .busy_handler(Some(|_| {
            BUSY_CALLS.fetch_add(1, Ordering::Relaxed);
            false
        }))
        .unwrap();

    let report = database.checkpoint_passive().unwrap();
    assert_eq!(report.busy, 0);
    assert_eq!(report.outcome(), CheckpointOutcome::Deferred);
    assert!(report.wal_pages.unwrap() > report.checkpointed_pages.unwrap());
    database
        .connection
        .execute("INSERT INTO checkpoint_fixture VALUES(4)", [])
        .unwrap();
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        database.checkpoint_passive().unwrap().outcome(),
        CheckpointOutcome::Complete
    );
    assert_eq!(BUSY_CALLS.load(Ordering::Relaxed), 0);
    assert!(database.integrity_check().unwrap());
}

#[test]
fn checkpoint_paths_preserve_live_connection_pragmas_on_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("host.sqlite");
    let config = Config {
        busy_timeout: Duration::from_millis(735),
        ..Config::default()
    };
    for existing in [false, true] {
        let database = if existing {
            Database::open_existing(&path, config.clone()).unwrap()
        } else {
            Database::open(&path, config.clone()).unwrap()
        };
        let before = database.pragmas().unwrap();
        assert_eq!(before.busy_timeout_millis, 735);
        assert_eq!(before.wal_autocheckpoint_pages, 1000);
        assert_eq!(before.wal_reuse_limit_bytes, 16 * 1024 * 1024);
        assert_eq!(before.journal_mode, "wal");
        assert_eq!(before.synchronous, 2);
        database.checkpoint_passive().unwrap();
        assert_eq!(database.pragmas().unwrap(), before);
        database.checkpoint().unwrap();
        assert_eq!(database.pragmas().unwrap(), before);
        database
            .connection
            .busy_timeout(Duration::from_millis(123))
            .unwrap();
        assert_eq!(database.pragmas().unwrap().busy_timeout_millis, 123);
    }
}

#[test]
fn reuse_limits_shrink_on_next_write_and_preserve_data_across_restart() {
    for limit in [0, 64 * 1024] {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("host.sqlite");
        let config = Config {
            wal_reuse_limit_bytes: limit,
            ..Config::default()
        };
        let database = Database::open(&path, config.clone()).unwrap();
        database
            .connection
            .execute_batch("CREATE TABLE checkpoint_fixture(data BLOB)")
            .unwrap();
        database.checkpoint().unwrap();
        database
            .connection
            .execute(
                "INSERT INTO checkpoint_fixture VALUES(zeroblob(524288))",
                [],
            )
            .unwrap();
        let reader = database.open_reader().unwrap();
        reader
            .execute_batch("BEGIN; SELECT * FROM checkpoint_fixture")
            .unwrap();
        database
            .connection
            .execute(
                "INSERT INTO checkpoint_fixture VALUES(zeroblob(524288))",
                [],
            )
            .unwrap();
        assert_eq!(
            database.checkpoint_passive().unwrap().outcome(),
            CheckpointOutcome::Deferred
        );
        let retained = database.storage_report().unwrap().wal_bytes;
        assert!(retained > 1024 * 1024);
        reader.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            database.checkpoint_passive().unwrap().outcome(),
            CheckpointOutcome::Complete
        );
        assert_eq!(database.storage_report().unwrap().wal_bytes, retained);
        database
            .connection
            .execute("INSERT INTO checkpoint_fixture VALUES(x'01')", [])
            .unwrap();
        let reused = database.storage_report().unwrap().wal_bytes;
        assert!(reused < retained);
        // The zero policy still needs storage for the new transaction's frames.
        assert!(reused <= limit.max(64 * 1024));
        assert!(database.integrity_check().unwrap());
        drop(reader);
        drop(database);

        let database = Database::open_existing(&path, config).unwrap();
        assert_eq!(database.pragmas().unwrap().wal_reuse_limit_bytes, limit);
        let bytes: i64 = database
            .connection
            .query_row(
                "SELECT sum(length(data)) FROM checkpoint_fixture",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bytes, 1024 * 1024 + 1);
        assert!(database.integrity_check().unwrap());
        let truncate = database.checkpoint().unwrap();
        assert_eq!(truncate.wal_pages, Some(0));
        assert_eq!(truncate.checkpointed_pages, Some(0));
        assert_eq!(database.storage_report().unwrap().wal_bytes, 0);
    }
}

#[test]
fn reuse_limit_must_fit_sqlite_integer() {
    let temporary = tempfile::tempdir().unwrap();
    let config = Config {
        wal_reuse_limit_bytes: u64::MAX,
        ..Config::default()
    };
    assert!(matches!(
        Database::open(temporary.path().join("host.sqlite"), config),
        Err(Error::IntegerRange)
    ));
}
