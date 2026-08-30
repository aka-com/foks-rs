mod common;

use foks_server_db::{
    Error, KvDirectoryMutation, KvFileChunkMutation, KvRootMutation, ReadDatabase,
};

const UID: [u8; 33] = [1; 33];

fn initialized_database() -> (common::TestDatabase, [u8; 16]) {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let root = [0x51; 16];
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &root,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: b"root-directory",
        })
        .unwrap();
    test.database
        .put_kv_root(&KvRootMutation {
            uid: &UID,
            version: 1,
            directory_id: &root,
            directory_version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            exact: b"root",
        })
        .unwrap();
    (test, root)
}

#[test]
fn incomplete_uploads_are_hidden_and_final_chunks_replay_exactly() {
    let (mut test, _) = initialized_database();
    let file = [0x61; 16];
    let first = KvFileChunkMutation {
        uid: &UID,
        file_id: &file,
        exact_metadata: Some(b"metadata"),
        offset: 0,
        ciphertext: b"abc",
        final_size: None,
        exact_chunk: b"first",
        now: 100,
    };
    test.database.put_kv_file_chunk(&first).unwrap();
    test.database.put_kv_file_chunk(&first).unwrap();
    let reader = ReadDatabase::open(&test.path, foks_server_db::Config::default()).unwrap();
    assert!(reader.kv_file(&UID, &file).unwrap().is_none());
    assert!(reader.kv_file_chunk(&UID, &file, 0).unwrap().is_none());

    let final_chunk = KvFileChunkMutation {
        uid: &UID,
        file_id: &file,
        exact_metadata: None,
        offset: 4,
        ciphertext: b"de",
        final_size: Some(5),
        exact_chunk: b"final",
        now: 200,
    };
    test.database.put_kv_file_chunk(&final_chunk).unwrap();
    test.database.put_kv_file_chunk(&final_chunk).unwrap();
    assert_eq!(
        reader.kv_file(&UID, &file).unwrap().unwrap().exact_metadata,
        b"metadata"
    );
    let stored = reader.kv_file_chunk(&UID, &file, 4).unwrap().unwrap();
    assert_eq!(stored.ciphertext, b"de");
    assert!(stored.final_chunk);
    assert!(matches!(
        test.database.put_kv_file_chunk(&KvFileChunkMutation {
            exact_chunk: b"different-final",
            ..final_chunk
        }),
        Err(Error::KvConflict)
    ));
}

#[test]
fn locks_are_token_bound_and_expiry_is_effective_without_cleanup() {
    let (mut test, root) = initialized_database();
    let dirent = [0x71; 16];
    let first = [0x81; 16];
    let second = [0x82; 16];
    test.database
        .acquire_kv_lock(&UID, &root, &dirent, &first, 100, 200)
        .unwrap();
    assert!(matches!(
        test.database
            .acquire_kv_lock(&UID, &root, &dirent, &first, 150, 250),
        Err(Error::KvLocked)
    ));
    assert!(matches!(
        test.database
            .acquire_kv_lock(&UID, &root, &dirent, &second, 150, 250),
        Err(Error::KvLocked)
    ));
    assert!(matches!(
        test.database.release_kv_lock(&UID, &root, &dirent, &second),
        Err(Error::KvLockTimeout)
    ));
    test.database
        .acquire_kv_lock(&UID, &root, &dirent, &second, 200, 100)
        .unwrap();
    test.database
        .release_kv_lock(&UID, &root, &dirent, &second)
        .unwrap();

    test.database
        .acquire_kv_lock(&UID, &root, &dirent, &first, 300, 200)
        .unwrap();
    test.database
        .acquire_kv_lock(&UID, &root, &dirent, &second, 300, 0)
        .unwrap();
    test.database
        .release_kv_lock(&UID, &root, &dirent, &second)
        .unwrap();
}
