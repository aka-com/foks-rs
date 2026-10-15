mod common;

use foks_proto::{KvDirectoryVersion, KvPathVersionVector};
use foks_server_db::{
    CapacityResource, Error, KvDirectoryMutation, KvDirentMutation, KvNamespaceUsage,
    KvNodeMutation, KvRootMutation,
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
fn pooled_reader_open_skips_the_kv_tree_capacity_check() {
    let (mut test, _) = initialized(foks_server_db::Config::default());
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[3; 17],
            node_type: 3,
            exact: &[0x41; 5],
        })
        .unwrap();
    let tighter = foks_server_db::Config {
        maximum_kv_node_bytes: 4,
        ..foks_server_db::Config::default()
    };
    assert!(matches!(
        foks_server_db::ReadDatabase::open(&test.path, tighter.clone()),
        Err(Error::QuotaExceeded)
    ));
    let reader = foks_server_db::ReadDatabase::open_pooled(&test.path, tighter).unwrap();
    assert!(reader.kv_version_vector(&UID).unwrap().is_some());

    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("other.sqlite");
    rusqlite::Connection::open(&other).unwrap();
    assert!(matches!(
        foks_server_db::ReadDatabase::open_pooled(&other, foks_server_db::Config::default()),
        Err(Error::ApplicationId { found: 0 })
    ));
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

    let path = test.path.with_file_name("unsafe-team-capacity.sqlite");
    let config = foks_server_db::Config {
        maximum_team_members: foks_server_db::RealtimeLimits::FANOUT_MEMBERS + 1,
        ..foks_server_db::Config::default()
    };
    assert!(matches!(
        foks_server_db::Database::open(&path, config),
        Err(Error::Invalid(
            "team member limit exceeds the realtime fanout limit"
        ))
    ));
    let config = foks_server_db::Config {
        maximum_team_members: foks_server_db::RealtimeLimits::FANOUT_MEMBERS,
        ..foks_server_db::Config::default()
    };
    assert!(foks_server_db::Database::open(&path, config).is_ok());
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

#[test]
fn capacity_report_counts_usage_as_the_limits_count_it() {
    // The root and a 10-byte node fill the 28-byte namespace exactly, so the
    // report must show no remaining bytes where the quota check refuses the
    // next byte.
    let config = foks_server_db::Config {
        maximum_kv_namespace_bytes: 28,
        ..foks_server_db::Config::default()
    };
    let (mut test, _) = initialized(config.clone());
    test.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[3; 17],
            node_type: 3,
            exact: &[0x41; 10],
        })
        .unwrap();
    assert!(matches!(
        test.database.put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[4; 17],
            node_type: 4,
            exact: &[0x41; 1],
        }),
        Err(Error::QuotaExceeded)
    ));
    test.database
        .reserve_team_name(b"fixtureteam", &[0x45; 17], 1, 2_000_000, 3_000_000)
        .unwrap();
    let mut waitlist_id = [7; 13];
    waitlist_id[0] = 1;
    test.database
        .join_waitlist(&waitlist_id, "a@example.com", 5)
        .unwrap();

    let reader = foks_server_db::ReadDatabase::open_pooled(&test.path, config.clone()).unwrap();
    let snapshot = reader.snapshot().unwrap();
    let report = snapshot.capacity_report(2_500_000).unwrap();
    assert_eq!(
        report
            .usage
            .iter()
            .map(|usage| usage.resource)
            .collect::<Vec<_>>(),
        CapacityResource::ALL
    );
    let usage = |resource| report.usage(resource).unwrap();
    assert_eq!(usage(CapacityResource::MerkleEpoch).used, 1);
    assert_eq!(usage(CapacityResource::Teams).used, 0);
    assert_eq!(usage(CapacityResource::Teams).limit, 4096);
    assert_eq!(usage(CapacityResource::TeamNameReservations).used, 1);
    assert_eq!(usage(CapacityResource::WaitlistEntries).used, 1);
    assert_eq!(usage(CapacityResource::WaitlistEntries).limit, 10_000);
    assert_eq!(
        usage(CapacityResource::InvitationCertificates).limit,
        65_536
    );
    assert_eq!(usage(CapacityResource::LocalJoinRequests).limit, 1_000_000);
    let kv_bytes = usage(CapacityResource::KvNamespaceBytes);
    assert_eq!(
        (kv_bytes.used, kv_bytes.limit, kv_bytes.remaining()),
        (28, 28, 0)
    );
    assert_eq!(usage(CapacityResource::KvNamespaceObjects).used, 3);
    assert_eq!(
        report.kv_namespaces,
        [KvNamespaceUsage {
            namespace_id: UID.to_vec(),
            bytes: 28,
            objects: 3,
        }]
    );
    assert!(report.realtime_teams.is_empty());

    let page_size: u64 = reader_pragma(&test.path, "page_size");
    let database = usage(CapacityResource::DatabaseBytes);
    assert_eq!(database.limit, config.maximum_database_bytes);
    assert!(database.used > 0 && database.used % page_size == 0);
    assert!(report.database_free_bytes <= database.used);
    drop(snapshot);

    // An expired reservation no longer counts; the reservation path deletes
    // it before counting.
    let expired = reader
        .snapshot()
        .unwrap()
        .capacity_report(3_000_000)
        .unwrap();
    assert_eq!(
        expired
            .usage(CapacityResource::TeamNameReservations)
            .unwrap()
            .used,
        0
    );
}

#[test]
fn capacity_report_lists_the_largest_namespaces_first() {
    let (test, _) = initialized(foks_server_db::Config::default());
    // Twelve more namespaces under the fixture host, each holding one node.
    // The report reads only stored rows, so they need no owning identity.
    let fixture = rusqlite::Connection::open(&test.path).unwrap();
    for tag in 2..=13_u8 {
        let mut uid = [tag; 33];
        uid[0] = 1;
        fixture
            .execute(
                "INSERT INTO kv_namespaces(namespace_id, party_kind, host_id)
                 SELECT ?1, party_kind, host_id FROM kv_namespaces LIMIT 1",
                [uid.as_slice()],
            )
            .unwrap();
        fixture
            .execute(
                "INSERT INTO kv_nodes(uid, node_id, node_type, exact_node)
                 VALUES (?1, ?2, 3, ?3)",
                rusqlite::params![
                    uid.as_slice(),
                    [3_u8; 17].as_slice(),
                    vec![0x41_u8; usize::from(tag) * 4]
                ],
            )
            .unwrap();
    }
    drop(fixture);
    let reader =
        foks_server_db::ReadDatabase::open_pooled(&test.path, foks_server_db::Config::default())
            .unwrap();
    let report = reader.snapshot().unwrap().capacity_report(0).unwrap();
    assert_eq!(
        report
            .kv_namespaces
            .iter()
            .map(|namespace| namespace.bytes)
            .collect::<Vec<_>>(),
        // The fixture namespace holds 18 bytes, between the 16- and 20-byte
        // namespaces.
        [52, 48, 44, 40, 36, 32, 28, 24, 20, 18]
    );
    assert_eq!(
        report
            .usage(CapacityResource::KvNamespaceBytes)
            .unwrap()
            .used,
        52
    );
}

#[test]
fn merkle_epoch_limit_is_the_last_epoch_with_a_canonical_back_pointer_array() {
    let limit = CapacityResource::MerkleEpoch.limit(&foks_server_db::Config::default());
    assert!(foks_merkle_store::back_pointer_sequence(limit).len() < 16);
    assert_eq!(
        foks_merkle_store::back_pointer_sequence(limit + 1).len(),
        16
    );
    let labels = CapacityResource::ALL.map(CapacityResource::label);
    assert!(labels
        .iter()
        .enumerate()
        .all(|(index, label)| !labels[..index].contains(label)));
}

fn reader_pragma(path: &std::path::Path, name: &str) -> u64 {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    connection
        .pragma_query_value(None, name, |row| row.get::<_, i64>(0))
        .unwrap()
        .try_into()
        .unwrap()
}
