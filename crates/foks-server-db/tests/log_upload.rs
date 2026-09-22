mod common;

use foks_server_db::{
    Error, LogSendBlockMutation, LogSendFileMutation, MAXIMUM_LOG_SEND_BLOCK_BYTES,
    MAXIMUM_LOG_SEND_FILES,
};

#[test]
fn log_send_uploads_accept_go_uncompressed_length_and_bound_actual_bytes() {
    let mut store = common::TestDatabase::new();
    let id = log_send_id(3);
    store
        .database
        .begin_log_send(&id, Some(&[1; 33]), 1_000_000)
        .unwrap();
    let hash = [7; 32];
    store
        .database
        .begin_log_send_file(&LogSendFileMutation {
            log_send_id: &id,
            file_id: 9,
            filename: "client.log",
            // Client reports uncompressed content length while sending
            // gzip-compressed blocks.
            content_length: 10 * 1024 * 1024,
            block_count: 2,
            content_hash: &hash,
            now: 1_000_001,
        })
        .unwrap();
    let first = vec![0x41; MAXIMUM_LOG_SEND_BLOCK_BYTES];
    store
        .database
        .put_log_send_block(&LogSendBlockMutation {
            log_send_id: &id,
            file_id: 9,
            block_number: 0,
            block: &first,
            now: 1_000_002,
        })
        .unwrap();
    store
        .database
        .put_log_send_block(&LogSendBlockMutation {
            log_send_id: &id,
            file_id: 9,
            block_number: 1,
            block: b"xy",
            now: 1_000_003,
        })
        .unwrap();
    assert!(matches!(
        store.database.put_log_send_block(&LogSendBlockMutation {
            log_send_id: &id,
            file_id: 9,
            block_number: 1,
            block: b"xy",
            now: 1_000_004,
        }),
        Err(Error::Duplicate("log-send block"))
    ));

    let connection = rusqlite::Connection::open(&store.path).unwrap();
    let stored: (String, i64, i64, Vec<u8>, i64) = connection
        .query_row(
            "SELECT filename, content_length, block_count, content_hash,
                    (SELECT SUM(length(block)) FROM log_send_blocks
                     WHERE log_send_id = log_send_files.log_send_id
                       AND file_id = log_send_files.file_id)
             FROM log_send_files",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        stored,
        (
            "client.log".to_owned(),
            10 * 1024 * 1024,
            2,
            hash.to_vec(),
            MAXIMUM_LOG_SEND_BLOCK_BYTES as i64 + 2,
        )
    );
}

#[test]
fn log_send_rejects_missing_expired_and_inconsistent_uploads() {
    let mut store = common::TestDatabase::new();
    let id = log_send_id(4);
    let hash = [8; 32];
    assert!(matches!(
        store.database.begin_log_send_file(&LogSendFileMutation {
            log_send_id: &id,
            file_id: 1,
            filename: "client.log",
            content_length: 3,
            block_count: 1,
            content_hash: &hash,
            now: 1,
        }),
        Err(Error::NotFound("active log-send session"))
    ));
    store.database.begin_log_send(&id, None, 1).unwrap();
    assert!(matches!(
        store.database.begin_log_send_file(&LogSendFileMutation {
            log_send_id: &id,
            file_id: 1,
            filename: "../client.log",
            content_length: 3,
            block_count: 1,
            content_hash: &hash,
            now: 2,
        }),
        Err(Error::Invalid("log-send file metadata"))
    ));
    store
        .database
        .begin_log_send_file(&LogSendFileMutation {
            log_send_id: &id,
            file_id: 1,
            filename: "client.log",
            content_length: MAXIMUM_LOG_SEND_BLOCK_BYTES as u64 + 1,
            block_count: 1,
            content_hash: &hash,
            now: 2,
        })
        .unwrap();
    assert!(matches!(
        store.database.begin_log_send_file(&LogSendFileMutation {
            log_send_id: &id,
            file_id: 1,
            filename: "client.log",
            content_length: 3,
            block_count: 1,
            content_hash: &hash,
            now: 86_400_000_002,
        }),
        Err(Error::NotFound("active log-send session"))
    ));

    let active = log_send_id(5);
    store.database.begin_log_send(&active, None, 10).unwrap();
    store
        .database
        .begin_log_send_file(&LogSendFileMutation {
            log_send_id: &active,
            file_id: 2,
            filename: "client.log",
            content_length: 3,
            block_count: 1,
            content_hash: &hash,
            now: 11,
        })
        .unwrap();
    assert!(matches!(
        store.database.put_log_send_block(&LogSendBlockMutation {
            log_send_id: &active,
            file_id: 2,
            block_number: 1,
            block: b"wrong",
            now: 12,
        }),
        Err(Error::Invalid("log-send block position"))
    ));
}

#[test]
fn log_send_caps_the_number_of_files_per_public_capability() {
    let mut store = common::TestDatabase::new();
    let id = log_send_id(6);
    let hash = [9; 32];
    store.database.begin_log_send(&id, None, 1).unwrap();
    for file_id in 0..MAXIMUM_LOG_SEND_FILES {
        store
            .database
            .begin_log_send_file(&LogSendFileMutation {
                log_send_id: &id,
                file_id,
                filename: "empty.log",
                content_length: 0,
                block_count: 0,
                content_hash: &hash,
                now: 2,
            })
            .unwrap();
    }
    assert!(matches!(
        store.database.begin_log_send_file(&LogSendFileMutation {
            log_send_id: &id,
            file_id: MAXIMUM_LOG_SEND_FILES,
            filename: "overflow.log",
            content_length: 0,
            block_count: 0,
            content_hash: &hash,
            now: 3,
        }),
        Err(Error::Capacity("log-send file count"))
    ));
}

#[test]
fn log_send_treats_reported_content_length_as_fully_advisory() {
    let mut store = common::TestDatabase::new();
    let id = log_send_id(7);
    let hash = [0x71; 32];
    store.database.begin_log_send(&id, None, 1).unwrap();
    store
        .database
        .begin_log_send_file(&LogSendFileMutation {
            log_send_id: &id,
            file_id: 1,
            filename: "empty-source.log.gz",
            content_length: 0,
            block_count: 1,
            content_hash: &hash,
            now: 2,
        })
        .unwrap();
    store
        .database
        .put_log_send_block(&LogSendBlockMutation {
            log_send_id: &id,
            file_id: 1,
            block_number: 0,
            block: b"gzip-stream",
            now: 3,
        })
        .unwrap();
}

fn log_send_id(fill: u8) -> [u8; 17] {
    let mut id = [fill; 17];
    id[0] = 48;
    id
}
