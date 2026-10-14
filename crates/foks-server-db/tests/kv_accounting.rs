mod common;

use foks_server_db::{
    Config, Error, KvFileChunkMutation, KvNodeMutation, ReadDatabase, ReadDatabaseOpener,
};
use rusqlite::{params, Connection};

const UID: [u8; 33] = [1; 33];

// Intentionally independent of the counter implementation: enumerate the rows
// that the previous quota query charged, including all historical versions.
fn assert_accounting(connection: &Connection) {
    let namespaces: Vec<Vec<u8>> = connection
        .prepare("SELECT namespace_id FROM kv_namespaces")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    for uid in namespaces {
        let mut bytes = 0_i64;
        let mut objects = 0_i64;
        for (table, expression) in [
            ("kv_directories", "length(exact_directory)"),
            ("kv_roots", "length(exact_root)"),
            ("kv_nodes", "length(exact_node)"),
            ("kv_dirents", "length(exact_dirent)"),
            ("kv_file_uploads", "length(exact_metadata)"),
            (
                "kv_file_chunks",
                "length(ciphertext) + length(exact_upload_chunk)",
            ),
            ("kv_locks", "64"),
        ] {
            let (b, n): (i64, i64) = connection
                .query_row(
                    &format!(
                        "SELECT coalesce(sum({expression}),0),count(*) FROM {table} WHERE uid=?1"
                    ),
                    [&uid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            bytes += b;
            objects += n;
        }
        let directories: i64 = connection
            .query_row(
                "SELECT count(*) FROM kv_directory_heads WHERE uid=?1",
                [&uid],
                |r| r.get(0),
            )
            .unwrap();
        let dirents: i64 = connection
            .query_row(
                "SELECT count(*) FROM kv_dirent_heads WHERE uid=?1",
                [&uid],
                |r| r.get(0),
            )
            .unwrap();
        let stored: (i64,i64,i64,i64) = connection.query_row("SELECT accounted_bytes,accounted_objects,directory_count,dirent_count FROM kv_namespaces WHERE namespace_id=?1", [&uid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(stored, (bytes, objects, directories, dirents));
    }
    let mismatches: i64 = connection.query_row("SELECT count(*) FROM kv_file_uploads u WHERE chunk_count != (SELECT count(*) FROM kv_file_chunks c WHERE c.uid=u.uid AND c.file_id=u.file_id) OR chunk_bytes != (SELECT coalesce(sum(length(ciphertext)),0) FROM kv_file_chunks c WHERE c.uid=u.uid AND c.file_id=u.file_id)", [], |r| r.get(0)).unwrap();
    assert_eq!(mismatches, 0);
}

#[test]
fn accounting_tracks_history_updates_rollback_and_cascades() {
    let test = common::TestDatabase::new();
    let mut c = Connection::open(&test.path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF;
        INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33));
        INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'0102');
        INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),2,3,0,1,0,X'010203');
        INSERT INTO kv_directory_heads VALUES(zeroblob(33),zeroblob(16),2);
        INSERT INTO kv_roots VALUES(zeroblob(33),1,zeroblob(16),2,X'01');
        INSERT INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),zeroblob(16),1,1,zeroblob(17),zeroblob(32),0,X'0102');
        INSERT INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),zeroblob(16),2,2,zeroblob(17),zeroblob(32),0,X'010203');
        INSERT INTO kv_dirent_heads VALUES(zeroblob(33),zeroblob(16),zeroblob(16),2);
        INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(zeroblob(33),zeroblob(16),X'0102',0,0);
        INSERT INTO kv_file_chunks VALUES(zeroblob(33),zeroblob(16),0,X'010203',0,X'01020304');
        INSERT INTO kv_locks VALUES(zeroblob(33),zeroblob(16),zeroblob(16),zeroblob(16),0);").unwrap();
    assert_accounting(&c);
    c.execute_batch("UPDATE kv_roots SET exact_root=X'01020304'; UPDATE kv_directory_heads SET version=1; UPDATE kv_dirent_heads SET version=1; UPDATE kv_file_chunks SET ciphertext=X'0102',exact_upload_chunk=X'01'; UPDATE kv_file_uploads SET exact_metadata=X'01020304';").unwrap();
    assert_accounting(&c);
    // Deterministic pseudo-random mutations exercise commits and rollback while
    // retaining old history. Check after every operation, not only at the end.
    let mut random = 71_u64;
    for step in 0..300 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut id = [0_u8; 17];
        id[0] = 3;
        id[1] = ((random >> 32) % 24) as u8;
        let tx = c.transaction().unwrap();
        match (random >> 40) % 3 {
            0 => {
                tx.execute("INSERT INTO kv_nodes(uid,node_id,node_type,exact_node) VALUES(zeroblob(33),?1,3,?2) ON CONFLICT(uid,node_id) DO UPDATE SET exact_node=excluded.exact_node", params![id, vec![1_u8; (step % 31) + 1]]).unwrap();
            }
            1 => {
                tx.execute("DELETE FROM kv_nodes WHERE node_id=?1", [id])
                    .unwrap();
            }
            _ => {
                tx.execute(
                    "UPDATE kv_nodes SET exact_node=?2 WHERE node_id=?1",
                    params![id, vec![2_u8; (step % 13) + 1]],
                )
                .unwrap();
            }
        }
        assert_accounting(&tx);
        if random & 1 == 0 {
            tx.commit().unwrap();
        } else {
            tx.rollback().unwrap();
        }
        assert_accounting(&c);
    }
    c.execute_batch("DELETE FROM kv_file_uploads; DELETE FROM kv_dirents WHERE version=1;")
        .unwrap();
    assert_accounting(&c);
    c.execute_batch("DELETE FROM kv_namespaces;").unwrap();
    assert_accounting(&c);
    assert_eq!(
        c.query_row("SELECT count(*) FROM kv_file_chunks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn replay_and_failed_chunk_do_not_charge_or_leave_partial_uploads() {
    let mut test = common::TestDatabase::with_config(Config {
        maximum_kv_namespace_bytes: 12,
        ..Config::default()
    });
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let c = Connection::open(&test.path).unwrap();
    let first = KvFileChunkMutation {
        uid: &UID,
        file_id: &[9; 16],
        exact_metadata: Some(b"meta"),
        offset: 0,
        ciphertext: b"abc",
        final_size: None,
        exact_chunk: b"exact",
        now: 1,
    };
    test.database.put_kv_file_chunk(&first).unwrap();
    test.database.put_kv_file_chunk(&first).unwrap();
    assert_accounting(&c);
    assert!(matches!(
        test.database.put_kv_file_chunk(&KvFileChunkMutation {
            file_id: &[8; 16],
            ..first
        }),
        Err(Error::QuotaExceeded)
    ));
    assert!(matches!(
        test.database.put_kv_file_chunk(&KvFileChunkMutation {
            exact_metadata: None,
            offset: 1,
            ..first
        }),
        Err(Error::QuotaExceeded)
    ));
    assert_eq!(
        c.query_row("SELECT count(*) FROM kv_file_uploads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_accounting(&c);
}

#[test]
fn independent_writers_serialize_quota_admission() {
    let config = Config {
        maximum_kv_namespace_objects: 1,
        ..Config::default()
    };
    let mut test = common::TestDatabase::with_config(config.clone());
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let path = test.path.clone();
    let mut second = foks_server_db::Database::open(&path, config).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other = barrier.clone();
    let handle = std::thread::spawn(move || {
        other.wait();
        second.put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &[3; 17],
            node_type: 3,
            exact: b"one",
        })
    });
    barrier.wait();
    let a = test.database.put_kv_node(&KvNodeMutation {
        uid: &UID,
        id: &[4; 17],
        node_type: 4,
        exact: b"two",
    });
    let b = handle.join().unwrap();
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(a, Err(Error::QuotaExceeded)) || matches!(b, Err(Error::QuotaExceeded)));
    assert_accounting(&Connection::open(path).unwrap());
}

#[test]
fn pooled_opener_validates_initial_limits_and_rejects_replacement() {
    let test = common::TestDatabase::new();
    let c = Connection::open(&test.path).unwrap();
    c.execute_batch("INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33)); INSERT INTO kv_nodes VALUES(zeroblob(33),X'0300000000000000000000000000000000',3,zeroblob(5));").unwrap();
    let strict = Config {
        maximum_kv_node_bytes: 4,
        ..Config::default()
    };
    assert!(matches!(
        ReadDatabase::open(&test.path, strict.clone()),
        Err(Error::QuotaExceeded)
    ));
    assert!(matches!(
        ReadDatabaseOpener::new(&test.path, strict),
        Err(Error::QuotaExceeded)
    ));
    let opener = ReadDatabaseOpener::new(&test.path, Config::default()).unwrap();
    opener.open().unwrap();
    let moved = test.path.with_extension("moved");
    std::fs::rename(&test.path, &moved).unwrap();
    std::fs::write(&test.path, []).unwrap();
    assert!(matches!(opener.open(), Err(Error::UnsafeDatabasePath(_))));
}
