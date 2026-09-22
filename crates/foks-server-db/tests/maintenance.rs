mod common;

use foks_server_db::{
    Error, KvDirectoryMutation, KvFileChunkMutation, KvNodeMutation, KvRootMutation,
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
        .put_kv_file_chunk(&KvFileChunkMutation {
            uid: &UID,
            file_id: &[0x82; 16],
            exact_metadata: Some(b"finalized-metadata"),
            offset: 0,
            ciphertext: b"finalized-but-unlinked",
            final_size: Some(22),
            exact_chunk: b"finalized-chunk",
            now: 10,
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
fn maintenance_reclaims_only_expired_or_abandoned_state() {
    let (mut test, root) = initialized_database();
    test.database
        .reserve_name(b"expired", &[0x41; 17], 1, 10, 20)
        .unwrap();
    test.database
        .acquire_kv_lock(&UID, &root, &[0x61; 16], &[0x71; 16], 10, 20)
        .unwrap();
    test.database
        .put_kv_file_chunk(&KvFileChunkMutation {
            uid: &UID,
            file_id: &[0x81; 16],
            exact_metadata: Some(b"metadata"),
            offset: 0,
            ciphertext: b"incomplete",
            final_size: None,
            exact_chunk: b"chunk",
            now: 10,
        })
        .unwrap();
    {
        let connection = rusqlite::Connection::open(&test.path).unwrap();
        connection
            .execute(
                "INSERT INTO capability_key_generations
                 (generation_id, encrypted_file_name, state, created_at, retire_after)
                 VALUES (?1, 'capability.key', 1, 1, NULL)",
                [[0x90_u8; 16]],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO federation_user_view_permissions
                 (target_user_id, viewer_party_id, viewer_host_id, token_hash,
                  token_nonce, token_ciphertext, key_generation, state, issued_at,
                  updated_at, expires_at, revoked_at)
                 VALUES (?1, ?2, ?3, zeroblob(32), zeroblob(24), zeroblob(33),
                         ?4, 1, 1, 1, 20, NULL)",
                rusqlite::params![UID, [1_u8; 33], [2_u8; 33], [0x90_u8; 16]],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO log_sends(log_send_id, uid, created_at)
                 VALUES (?1, NULL, 0)",
                [[0x48_u8; 17]],
            )
            .unwrap();
    }

    let report = test.database.run_maintenance(21, 11).unwrap();
    assert_eq!(report.reservations, 1);
    assert_eq!(report.locks, 0);
    assert_eq!(report.uploads, 2);
    assert_eq!(report.log_sends, 1);
    assert_eq!(report.idempotency_records, 0);
    assert_eq!(report.challenges, 0);
    assert_eq!(report.team_reservations, 0);
    assert_eq!(report.team_view_challenges, 0);
    assert_eq!(report.team_view_tokens, 0);
    assert_eq!(report.federation_user_permissions, 0);
    assert_eq!(report.federation_team_permissions, 0);
    let retained: i64 = rusqlite::Connection::open(&test.path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM federation_user_view_permissions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 1);
    assert!(test.database.integrity_check().unwrap());
    let checkpoint = test.database.checkpoint().unwrap();
    assert_eq!(checkpoint.busy, 0);
    assert!(test.database.storage_report().unwrap().database_bytes > 0);
}

#[test]
fn namespace_and_database_limits_are_explicit() {
    let (test, _) = initialized_database();
    let path = test.path.clone();
    drop(test.database);
    let config = foks_server_db::Config {
        maximum_kv_namespace_bytes: 32,
        maximum_kv_namespace_objects: 3,
        ..foks_server_db::Config::default()
    };
    let mut database = foks_server_db::Database::open(&path, config).unwrap();
    let node = [3; 17];
    assert!(matches!(
        database.put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &node,
            node_type: 3,
            exact: b"this node exceeds the remaining namespace quota",
        }),
        Err(Error::QuotaExceeded)
    ));

    let invalid = foks_server_db::Config {
        maximum_database_bytes: 0,
        ..foks_server_db::Config::default()
    };
    assert!(matches!(
        foks_server_db::Database::open(path.with_file_name("invalid.sqlite"), invalid),
        Err(Error::Invalid("zero database capacity limit"))
    ));
    let too_small = foks_server_db::Config {
        maximum_database_bytes: 1,
        ..foks_server_db::Config::default()
    };
    assert!(matches!(
        foks_server_db::Database::open(path, too_small),
        Err(Error::QuotaExceeded)
    ));
}
