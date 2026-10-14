mod common;

use foks_proto::{KvUploadChunk, KvUploadFinal};
use foks_server_db::{Config, Error, KvChunkPayload, KvFileChunkMutation, ReadDatabase};
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};

const UID: [u8; 33] = [1; 33];
const FILE: [u8; 16] = [2; 16];

fn setup(config: Config) -> common::TestDatabase {
    let mut test = common::TestDatabase::with_config(config);
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    test
}

fn encode(bytes: &[u8], offset: u64, final_size: Option<u64>) -> Vec<u8> {
    KvUploadChunk {
        ciphertext: bytes.to_vec(),
        offset,
        final_upload: final_size.map(|size| KvUploadFinal {
            size,
            chunk_sum: [7; 32],
        }),
    }
    .encode()
    .unwrap()
}

fn digest(exact: &[u8]) -> [u8; 32] {
    Sha256::digest(exact).into()
}

fn totals(c: &Connection) -> (i64, i64, i64, i64) {
    c.query_row(
        "SELECT accounted_bytes,accounted_objects,
        coalesce((SELECT sum(chunk_count) FROM kv_file_uploads),0),
        coalesce((SELECT sum(chunk_bytes) FROM kv_file_uploads),0)
        FROM kv_namespaces WHERE namespace_id=?1",
        [UID],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

#[test]
fn mixed_uploads_preserve_exact_retries_usage_and_charge_through_conversion() {
    let mut test = setup(Config::default());
    let c = Connection::open(&test.path).unwrap();
    let first = encode(b"abc", 0, None);
    let last = encode(b"de", 4, Some(5));
    let mutation = KvFileChunkMutation {
        uid: &UID,
        file_id: &FILE,
        exact_metadata: Some(b"meta"),
        offset: 0,
        ciphertext: b"abc",
        final_size: None,
        exact_chunk: &first,
        now: 1,
    };
    test.database.preflight_kv_file_chunk(&mutation).unwrap();
    assert_eq!(totals(&c), (0, 0, 0, 0));
    assert_eq!(
        c.query_row("SELECT count(*) FROM kv_file_uploads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    test.database
        .put_kv_file_chunk_external(&mutation, &digest(&first))
        .unwrap();
    test.database
        .put_kv_file_chunk_external(&mutation, &digest(&first))
        .unwrap();
    // A replay through the inline path must recognize an external row too.
    test.database.put_kv_file_chunk(&mutation).unwrap();
    let reader = ReadDatabase::open(&test.path, Config::default()).unwrap();
    assert!(reader.has_external_chunks().unwrap());
    assert!(reader.kv_file_chunk(&UID, &FILE, 0).unwrap().is_none());
    let final_mutation = KvFileChunkMutation {
        exact_metadata: None,
        offset: 4,
        ciphertext: b"de",
        final_size: Some(5),
        exact_chunk: &last,
        now: 2,
        ..mutation
    };
    test.database.put_kv_file_chunk(&final_mutation).unwrap();
    // Replay external publication against a preexisting inline chunk is a no-op.
    test.database
        .put_kv_file_chunk_external(&final_mutation, &digest(&last))
        .unwrap();
    let original = totals(&c);
    assert_eq!(
        original,
        (4 + 5 + first.len() as i64 + last.len() as i64, 3, 2, 5)
    );
    let usage = reader.kv_usage(&UID).unwrap();
    assert_eq!(usage.large.base.bytes, 5);
    assert_eq!(usage.large.base.number, 1);
    assert_eq!(usage.large.chunks, 2);
    assert!(matches!(
        reader
            .kv_file_chunk(&UID, &FILE, 0)
            .unwrap()
            .unwrap()
            .payload,
        KvChunkPayload::External(_)
    ));
    assert_eq!(
        reader
            .kv_file_chunk(&UID, &FILE, 4)
            .unwrap()
            .unwrap()
            .payload,
        KvChunkPayload::Inline(b"de".to_vec())
    );
    let inline = test.database.next_kv_chunk(None, false).unwrap().unwrap();
    assert_eq!(inline.key.offset, 4);
    assert_eq!(inline.exact_chunk.as_deref(), Some(last.as_slice()));
    assert!(test
        .database
        .externalize_kv_chunk(&inline.key, &last, &digest(&last))
        .unwrap());
    assert!(!test
        .database
        .externalize_kv_chunk(&inline.key, &last, &digest(&last))
        .unwrap());
    assert!(test.database.next_kv_chunk(None, false).unwrap().is_none());
    assert_eq!(totals(&c), original);
    // Cursor survives changing the previous row's representation.
    let mut after = None;
    let mut hydrated = 0;
    while let Some(object) = test.database.next_kv_chunk(after.as_ref(), true).unwrap() {
        let KvChunkPayload::External(reference) = &object.chunk.payload else {
            panic!("external cursor returned inline")
        };
        let (exact, bytes) = if object.key.offset == 0 {
            (&first, b"abc".as_slice())
        } else {
            (&last, b"de".as_slice())
        };
        assert!(test
            .database
            .hydrate_kv_chunk(&object.key, reference, exact, bytes)
            .unwrap());
        assert!(!test
            .database
            .hydrate_kv_chunk(&object.key, reference, exact, bytes)
            .unwrap());
        assert_eq!(totals(&c), original);
        hydrated += 1;
        after = Some(object.key);
    }
    assert_eq!(hydrated, 2);
    assert!(!test.database.has_external_chunks().unwrap());
    assert_eq!(reader.kv_usage(&UID).unwrap(), usage);
    assert_eq!(
        reader
            .kv_file_chunk(&UID, &FILE, 0)
            .unwrap()
            .unwrap()
            .payload,
        KvChunkPayload::Inline(b"abc".to_vec())
    );
    // Logical counter removal applies even though external payload is absent.
    let inline = test.database.next_kv_chunk(None, false).unwrap().unwrap();
    test.database
        .externalize_kv_chunk(&inline.key, &first, &digest(&first))
        .unwrap();
    test.database.run_maintenance(1000, 100).unwrap();
    assert_eq!(totals(&c), (0, 0, 0, 0));
    assert!(!test.database.has_external_chunks().unwrap());
}

#[test]
fn external_admission_rejects_digest_field_order_and_quota_errors_atomically() {
    let first = encode(b"abc", 0, None);
    let charge = 4 + 3 + first.len() as u64;
    let mut test = setup(Config {
        maximum_kv_namespace_bytes: charge,
        ..Config::default()
    });
    let c = Connection::open(&test.path).unwrap();
    let mutation = KvFileChunkMutation {
        uid: &UID,
        file_id: &FILE,
        exact_metadata: Some(b"meta"),
        offset: 0,
        ciphertext: b"abc",
        final_size: None,
        exact_chunk: &first,
        now: 1,
    };
    assert!(matches!(
        test.database
            .put_kv_file_chunk_external(&mutation, &[0; 32]),
        Err(Error::Invalid(_))
    ));
    test.database.preflight_kv_file_chunk(&mutation).unwrap();
    test.database
        .put_kv_file_chunk_external(&mutation, &digest(&first))
        .unwrap();
    let before = totals(&c);
    let conflict = encode(b"xyz", 0, None);
    assert!(matches!(
        test.database.preflight_kv_file_chunk(&KvFileChunkMutation {
            ciphertext: b"xyz",
            exact_chunk: &conflict,
            ..mutation
        }),
        Err(Error::KvConflict)
    ));
    assert!(matches!(
        test.database.put_kv_file_chunk_external(
            &KvFileChunkMutation {
                ciphertext: b"xyz",
                exact_chunk: &conflict,
                ..mutation
            },
            &digest(&conflict)
        ),
        Err(Error::KvConflict)
    ));
    let next = encode(b"de", 4, Some(5));
    let final_mutation = KvFileChunkMutation {
        exact_metadata: None,
        offset: 4,
        ciphertext: b"de",
        final_size: Some(5),
        exact_chunk: &next,
        ..mutation
    };
    assert!(matches!(
        test.database.preflight_kv_file_chunk(&final_mutation),
        Err(Error::QuotaExceeded)
    ));
    assert!(matches!(
        test.database
            .put_kv_file_chunk_external(&final_mutation, &digest(&next)),
        Err(Error::QuotaExceeded)
    ));
    assert!(matches!(
        test.database.put_kv_file_chunk_external(
            &KvFileChunkMutation {
                ciphertext: b"xx",
                ..final_mutation
            },
            &digest(&next)
        ),
        Err(Error::Invalid(_))
    ));
    assert_eq!(totals(&c), before);
    let object = test.database.next_kv_chunk(None, true).unwrap().unwrap();
    let KvChunkPayload::External(reference) = object.chunk.payload else {
        panic!()
    };
    assert!(test
        .database
        .hydrate_kv_chunk(&object.key, &reference, &first, b"xyz")
        .is_err());
    assert!(test
        .database
        .hydrate_kv_chunk(&object.key, &reference, &conflict, b"xyz")
        .is_err());
    assert_eq!(totals(&c), before);
    // Strict representation constraints prevent half-external rows and stored
    // payload duplicates even from maintenance SQL.
    assert!(c
        .execute("UPDATE kv_file_chunks SET ciphertext=X'01'", [])
        .is_err());
    assert!(c
        .execute("UPDATE kv_file_chunks SET external_digest=NULL", [])
        .is_err());
    assert!(c
        .execute("UPDATE kv_file_chunks SET external_encoded_bytes=NULL", [])
        .is_err());
    assert_eq!(totals(&c), before);
}

#[test]
fn preflight_does_not_reserve_capacity_and_commit_rechecks_after_intervening_write() {
    let first = encode(b"abc", 0, Some(3));
    let mut test = setup(Config {
        maximum_kv_namespace_objects: 2,
        ..Config::default()
    });
    let mutation = KvFileChunkMutation {
        uid: &UID,
        file_id: &FILE,
        exact_metadata: Some(b"meta"),
        offset: 0,
        ciphertext: b"abc",
        final_size: Some(3),
        exact_chunk: &first,
        now: 1,
    };
    test.database.preflight_kv_file_chunk(&mutation).unwrap();
    test.database
        .put_kv_file_chunk_external(
            &KvFileChunkMutation {
                file_id: &[3; 16],
                ..mutation
            },
            &digest(&first),
        )
        .unwrap();
    assert!(matches!(
        test.database
            .put_kv_file_chunk_external(&mutation, &digest(&first)),
        Err(Error::QuotaExceeded)
    ));
    let c = Connection::open(&test.path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM kv_file_uploads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
