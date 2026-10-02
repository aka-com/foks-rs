mod common;

use foks_proto::{
    KvDirectoryStatus, KvDirectoryVersion, KvDirent, KvDirentVersion, KvNodeId,
    KvPathVersionVector, Role, SecretBox,
};
use foks_server_db::{
    Error, KvDirectoryMutation, KvDirentMutation, KvNodeMutation, KvRootMutation, KvVersionCheck,
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
fn small_node_ids_are_immutable_nonce_bindings() {
    let (mut test, _) = initialized_database();
    let node = [3; 17];
    let original = KvNodeMutation {
        uid: &UID,
        id: &node,
        node_type: 3,
        exact: b"first-ciphertext",
    };
    test.database.put_kv_node(&original).unwrap();
    // Idempotent transport retries remain valid.
    test.database.put_kv_node(&original).unwrap();
    // A distinct ciphertext may not reuse the object ID that supplies its
    // deterministic v0.1.9 Secretbox nonce.
    assert!(matches!(
        test.database.put_kv_node(&KvNodeMutation {
            exact: b"second-ciphertext",
            ..original
        }),
        Err(Error::KvConflict)
    ));
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
            Some(&initial),
            foks_proto::Role::OWNER,
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
            Some(&initial),
            foks_proto::Role::OWNER,
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
    assert_eq!(
        reader.kv_version_check(&UID, &initial).unwrap(),
        KvVersionCheck::Current
    );
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
    test.database
        .put_kv_dirents(
            &UID,
            Some(&initial),
            foks_proto::Role::OWNER,
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
        )
        .unwrap();
    let wildcard_entry = [0x64; 16];
    test.database
        .put_kv_dirents(
            &UID,
            None,
            foks_proto::Role::OWNER,
            &[KvDirentMutation {
                parent: &root,
                id: &wildcard_entry,
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &[0x74; 32],
                creation_time: 43,
                exact: b"dirent-wildcard",
            }],
        )
        .unwrap();
    assert_eq!(reader.kv_list(&UID, &root, None, 10).unwrap().len(), 3);
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
            Some(&empty),
            foks_proto::Role::OWNER,
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
            Some(&live),
            foks_proto::Role::OWNER,
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
    assert_eq!(
        reader.kv_version_check(&UID, &live).unwrap(),
        KvVersionCheck::Stale(KvPathVersionVector {
            root_version: 1,
            directories: vec![KvDirectoryVersion {
                id: root,
                version: 1,
                entries: vec![KvDirentVersion {
                    id: entry,
                    version: 2,
                }],
            }],
        })
    );
    assert_eq!(
        reader
            .kv_dirent_at_name(&UID, &root, 1, &name_mac)
            .unwrap()
            .unwrap()
            .exact,
        b"tombstone"
    );
}

#[test]
fn replacing_a_dirent_requires_its_stored_write_role() {
    let (mut test, root) = initialized_database();
    let node = [3; 17];
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &node,
            node_type: 3,
            exact: b"small-file-box",
        })
        .unwrap();
    let id = [0x65; 16];
    let name_mac = [0x75; 32];
    let name_box = SecretBox {
        nonce: [0x85; 16],
        ciphertext: vec![0x95; 16],
    };
    let initial = KvDirent::new(
        root,
        id,
        KvNodeId(node),
        1,
        1,
        Role::ADMIN,
        name_mac,
        name_box.clone(),
        KvDirectoryStatus::Active,
        [0xa5; 32],
        1,
    )
    .unwrap();
    test.database
        .put_kv_dirents(
            &UID,
            None,
            Role::OWNER,
            &[KvDirentMutation {
                parent: &root,
                id: &id,
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &name_mac,
                creation_time: 1,
                exact: initial.encoded(),
            }],
        )
        .unwrap();
    let replacement = KvDirent::new(
        root,
        id,
        KvNodeId(node),
        2,
        1,
        Role::member(0),
        name_mac,
        name_box,
        KvDirectoryStatus::Active,
        [0xb5; 32],
        1,
    )
    .unwrap();
    assert!(matches!(
        test.database.put_kv_dirents(
            &UID,
            None,
            Role::member(0),
            &[KvDirentMutation {
                parent: &root,
                id: &id,
                version: 2,
                directory_version: 1,
                node_id: &node,
                name_mac: &name_mac,
                creation_time: 1,
                exact: replacement.encoded(),
            }],
        ),
        Err(foks_server_db::Error::KvPermission)
    ));
}

fn empty_folder_fixture() -> (common::TestDatabase, foks_proto::KvEmptyDirectoryAssertion) {
    let (mut test, root) = initialized_database();
    let directory = [0x62; 16];
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &directory,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: b"empty-directory",
        })
        .unwrap();
    let assertion = foks_proto::KvEmptyDirectoryAssertion {
        parent: root,
        dirent: [0x63; 16],
        version: 1,
        directory,
    };
    let mut node = [1; 17];
    node[1..].copy_from_slice(&directory);
    test.database
        .put_kv_dirents(
            &UID,
            None,
            Role::OWNER,
            &[folder_mutation(&assertion, &node, 1, b"folder-source")],
        )
        .unwrap();
    (test, assertion)
}

fn folder_mutation<'a>(
    assertion: &'a foks_proto::KvEmptyDirectoryAssertion,
    node: &'a [u8; 17],
    version: u64,
    exact: &'a [u8],
) -> KvDirentMutation<'a> {
    KvDirentMutation {
        parent: &assertion.parent,
        id: &assertion.dirent,
        version,
        directory_version: 1,
        node_id: node,
        name_mac: &[0x64; 32],
        creation_time: 1,
        exact,
    }
}

#[test]
fn empty_directory_guard_catches_phantom_child_and_rejects_batch_atomically() {
    let (mut test, assertion) = empty_folder_fixture();
    let reader = foks_server_db::ReadDatabase::open(&test.path, Default::default()).unwrap();
    let apparent_empty = reader.kv_version_vector(&UID).unwrap().unwrap();
    assert!(apparent_empty
        .directories
        .iter()
        .find(|d| d.id == assertion.directory)
        .unwrap()
        .entries
        .is_empty());
    let node = [3; 17];
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &node,
            node_type: 3,
            exact: b"concurrent-child-content",
        })
        .unwrap();
    let child = KvDirentMutation {
        parent: &assertion.directory,
        id: &[0x65; 16],
        version: 1,
        directory_version: 1,
        node_id: &node,
        name_mac: &[0x66; 32],
        creation_time: 2,
        exact: b"concurrent-child",
    };
    test.database
        .put_kv_dirents(&UID, None, Role::OWNER, &[child])
        .unwrap();
    // The supplied vector still passes: new entries are not an existing-version change.
    assert_eq!(
        reader.kv_version_check(&UID, &apparent_empty).unwrap(),
        KvVersionCheck::Current
    );
    let sibling = KvDirentMutation {
        parent: &assertion.parent,
        id: &[0x67; 16],
        version: 1,
        directory_version: 1,
        node_id: &node,
        name_mac: &[0x68; 32],
        creation_time: 2,
        exact: b"must-not-be-inserted",
    };
    let removal = folder_mutation(&assertion, &[0; 17], 2, b"tombstone");
    assert!(matches!(
        test.database.put_kv_dirents_with_empty_directories(
            &UID,
            Some(&apparent_empty),
            Role::OWNER,
            &[sibling, removal],
            &[assertion],
        ),
        Err(Error::KvConflict)
    ));
    assert!(test
        .database
        .kv_dirent(&UID, &assertion.parent, &[0x67; 16])
        .unwrap()
        .is_none());
    assert_eq!(
        test.database
            .kv_dirent(&UID, &assertion.parent, &assertion.dirent)
            .unwrap()
            .unwrap()
            .exact,
        b"folder-source"
    );
    assert!(reader
        .kv_version_vector(&UID)
        .unwrap()
        .unwrap()
        .directories
        .iter()
        .any(|d| d.id == assertion.directory && d.entries.len() == 1));
    // Explicit legacy recursive unlink retains its old semantics.
    test.database
        .put_kv_dirents(
            &UID,
            Some(&apparent_empty),
            Role::OWNER,
            &[folder_mutation(
                &assertion,
                &[0; 17],
                2,
                b"legacy-tombstone",
            )],
        )
        .unwrap();
}

#[test]
fn empty_directory_guard_binds_source_version_and_requires_tombstone() {
    let (mut test, assertion) = empty_folder_fixture();
    let wrong_directory = foks_proto::KvEmptyDirectoryAssertion {
        directory: [0x70; 16],
        ..assertion
    };
    let removal = folder_mutation(&assertion, &[0; 17], 2, b"tombstone");
    assert!(matches!(
        test.database.put_kv_dirents_with_empty_directories(
            &UID,
            None,
            Role::OWNER,
            &[removal],
            &[wrong_directory],
        ),
        Err(Error::KvConflict)
    ));
    let wrong_version = foks_proto::KvEmptyDirectoryAssertion {
        version: 2,
        ..assertion
    };
    assert!(matches!(
        test.database.put_kv_dirents_with_empty_directories(
            &UID,
            None,
            Role::OWNER,
            &[folder_mutation(&assertion, &[0; 17], 3, b"tombstone")],
            &[wrong_version],
        ),
        Err(Error::KvConflict)
    ));
    assert!(matches!(
        test.database.put_kv_dirents_with_empty_directories(
            &UID,
            None,
            Role::OWNER,
            &[folder_mutation(&assertion, &[3; 17], 2, b"replacement")],
            &[assertion],
        ),
        Err(Error::Invalid(_))
    ));
    let mut directory_node = [1; 17];
    directory_node[1..].copy_from_slice(&assertion.directory);
    test.database
        .put_kv_dirents(
            &UID,
            None,
            Role::OWNER,
            &[folder_mutation(
                &assertion,
                &directory_node,
                2,
                b"changed-folder-source",
            )],
        )
        .unwrap();
    assert!(matches!(
        test.database.put_kv_dirents_with_empty_directories(
            &UID,
            None,
            Role::OWNER,
            &[folder_mutation(&assertion, &[0; 17], 2, b"tombstone")],
            &[assertion],
        ),
        Err(Error::KvConflict)
    ));
}

#[test]
fn empty_directory_guard_succeeds_and_exact_replay_ignores_later_children() {
    let (mut test, assertion) = empty_folder_fixture();
    let reader = foks_server_db::ReadDatabase::open(&test.path, Default::default()).unwrap();
    let original_vector = reader.kv_version_vector(&UID).unwrap().unwrap();
    test.database
        .put_kv_dirents_with_empty_directories(
            &UID,
            Some(&original_vector),
            Role::OWNER,
            &[folder_mutation(&assertion, &[0; 17], 2, b"tombstone")],
            &[assertion],
        )
        .unwrap();
    let node = [3; 17];
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &node,
            node_type: 3,
            exact: b"late-child-content",
        })
        .unwrap();
    // An ordinary writer with a stale directory ID can still target the retained
    // directory object. This must not turn a completed deletion's retry into failure.
    test.database
        .put_kv_dirents(
            &UID,
            None,
            Role::OWNER,
            &[KvDirentMutation {
                parent: &assertion.directory,
                id: &[0x69; 16],
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &[0x69; 32],
                creation_time: 2,
                exact: b"late-child",
            }],
        )
        .unwrap();
    test.database
        .put_kv_dirents_with_empty_directories(
            &UID,
            Some(&original_vector),
            Role::OWNER,
            &[folder_mutation(&assertion, &[0; 17], 2, b"tombstone")],
            &[assertion],
        )
        .unwrap();
    assert_eq!(
        test.database
            .kv_dirent(&UID, &assertion.parent, &assertion.dirent)
            .unwrap()
            .unwrap()
            .exact,
        b"tombstone"
    );
}

#[test]
fn empty_directory_guard_rejects_child_insert_in_same_batch() {
    let (mut test, assertion) = empty_folder_fixture();
    let node = [3; 17];
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &node,
            node_type: 3,
            exact: b"child-content",
        })
        .unwrap();
    let child = KvDirentMutation {
        parent: &assertion.directory,
        id: &[0x71; 16],
        version: 1,
        directory_version: 1,
        node_id: &node,
        name_mac: &[0x72; 32],
        creation_time: 1,
        exact: b"child",
    };
    assert!(matches!(
        test.database.put_kv_dirents_with_empty_directories(
            &UID,
            None,
            Role::OWNER,
            &[
                child,
                folder_mutation(&assertion, &[0; 17], 2, b"tombstone")
            ],
            &[assertion],
        ),
        Err(Error::Invalid(_))
    ));
}
