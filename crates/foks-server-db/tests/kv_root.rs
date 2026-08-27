mod common;

use foks_server_db::{KvDirectoryMutation, KvRootMutation};

#[test]
fn root_requires_an_owned_matching_directory_and_replays_exactly() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let directory = [0x41; 16];
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &[1; 33],
            id: &directory,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: b"directory",
        })
        .unwrap();
    let mutation = KvRootMutation {
        uid: &[1; 33],
        version: 1,
        directory_id: &directory,
        directory_version: 1,
        key_role: 3,
        key_visibility: 0,
        key_generation: 1,
        exact: b"root",
    };
    test.database.put_kv_root(&mutation).unwrap();
    test.database.put_kv_root(&mutation).unwrap();

    let reader =
        foks_server_db::ReadDatabase::open(&test.path, foks_server_db::Config::default()).unwrap();
    let root = reader.kv_root(&[1; 33]).unwrap().unwrap();
    assert_eq!(root.directory_id, directory);
    assert_eq!(root.exact, b"root");
    let stored_directory = reader.kv_directory(&[1; 33], &directory).unwrap().unwrap();
    assert_eq!(stored_directory.exact, b"directory");
}

#[test]
fn conflicting_root_or_directory_never_replaces_authoritative_bytes() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let directory = [0x42; 16];
    let first = KvDirectoryMutation {
        uid: &[1; 33],
        id: &directory,
        version: 1,
        key_role: 3,
        key_visibility: 0,
        key_generation: 1,
        status: 0,
        exact: b"directory-one",
    };
    test.database.put_kv_directory(&first).unwrap();
    assert!(test
        .database
        .put_kv_directory(&KvDirectoryMutation {
            exact: b"directory-two",
            ..first
        })
        .is_err());
    assert!(test
        .database
        .put_kv_root(&KvRootMutation {
            uid: &[1; 33],
            version: 1,
            directory_id: &directory,
            directory_version: 1,
            key_role: 2,
            key_visibility: 0,
            key_generation: 1,
            exact: b"wrong-key-root",
        })
        .is_err());
    assert!(
        foks_server_db::ReadDatabase::open(&test.path, foks_server_db::Config::default(),)
            .unwrap()
            .kv_root(&[1; 33])
            .unwrap()
            .is_none()
    );
}
