mod common;

use foks_server_db::{Config, Error, ReadDatabase};

#[test]
fn read_only_snapshots_preserve_requested_order_and_duplicates() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();

    let reader = ReadDatabase::open(&fixture.path, Config::default()).unwrap();
    assert_eq!(reader.current_root().unwrap().unwrap().epoch, 1);
    let roots = reader.roots_at(&[1, 1]).unwrap().unwrap();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], roots[1]);
    assert_eq!(reader.roots_at(&[0]).unwrap(), None);
}

#[test]
fn read_only_open_rejects_an_unowned_sqlite_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("other.sqlite");
    rusqlite::Connection::open(&path).unwrap();
    assert!(matches!(
        ReadDatabase::open(&path, Config::default()),
        Err(Error::ApplicationId { found: 0 })
    ));
}
