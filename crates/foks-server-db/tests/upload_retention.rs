mod common;

use foks_proto::Role;
use foks_server_db::{
    Config, Database, Error, KvDirectoryMutation, KvDirentMutation, KvFileChunkMutation,
    ReadDatabase,
};
use rusqlite::{params, Connection};

const UID: [u8; 33] = [1; 33];
const ROOT: [u8; 16] = [0x51; 16];
const ENTRY: [u8; 16] = [0x61; 16];
const NAME: [u8; 32] = [0x71; 32];

fn setup() -> common::TestDatabase {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    directory(&mut test.database, 1);
    test
}

fn directory(database: &mut Database, version: u64) {
    database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &ROOT,
            version,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: &[version as u8],
        })
        .unwrap();
}

fn upload(database: &mut Database, file: &[u8; 16], complete: bool, now: u64) {
    database
        .put_kv_file_chunk(&KvFileChunkMutation {
            uid: &UID,
            file_id: file,
            exact_metadata: Some(b"metadata"),
            offset: 0,
            ciphertext: b"ciphertext",
            final_size: complete.then_some(10),
            exact_chunk: b"chunk",
            now,
        })
        .unwrap();
}

fn link(
    database: &mut Database,
    file: Option<&[u8; 16]>,
    version: u64,
) -> foks_server_db::Result<()> {
    let mut node = [0; 17];
    if let Some(file) = file {
        node[0] = 2;
        node[1..].copy_from_slice(file);
    }
    database.put_kv_dirents(
        &UID,
        None,
        Role::OWNER,
        &[KvDirentMutation {
            parent: &ROOT,
            id: &ENTRY,
            version,
            directory_version: version,
            node_id: &node,
            name_mac: &NAME,
            creation_time: 10,
            exact: &[version as u8],
        }],
    )
}

fn drain(database: &mut Database, cutoff: u64) -> u64 {
    let mut deleted = 0;
    for _ in 0..100 {
        let report = database.run_maintenance(1000, cutoff).unwrap();
        assert!(report.upload_candidates_examined <= 128);
        assert!(report.upload_chunks <= 16);
        assert!(report.upload_bytes <= 32 * 1024 * 1024);
        deleted += report.uploads;
        if !report.upload_cleanup_deferred {
            return deleted;
        }
    }
    panic!("maintenance failed to finish bounded work");
}

#[test]
fn incomplete_and_complete_unlinked_uploads_obey_cutoff() {
    let mut test = setup();
    for (file, complete) in [([1; 16], false), ([2; 16], true)] {
        upload(&mut test.database, &file, complete, 10);
    }
    assert_eq!(drain(&mut test.database, 9), 0);
    assert_eq!(drain(&mut test.database, 10), 2);
    assert_eq!(drain(&mut test.database, 10), 0);
    assert!(test.database.integrity_check().unwrap());
}

#[test]
fn overwritten_and_unlinked_versions_remain_readable() {
    let mut test = setup();
    let old = [1; 16];
    let new = [2; 16];
    upload(&mut test.database, &old, true, 10);
    upload(&mut test.database, &new, true, 10);
    link(&mut test.database, Some(&old), 1).unwrap();
    // Public directory creation is immutable; seed retained directory versions
    // directly to exercise the database's supported historical lookup predicate.
    let connection = Connection::open(&test.path).unwrap();
    for version in [2, 3] {
        connection
            .execute(
                "INSERT INTO kv_directories VALUES (?1, ?2, ?3, 3, 0, 1, 0, X'01')",
                params![UID, ROOT, version],
            )
            .unwrap();
    }
    link(&mut test.database, Some(&new), 2).unwrap();
    link(&mut test.database, None, 3).unwrap();
    assert_eq!(drain(&mut test.database, 100), 0);
    let reader = ReadDatabase::open(&test.path, Config::default()).unwrap();
    for (version, file) in [(1, old), (2, new)] {
        let historical = reader
            .kv_dirent_at_name(&UID, &ROOT, version, &NAME)
            .unwrap()
            .unwrap();
        assert_eq!(&historical.node_id[1..], file);
        assert_eq!(
            reader.kv_file(&UID, &file).unwrap().unwrap().exact_metadata,
            b"metadata"
        );
        assert_eq!(
            reader
                .kv_file_chunk(&UID, &file, 0)
                .unwrap()
                .unwrap()
                .ciphertext,
            b"ciphertext"
        );
    }
}

#[test]
fn publication_before_cleanup_survives_lost_response_and_incomplete_link() {
    for complete in [false, true] {
        let mut test = setup();
        let file = [1; 16];
        upload(&mut test.database, &file, complete, 10);
        link(&mut test.database, Some(&file), 1).unwrap();
        assert_eq!(drain(&mut test.database, 100), 0);
        // Exact retry after a lost successful publication response is harmless.
        link(&mut test.database, Some(&file), 1).unwrap();
        let connection = Connection::open(&test.path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT reclaiming FROM kv_file_uploads", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn cleanup_before_publication_rejects_link() {
    let mut test = setup();
    let file = [1; 16];
    upload(&mut test.database, &file, true, 10);
    assert_eq!(drain(&mut test.database, 100), 1);
    assert!(matches!(
        link(&mut test.database, Some(&file), 1),
        Err(Error::KvConflict)
    ));
    assert!(test
        .database
        .kv_dirent(&UID, &ROOT, &ENTRY)
        .unwrap()
        .is_none());
}

#[test]
fn staged_reclamation_fences_reads_replays_and_publication_and_resumes_after_restart() {
    let mut test = setup();
    let file = [1; 16];
    for offset in 0..40 {
        test.database
            .put_kv_file_chunk(&KvFileChunkMutation {
                uid: &UID,
                file_id: &file,
                exact_metadata: (offset == 0).then_some(b"metadata".as_slice()),
                offset,
                ciphertext: b"x",
                final_size: (offset == 39).then_some(40),
                exact_chunk: &[offset as u8],
                now: 10,
            })
            .unwrap();
    }
    let first = test.database.run_maintenance(1000, 100).unwrap();
    assert!(first.upload_cleanup_deferred);
    assert!(first.upload_chunks <= 16);
    assert_eq!(first.uploads, 0);
    let reader = ReadDatabase::open(&test.path, Config::default()).unwrap();
    assert!(reader.kv_file(&UID, &file).unwrap().is_none());
    assert!(reader.kv_file_chunk(&UID, &file, 39).unwrap().is_none());
    // This chunk still exists. The fence must run before exact-replay success.
    assert!(matches!(
        test.database.put_kv_file_chunk(&KvFileChunkMutation {
            uid: &UID,
            file_id: &file,
            exact_metadata: None,
            offset: 39,
            ciphertext: b"x",
            final_size: Some(40),
            exact_chunk: &[39],
            now: 10,
        }),
        Err(Error::KvConflict)
    ));
    assert!(matches!(
        link(&mut test.database, Some(&file), 1),
        Err(Error::KvConflict)
    ));
    drop(reader);
    drop(test.database);
    let mut reopened = Database::open(&test.path, Config::default()).unwrap();
    assert_eq!(drain(&mut reopened, 0), 1); // Reclamation does not depend on a later clock.
    assert!(reopened.integrity_check().unwrap());
}

#[test]
fn chunk_payload_bytes_are_bounded_as_well_as_rows() {
    let mut test = setup();
    let file = [1; 16];
    let payload = vec![7; 9 * 1024 * 1024];
    for offset in 0..3 {
        test.database
            .put_kv_file_chunk(&KvFileChunkMutation {
                uid: &UID,
                file_id: &file,
                exact_metadata: (offset == 0).then_some(b"metadata".as_slice()),
                offset,
                ciphertext: &payload,
                final_size: None,
                exact_chunk: &payload,
                now: 10,
            })
            .unwrap();
    }
    let first = test.database.run_maintenance(1000, 100).unwrap();
    assert!(first.upload_bytes <= 32 * 1024 * 1024);
    assert!(first.upload_chunks <= 1);
    assert!(first.upload_cleanup_deferred);
    assert_eq!(drain(&mut test.database, 100), 1);
}

#[test]
fn equal_file_ids_in_distinct_party_namespaces_do_not_share_liveness() {
    let mut test = setup();
    let file = [1; 16];
    upload(&mut test.database, &file, true, 10);
    link(&mut test.database, Some(&file), 1).unwrap();
    let connection = Connection::open(&test.path).unwrap();
    // Storage fixture: exercise ordinary and big-team namespace discriminators.
    for kind in [3u8, 20] {
        let party = [kind; 33];
        connection
            .execute(
                "INSERT INTO kv_namespaces VALUES (?1, ?2, zeroblob(33))",
                params![party, kind],
            )
            .unwrap();
        connection.execute("INSERT INTO kv_file_uploads(uid, file_id, exact_metadata, complete, created_at, updated_at) VALUES (?1, ?2, X'01', 1, 10, 10)", params![party, file]).unwrap();
    }
    assert_eq!(drain(&mut test.database, 100), 2);
    let reader = ReadDatabase::open(&test.path, Config::default()).unwrap();
    assert!(reader.kv_file(&UID, &file).unwrap().is_some());
}

#[test]
fn candidate_scan_advances_past_large_live_history_and_survives_restart() {
    let mut test = setup();
    let connection = Connection::open(&test.path).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    for n in 0u64..300 {
        let mut file = [0; 16];
        file[..8].copy_from_slice(&n.to_be_bytes());
        let mut node = [0; 17];
        node[0] = 2;
        node[1..].copy_from_slice(&file);
        connection.execute("INSERT INTO kv_file_uploads(uid, file_id, exact_metadata, complete, created_at, updated_at) VALUES (?1, ?2, X'01', 1, 10, 10)", params![UID, file]).unwrap();
        connection
            .execute(
                "INSERT INTO kv_dirents VALUES (?1, ?2, ?3, 1, 1, ?4, zeroblob(32), 0, X'01')",
                params![UID, ROOT, file, node],
            )
            .unwrap();
    }
    connection.execute_batch("COMMIT").unwrap();
    upload(&mut test.database, &[255; 16], true, 10);
    let first = test.database.run_maintenance(1000, 100).unwrap();
    assert!(first.upload_candidates_examined <= 128);
    assert_eq!(first.uploads, 0);
    assert!(first.upload_cleanup_deferred);
    drop(test.database);
    let mut reopened = Database::open(&test.path, Config::default()).unwrap();
    assert_eq!(drain(&mut reopened, 100), 1);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM kv_file_uploads", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        300
    );
}

#[test]
fn reference_and_age_queries_use_indexes_without_sorting_history() {
    let test = setup();
    let connection = Connection::open(&test.path).unwrap();
    for (sql, expected) in [
        ("EXPLAIN QUERY PLAN SELECT 1 FROM kv_dirents WHERE uid = zeroblob(33) AND substr(node_id, 1, 1) = X'02' AND substr(node_id, 2, 16) = zeroblob(16)", "kv_dirent_file_reference"),
        ("EXPLAIN QUERY PLAN SELECT updated_at, uid, file_id FROM kv_file_uploads WHERE reclaiming = 0 AND updated_at <= 100 AND (updated_at, uid, file_id) > (0, X'', X'') ORDER BY updated_at, uid, file_id LIMIT 128", "kv_upload_age"),
        ("EXPLAIN QUERY PLAN SELECT uid, file_id FROM kv_file_uploads WHERE reclaiming = 1 ORDER BY updated_at, uid, file_id LIMIT 128", "kv_upload_reclaiming"),
    ] {
        let mut statement = connection.prepare(sql).unwrap();
        let plan = statement.query_map([], |r| r.get::<_, String>(3)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
        assert!(plan.contains(expected), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
}
