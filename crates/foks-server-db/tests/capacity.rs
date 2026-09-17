mod common;

use foks_proto::{KvDirectoryVersion, KvPathVersionVector};
use foks_server_db::{
    Error, KvDirectoryMutation, KvDirentMutation, KvNodeMutation, KvRootMutation,
};

const UID: [u8; 33] = [1; 33];

fn initialized(config: foks_server_db::Config) -> (common::TestDatabase, [u8; 16]) {
    let mut test = common::TestDatabase::with_config(config);
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
fn namespace_byte_limit_accepts_minus_one_and_exact_but_rejects_plus_one() {
    // The initialized root consumes 18 exact bytes. Each case gets a fresh
    // database so the candidate mutation itself lands at limit - 1, limit,
    // or limit + 1.
    for (node_bytes, succeeds) in [(9, true), (10, true), (11, false)] {
        let config = foks_server_db::Config {
            maximum_kv_namespace_bytes: 28,
            ..foks_server_db::Config::default()
        };
        let (mut test, _) = initialized(config);
        let result = test.database.put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[3; 17],
            node_type: 3,
            exact: &vec![0x41; node_bytes],
        });
        assert_eq!(result.is_ok(), succeeds, "node bytes: {node_bytes}");
        if !succeeds {
            assert!(matches!(result, Err(Error::QuotaExceeded)));
        }
    }
}

#[test]
fn namespace_object_and_node_limits_are_exact_and_atomic() {
    let config = foks_server_db::Config {
        maximum_kv_namespace_objects: 3,
        maximum_kv_node_bytes: 4,
        ..foks_server_db::Config::default()
    };
    let (mut test, _) = initialized(config);
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[3; 17],
            node_type: 3,
            exact: &[0x41; 4],
        })
        .unwrap();
    assert!(matches!(
        test.database.put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[4; 17],
            node_type: 4,
            exact: &[0x42; 4],
        }),
        Err(Error::QuotaExceeded)
    ));
    assert!(test.database.integrity_check().unwrap());

    let config = foks_server_db::Config {
        maximum_kv_node_bytes: 4,
        ..foks_server_db::Config::default()
    };
    let (mut test, _) = initialized(config);
    assert!(matches!(
        test.database.put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[3; 17],
            node_type: 3,
            exact: &[0x43; 5],
        }),
        Err(Error::Invalid("KV node mutation"))
    ));
    assert!(test.database.integrity_check().unwrap());
}

#[test]
fn namespace_shape_limits_match_client_traversal_limits() {
    let config = foks_server_db::Config {
        maximum_kv_directories: 1,
        ..foks_server_db::Config::default()
    };
    let (mut test, _) = initialized(config);
    assert!(matches!(
        test.database.put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &[0x52; 16],
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: b"second-directory",
        }),
        Err(Error::QuotaExceeded)
    ));

    let config = foks_server_db::Config {
        maximum_kv_dirents: 1,
        ..foks_server_db::Config::default()
    };
    let reader_config = config.clone();
    let (mut test, root) = initialized(config);
    let reader = foks_server_db::ReadDatabase::open(&test.path, reader_config).unwrap();
    let initial = reader.kv_version_vector(&UID).unwrap().unwrap();
    let node = [3; 17];
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &node,
            node_type: 3,
            exact: b"small-file-box",
        })
        .unwrap();
    test.database
        .put_kv_dirents(
            &UID,
            Some(&initial),
            foks_proto::Role::OWNER,
            &[KvDirentMutation {
                parent: &root,
                id: &[0x61; 16],
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &[0x71; 32],
                creation_time: 42,
                exact: b"first",
            }],
        )
        .unwrap();
    let current = reader.kv_version_vector(&UID).unwrap().unwrap();
    assert!(matches!(
        test.database.put_kv_dirents(
            &UID,
            Some(&current),
            foks_proto::Role::OWNER,
            &[KvDirentMutation {
                parent: &root,
                id: &[0x62; 16],
                version: 1,
                directory_version: 1,
                node_id: &node,
                name_mac: &[0x72; 32],
                creation_time: 43,
                exact: b"second",
            }],
        ),
        Err(Error::QuotaExceeded)
    ));
}

#[test]
fn oversized_dirents_and_unsafe_capacity_configuration_are_rejected() {
    let config = foks_server_db::Config {
        maximum_kv_dirent_bytes: 4,
        ..foks_server_db::Config::default()
    };
    let (mut test, root) = initialized(config);
    let precondition = KvPathVersionVector {
        root_version: 1,
        directories: vec![KvDirectoryVersion {
            id: root,
            version: 1,
            entries: Vec::new(),
        }],
    };
    assert!(matches!(
        test.database.put_kv_dirents(
            &UID,
            Some(&precondition),
            foks_proto::Role::OWNER,
            &[KvDirentMutation {
                parent: &root,
                id: &[0x61; 16],
                version: 1,
                directory_version: 1,
                node_id: &[0; 17],
                name_mac: &[0x71; 32],
                creation_time: 42,
                exact: b"12345",
            }],
        ),
        Err(Error::Invalid("KV dirent mutation"))
    ));

    let path = test.path.with_file_name("unsafe-kv-capacity.sqlite");
    let config = foks_server_db::Config {
        maximum_kv_directories: foks_proto::MAXIMUM_KV_DIRECTORIES as u64 + 1,
        ..foks_server_db::Config::default()
    };
    assert!(matches!(
        foks_server_db::Database::open(path, config),
        Err(Error::Invalid(
            "KV capacity exceeds the client synchronization limits"
        ))
    ));
}

#[test]
fn reopening_rejects_preexisting_state_above_sync_limits() {
    let (mut test, _) = initialized(foks_server_db::Config::default());
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &[0x52; 16],
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: b"second-directory",
        })
        .unwrap();
    drop(test.database);
    let config = foks_server_db::Config {
        maximum_kv_directories: 1,
        ..foks_server_db::Config::default()
    };
    assert!(matches!(
        foks_server_db::Database::open_existing(&test.path, config),
        Err(Error::QuotaExceeded)
    ));
}

#[test]
fn total_database_limit_is_enforced_at_the_existing_page_boundary() {
    let test = common::TestDatabase::new();
    let path = test.path.clone();
    drop(test.database);
    let bytes = std::fs::metadata(&path).unwrap().len();
    assert!(bytes > 1);

    let below = foks_server_db::Config {
        maximum_database_bytes: bytes - 1,
        ..foks_server_db::Config::default()
    };
    assert!(matches!(
        foks_server_db::Database::open(&path, below),
        Err(Error::QuotaExceeded)
    ));
    for maximum_database_bytes in [bytes, bytes + 1] {
        foks_server_db::Database::open(
            &path,
            foks_server_db::Config {
                maximum_database_bytes,
                ..foks_server_db::Config::default()
            },
        )
        .unwrap();
    }
}
