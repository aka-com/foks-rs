mod common;

use foks_proto::{KvDirectoryVersion, KvDirentVersion, KvPathVersionVector};
use foks_server_db::{
    Error, KvDirectoryMutation, KvDirentMutation, KvNodeMutation, KvRootMutation,
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
fn dirent_batch_is_atomic_and_updates_only_the_reachable_version_vector() {
    let (mut test, root) = initialized_database();
    let orphan = [0x52; 16];
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &orphan,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: b"orphan-directory",
        })
        .unwrap();
    let reader =
        foks_server_db::ReadDatabase::open(&test.path, foks_server_db::Config::default()).unwrap();
    let initial = KvPathVersionVector {
        root_version: 1,
        directories: vec![KvDirectoryVersion {
            id: root,
            version: 1,
            entries: Vec::new(),
        }],
    };
    assert_eq!(
        reader.kv_version_vector(&UID).unwrap(),
        Some(initial.clone())
    );

    let node = [3; 17];
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &node,
            node_type: 3,
            exact: b"small-file-box",
        })
        .unwrap();
    let entry = [0x61; 16];
    let name_mac = [0x71; 32];
    test.database
        .put_kv_dirents(
            &UID,
            &initial,
            &[KvDirentMutation {
                parent: &root,
                id: &entry,
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &name_mac,
                creation_time: 42,
                exact: b"dirent-one",
            }],
        )
        .unwrap();
    // The same request can be replayed after its own precondition advanced,
    // covering a committed mutation whose response was lost.
    test.database
        .put_kv_dirents(
            &UID,
            &initial,
            &[KvDirentMutation {
                parent: &root,
                id: &entry,
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &name_mac,
                creation_time: 42,
                exact: b"dirent-one",
            }],
        )
        .unwrap();

    let current = KvPathVersionVector {
        root_version: 1,
        directories: vec![KvDirectoryVersion {
            id: root,
            version: 1,
            entries: vec![KvDirentVersion {
                id: entry,
                version: 1,
            }],
        }],
    };
    assert_eq!(reader.kv_version_vector(&UID).unwrap(), Some(current));
    test.database
        .put_kv_directory_with_precondition(
            &KvDirectoryMutation {
                uid: &UID,
                id: &orphan,
                version: 1,
                key_role: 3,
                key_visibility: 0,
                key_generation: 1,
                status: 0,
                exact: b"orphan-directory",
            },
            Some(&initial),
        )
        .unwrap();
    assert_eq!(
        reader.kv_node(&UID, &node).unwrap().unwrap().exact,
        b"small-file-box"
    );
    assert_eq!(reader.kv_list(&UID, &root, None, 10).unwrap().len(), 1);

    let other_entry = [0x62; 16];
    assert!(matches!(
        test.database.put_kv_dirents(
            &UID,
            &initial,
            &[KvDirentMutation {
                parent: &root,
                id: &other_entry,
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &[0x72; 32],
                creation_time: 43,
                exact: b"dirent-two",
            }],
        ),
        Err(Error::KvConflict)
    ));
    assert_eq!(reader.kv_list(&UID, &root, None, 10).unwrap().len(), 1);
}

#[test]
fn tombstone_removes_an_entry_from_listing_and_cache_preconditions() {
    let (mut test, root) = initialized_database();
    let reader =
        foks_server_db::ReadDatabase::open(&test.path, foks_server_db::Config::default()).unwrap();
    let empty = reader.kv_version_vector(&UID).unwrap().unwrap();
    let node = [4; 17];
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &node,
            node_type: 4,
            exact: b"symlink-box",
        })
        .unwrap();
    let entry = [0x63; 16];
    let name_mac = [0x73; 32];
    test.database
        .put_kv_dirents(
            &UID,
            &empty,
            &[KvDirentMutation {
                parent: &root,
                id: &entry,
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &name_mac,
                creation_time: 44,
                exact: b"live",
            }],
        )
        .unwrap();
    let live = reader.kv_version_vector(&UID).unwrap().unwrap();
    test.database
        .put_kv_dirents(
            &UID,
            &live,
            &[KvDirentMutation {
                parent: &root,
                id: &entry,
                version: 2,
                directory_version: 1,
                node_id: &[0; 17],
                name_mac: &name_mac,
                creation_time: 44,
                exact: b"tombstone",
            }],
        )
        .unwrap();
    assert_eq!(reader.kv_version_vector(&UID).unwrap(), Some(empty));
    assert!(reader.kv_list(&UID, &root, None, 10).unwrap().is_empty());
}
