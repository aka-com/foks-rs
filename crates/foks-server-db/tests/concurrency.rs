mod common;

use foks_server_db::{Config, ReadDatabase, RootSnapshot};

#[test]
fn request_snapshot_observes_one_complete_root_state() {
    let mut database = common::TestDatabase::new();
    let reader = ReadDatabase::open(&database.path, Config::default()).unwrap();
    let snapshot = reader.snapshot().unwrap();
    assert_eq!(snapshot.current_root().unwrap(), None);

    database.reserve(1_000_000);
    database.commit(None).unwrap();
    assert_eq!(snapshot.current_root().unwrap(), None);
    drop(snapshot);
    assert!(matches!(
        reader.current_root().unwrap(),
        Some(RootSnapshot { epoch: 1, .. })
    ));
}
