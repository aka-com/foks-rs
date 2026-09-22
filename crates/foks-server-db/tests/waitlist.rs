mod common;

use foks_server_db::Error;

#[test]
fn waitlist_entries_are_typed_validated_and_persisted() {
    let mut store = common::TestDatabase::new();
    let id = waitlist_id(1);
    store
        .database
        .join_waitlist(&id, "alice@example.com", 100)
        .unwrap();
    assert!(matches!(
        store
            .database
            .join_waitlist(&waitlist_id(2), "not-an-email", 101),
        Err(Error::Invalid("waitlist entry"))
    ));
    assert!(matches!(
        store.database.join_waitlist(&id, "alice@example.com", 102),
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

fn waitlist_id(fill: u8) -> [u8; 13] {
    let mut id = [fill; 13];
    id[0] = 1;
    id
}
