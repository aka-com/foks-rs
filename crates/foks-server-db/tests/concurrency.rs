mod common;

use foks_server_db::{root_snapshot, RootSnapshot};

#[test]
fn read_transaction_observes_one_complete_root_snapshot() {
    let mut database = common::TestDatabase::new();
    let reader = database.database.open_reader().unwrap();
    reader.execute_batch("BEGIN").unwrap();
    assert_eq!(root_snapshot(&reader).unwrap(), None);

    database.reserve(1_000_000);
    database.commit(None).unwrap();
    assert_eq!(root_snapshot(&reader).unwrap(), None);
    reader.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        root_snapshot(&reader).unwrap(),
        Some(RootSnapshot { epoch: 1, .. })
    ));
}
