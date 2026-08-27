CREATE TABLE kv_directories (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    directory_id BLOB NOT NULL CHECK (length(directory_id) = 16),
    version INTEGER NOT NULL CHECK (version >= 1),
    key_role INTEGER NOT NULL CHECK (key_role BETWEEN 0 AND 3),
    key_visibility INTEGER NOT NULL,
    key_generation INTEGER NOT NULL CHECK (key_generation >= 1),
    status INTEGER NOT NULL CHECK (status BETWEEN 0 AND 2),
    exact_directory BLOB NOT NULL,
    PRIMARY KEY (uid, directory_id, version)
) STRICT;

CREATE TABLE kv_directory_heads (
    uid BLOB NOT NULL,
    directory_id BLOB NOT NULL,
    version INTEGER NOT NULL,
    PRIMARY KEY (uid, directory_id),
    FOREIGN KEY (uid, directory_id, version)
        REFERENCES kv_directories(uid, directory_id, version) ON DELETE CASCADE
) STRICT;

CREATE TABLE kv_roots (
    uid BLOB PRIMARY KEY REFERENCES users(uid) ON DELETE CASCADE,
    root_version INTEGER NOT NULL CHECK (root_version >= 1),
    directory_id BLOB NOT NULL CHECK (length(directory_id) = 16),
    directory_version INTEGER NOT NULL CHECK (directory_version >= 1),
    exact_root BLOB NOT NULL,
    FOREIGN KEY (uid, directory_id, directory_version)
        REFERENCES kv_directories(uid, directory_id, version)
) STRICT;

CREATE TABLE kv_nodes (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    node_id BLOB NOT NULL CHECK (length(node_id) = 17),
    node_type INTEGER NOT NULL CHECK (node_type IN (3, 4)),
    exact_node BLOB NOT NULL,
    PRIMARY KEY (uid, node_id),
    CHECK (
        (node_type = 3 AND substr(node_id, 1, 1) = X'03') OR
        (node_type = 4 AND substr(node_id, 1, 1) = X'04')
    )
) STRICT;

CREATE TABLE kv_dirents (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    parent_id BLOB NOT NULL CHECK (length(parent_id) = 16),
    dirent_id BLOB NOT NULL CHECK (length(dirent_id) = 16),
    version INTEGER NOT NULL CHECK (version >= 1),
    directory_version INTEGER NOT NULL CHECK (directory_version >= 1),
    node_id BLOB NOT NULL CHECK (length(node_id) = 17),
    name_mac BLOB NOT NULL CHECK (length(name_mac) = 32),
    creation_time INTEGER NOT NULL CHECK (creation_time >= 0),
    exact_dirent BLOB NOT NULL,
    PRIMARY KEY (uid, parent_id, dirent_id, version),
    FOREIGN KEY (uid, parent_id, directory_version)
        REFERENCES kv_directories(uid, directory_id, version)
) STRICT;

CREATE TABLE kv_dirent_heads (
    uid BLOB NOT NULL,
    parent_id BLOB NOT NULL,
    dirent_id BLOB NOT NULL,
    version INTEGER NOT NULL,
    PRIMARY KEY (uid, parent_id, dirent_id),
    FOREIGN KEY (uid, parent_id, dirent_id, version)
        REFERENCES kv_dirents(uid, parent_id, dirent_id, version) ON DELETE CASCADE
) STRICT;

CREATE INDEX kv_dirent_listing
    ON kv_dirents(uid, parent_id, name_mac, dirent_id, version);

CREATE TABLE kv_file_uploads (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    file_id BLOB NOT NULL CHECK (length(file_id) = 16),
    exact_metadata BLOB NOT NULL,
    complete INTEGER NOT NULL DEFAULT 0 CHECK (complete IN (0, 1)),
    encrypted_size INTEGER CHECK (encrypted_size IS NULL OR encrypted_size >= 1),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    PRIMARY KEY (uid, file_id)
) STRICT;

CREATE TABLE kv_file_chunks (
    uid BLOB NOT NULL,
    file_id BLOB NOT NULL,
    clear_offset INTEGER NOT NULL CHECK (clear_offset >= 0),
    ciphertext BLOB NOT NULL,
    final_chunk INTEGER NOT NULL CHECK (final_chunk IN (0, 1)),
    exact_upload_chunk BLOB NOT NULL,
    PRIMARY KEY (uid, file_id, clear_offset),
    FOREIGN KEY (uid, file_id) REFERENCES kv_file_uploads(uid, file_id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX kv_file_one_final_chunk
    ON kv_file_chunks(uid, file_id) WHERE final_chunk = 1;

CREATE TABLE kv_locks (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    parent_id BLOB NOT NULL CHECK (length(parent_id) = 16),
    dirent_id BLOB NOT NULL CHECK (length(dirent_id) = 16),
    lock_id BLOB NOT NULL CHECK (length(lock_id) = 16),
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
    PRIMARY KEY (uid, parent_id, dirent_id)
) STRICT;
