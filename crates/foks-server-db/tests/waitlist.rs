mod common;

use std::time::Duration;

use foks_server_db::{Config, Error, ReadDatabase, WaitlistPurge};

#[test]
fn waitlist_entries_are_typed_validated_and_persisted() {
    let mut store = common::TestDatabase::new();
    let id = waitlist_id(1);
    assert_eq!(
        store
            .database
            .join_waitlist(&id, "alice@example.com", 100)
            .unwrap(),
        id
    );
    assert!(matches!(
        store
            .database
            .join_waitlist(&waitlist_id(2), "not-an-email", 101),
        Err(Error::Invalid("waitlist entry"))
    ));
    assert!(matches!(
        store.database.join_waitlist(&id, "bob@example.com", 102),
        Err(Error::Duplicate("waitlist entry"))
    ));

    let connection = rusqlite::Connection::open(&store.path).unwrap();
    let stored: (Vec<u8>, String, i64) = connection
        .query_row(
            "SELECT waitlist_id, email, created_at FROM waitlist_entries",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored, (id.to_vec(), "alice@example.com".to_owned(), 100));
}

#[test]
fn repeated_address_returns_the_stored_identifier_without_a_new_row() {
    let mut store = common::TestDatabase::new();
    let first = waitlist_id(1);
    store
        .database
        .join_waitlist(&first, "alice@example.com", 100)
        .unwrap();
    for (fill, email) in [(2, "alice@example.com"), (3, "ALICE@Example.COM")] {
        assert_eq!(
            store
                .database
                .join_waitlist(&waitlist_id(fill), email, 200)
                .unwrap(),
            first
        );
    }
    let entries = store.database.waitlist_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].waitlist_id, first);
    assert_eq!(entries[0].email, "alice@example.com");
    assert_eq!(entries[0].created_at, 100);
}

#[test]
fn new_addresses_are_refused_at_the_configured_entry_cap() {
    let mut store = common::TestDatabase::with_config(Config {
        maximum_waitlist_entries: 2,
        ..Config::default()
    });
    for (fill, email) in [(1, "a@example.com"), (2, "b@example.com")] {
        store
            .database
            .join_waitlist(&waitlist_id(fill), email, 100)
            .unwrap();
    }
    // Listed and new addresses get the same answer at the cap, so a full
    // table does not reveal which addresses it holds.
    for (fill, email) in [(3, "c@example.com"), (4, "a@example.com")] {
        assert!(matches!(
            store.database.join_waitlist(&waitlist_id(fill), email, 101),
            Err(Error::Capacity("waitlist"))
        ));
    }
    assert_eq!(store.database.waitlist_entries().unwrap().len(), 2);

    assert_eq!(
        store
            .database
            .purge_waitlist(WaitlistPurge::Email("B@EXAMPLE.COM"))
            .unwrap(),
        1
    );
    store
        .database
        .join_waitlist(&waitlist_id(3), "c@example.com", 102)
        .unwrap();
    assert_eq!(store.database.waitlist_entries().unwrap().len(), 2);
}

#[test]
fn operator_purge_selects_by_creation_time_address_or_all() {
    let mut store = common::TestDatabase::new();
    for (fill, email, created_at) in [
        (1, "a@example.com", 100),
        (2, "b@example.com", 200),
        (3, "c@example.com", 300),
        (4, "d@example.com", 400),
    ] {
        store
            .database
            .join_waitlist(&waitlist_id(fill), email, created_at)
            .unwrap();
    }
    assert_eq!(
        store
            .database
            .purge_waitlist(WaitlistPurge::CreatedBefore(200))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .database
            .purge_waitlist(WaitlistPurge::Email("missing@example.com"))
            .unwrap(),
        0
    );
    let reader = ReadDatabase::open(&store.path, Config::default()).unwrap();
    assert_eq!(
        reader
            .waitlist_entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.email)
            .collect::<Vec<_>>(),
        ["b@example.com", "c@example.com", "d@example.com"]
    );
    assert_eq!(
        store.database.purge_waitlist(WaitlistPurge::All).unwrap(),
        3
    );
    assert!(reader.waitlist_entries().unwrap().is_empty());
}

#[test]
fn maintenance_deletes_entries_older_than_the_retention_period() {
    let mut store = common::TestDatabase::with_config(Config {
        waitlist_retention: Duration::from_secs(10),
        ..Config::default()
    });
    store
        .database
        .join_waitlist(&waitlist_id(1), "old@example.com", 1_000_000)
        .unwrap();
    store
        .database
        .join_waitlist(&waitlist_id(2), "new@example.com", 2_000_000)
        .unwrap();
    // A clock earlier than one retention period expires nothing, even an
    // entry created at zero.
    store
        .database
        .join_waitlist(&waitlist_id(3), "zero@example.com", 0)
        .unwrap();
    let report = store.database.run_maintenance(9_999_999, 0).unwrap();
    assert_eq!(report.waitlist_entries, 0);
    store
        .database
        .purge_waitlist(WaitlistPurge::Email("zero@example.com"))
        .unwrap();
    let report = store.database.run_maintenance(10_999_999, 0).unwrap();
    assert_eq!(report.waitlist_entries, 0);
    let report = store.database.run_maintenance(11_000_000, 0).unwrap();
    assert_eq!(report.waitlist_entries, 1);
    let entries = store.database.waitlist_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].email, "new@example.com");
}

#[test]
fn stored_entry_count_follows_joins_purges_and_maintenance() {
    let mut store = common::TestDatabase::with_config(Config {
        maximum_waitlist_entries: 3,
        waitlist_retention: Duration::from_secs(10),
        ..Config::default()
    });
    assert_eq!(stored_count(&store.path), 0);
    for (fill, email, created_at) in [
        (1, "a@example.com", 1_000_000),
        (2, "b@example.com", 2_000_000),
        (3, "A@EXAMPLE.COM", 2_000_000),
        (4, "c@example.com", 3_000_000),
    ] {
        store
            .database
            .join_waitlist(&waitlist_id(fill), email, created_at)
            .unwrap();
    }
    assert_eq!(stored_count(&store.path), 3);
    // Refused and rejected joins leave the count unchanged.
    assert!(store
        .database
        .join_waitlist(&waitlist_id(5), "d@example.com", 3_000_000)
        .is_err());
    assert!(store
        .database
        .join_waitlist(&waitlist_id(6), "invalid", 3_000_000)
        .is_err());
    assert_eq!(stored_count(&store.path), 3);
    assert_eq!(
        store
            .database
            .run_maintenance(11_000_000, 0)
            .unwrap()
            .waitlist_entries,
        1
    );
    assert_eq!(stored_count(&store.path), 2);
    store
        .database
        .purge_waitlist(WaitlistPurge::Email("B@example.com"))
        .unwrap();
    assert_eq!(stored_count(&store.path), 1);
    store.database.purge_waitlist(WaitlistPurge::All).unwrap();
    assert_eq!(stored_count(&store.path), 0);

    // The cap check reads the stored count, not the table.
    let connection = rusqlite::Connection::open(&store.path).unwrap();
    connection
        .execute("UPDATE waitlist_count SET entries = 3", [])
        .unwrap();
    assert!(matches!(
        store
            .database
            .join_waitlist(&waitlist_id(7), "e@example.com", 4_000_000),
        Err(Error::Capacity("waitlist"))
    ));
}

#[test]
fn zero_waitlist_limits_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    for config in [
        Config {
            maximum_waitlist_entries: 0,
            ..Config::default()
        },
        Config {
            waitlist_retention: Duration::ZERO,
            ..Config::default()
        },
    ] {
        assert!(matches!(
            foks_server_db::Database::open(directory.path().join("zero.sqlite"), config),
            Err(Error::Invalid("zero database capacity limit"))
        ));
    }
}

/// The trigger-maintained count, checked against the table it describes.
fn stored_count(path: &std::path::Path) -> i64 {
    let connection = rusqlite::Connection::open(path).unwrap();
    let (stored, actual): (i64, i64) = connection
        .query_row(
            "SELECT (SELECT entries FROM waitlist_count),
                    (SELECT count(*) FROM waitlist_entries)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, actual);
    stored
}

fn waitlist_id(fill: u8) -> [u8; 13] {
    let mut id = [fill; 13];
    id[0] = 1;
    id
}
