mod common;

use foks_server_db::{CommitOutcome, Error};

#[test]
fn lost_response_replays_exact_bytes_without_repeating_mutation() {
    let mut database = common::TestDatabase::new();
    database.reserve(1_000_000);
    assert_eq!(
        database.commit(None).unwrap(),
        CommitOutcome::Committed(b"response".to_vec())
    );
    assert_eq!(
        database.commit(None).unwrap(),
        CommitOutcome::Replayed(b"response".to_vec())
    );
    assert!(matches!(
        common::commit_with_request(&mut database.database, None, [0x98; 32]),
        Err(Error::OperationConflict)
    ));
    assert!(matches!(
        common::commit_with_request_at(&mut database.database, None, [0x99; 32], 2_000_000,),
        Err(Error::OperationExpired)
    ));
}
