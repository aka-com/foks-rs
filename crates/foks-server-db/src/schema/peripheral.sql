CREATE TABLE waitlist_entries (
    waitlist_id BLOB PRIMARY KEY NOT NULL CHECK(length(waitlist_id) = 13),
    email TEXT NOT NULL CHECK(length(email) BETWEEN 3 AND 320),
    created_at INTEGER NOT NULL CHECK(created_at >= 0)
) STRICT;

CREATE INDEX waitlist_entries_email_created
    ON waitlist_entries(email, created_at);

CREATE TABLE log_sends (
    log_send_id BLOB PRIMARY KEY NOT NULL CHECK(length(log_send_id) = 17),
    uid BLOB,
    created_at INTEGER NOT NULL CHECK(created_at >= 0)
) STRICT;

CREATE TABLE log_send_files (
    log_send_id BLOB NOT NULL REFERENCES log_sends(log_send_id) ON DELETE CASCADE,
    file_id INTEGER NOT NULL CHECK(file_id >= 0),
    filename TEXT NOT NULL CHECK(length(filename) BETWEEN 1 AND 255),
    -- The client may report uncompressed size while uploading compressed blocks.
    -- Actual bytes are bounded at block insertion time.
    content_length INTEGER NOT NULL CHECK(content_length >= 0),
    block_count INTEGER NOT NULL CHECK(block_count BETWEEN 0 AND 16),
    content_hash BLOB NOT NULL CHECK(length(content_hash) = 32),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    PRIMARY KEY(log_send_id, file_id)
) STRICT;

CREATE TABLE log_send_blocks (
    log_send_id BLOB NOT NULL,
    file_id INTEGER NOT NULL,
    block_number INTEGER NOT NULL CHECK(block_number >= 0),
    block BLOB NOT NULL CHECK(length(block) <= 4194304),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    PRIMARY KEY(log_send_id, file_id, block_number),
    FOREIGN KEY(log_send_id, file_id)
        REFERENCES log_send_files(log_send_id, file_id) ON DELETE CASCADE
) STRICT;
