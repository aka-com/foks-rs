pub(crate) const APPLICATION_ID: i64 = 0x464b_5653; // `FKVS`
pub(crate) const VERSION: u32 = 3;

pub(crate) const INITIAL: &str = r#"
CREATE TABLE kv_parties (
    host_id BLOB NOT NULL CHECK (length(host_id) = 33),
    party_id BLOB NOT NULL CHECK (length(party_id) = 33),
    root_version INTEGER NOT NULL CHECK (root_version > 0),
    root_dir_id BLOB NOT NULL CHECK (length(root_dir_id) = 16),
    root_bytes BLOB NOT NULL CHECK (length(root_bytes) > 0),
    cache_complete INTEGER NOT NULL DEFAULT 0 CHECK (cache_complete IN (0, 1)),
    PRIMARY KEY (host_id, party_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE kv_directories (
    host_id BLOB NOT NULL,
    party_id BLOB NOT NULL,
    dir_id BLOB NOT NULL CHECK (length(dir_id) = 16),
    version INTEGER NOT NULL CHECK (version > 0),
    dir_bytes BLOB NOT NULL CHECK (length(dir_bytes) > 0),
    PRIMARY KEY (host_id, party_id, dir_id),
    FOREIGN KEY (host_id, party_id) REFERENCES kv_parties(host_id, party_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE kv_entries (
    host_id BLOB NOT NULL,
    party_id BLOB NOT NULL,
    parent_dir_id BLOB NOT NULL CHECK (length(parent_dir_id) = 16),
    dirent_id BLOB NOT NULL CHECK (length(dirent_id) = 16),
    node_id BLOB NOT NULL CHECK (length(node_id) = 17),
    version INTEGER NOT NULL CHECK (version > 0),
    dir_version INTEGER NOT NULL CHECK (dir_version > 0),
    name BLOB NOT NULL CHECK (length(name) > 0),
    write_role_type INTEGER NOT NULL CHECK (write_role_type BETWEEN 1 AND 3),
    write_role_visibility INTEGER NOT NULL,
    creation_time INTEGER NOT NULL CHECK (creation_time >= 0),
    dirent_bytes BLOB NOT NULL CHECK (length(dirent_bytes) > 0),
    node_bytes BLOB,
    content BLOB,
    symlink BLOB,
    large_file_id INTEGER,
    PRIMARY KEY (host_id, party_id, parent_dir_id, dirent_id),
    UNIQUE (host_id, party_id, parent_dir_id, name),
    FOREIGN KEY (host_id, party_id, parent_dir_id)
        REFERENCES kv_directories(host_id, party_id, dir_id) ON DELETE CASCADE,
    FOREIGN KEY (large_file_id) REFERENCES kv_large_files(id)
) STRICT, WITHOUT ROWID;

CREATE TABLE kv_entry_history (
    host_id BLOB NOT NULL,
    party_id BLOB NOT NULL,
    parent_dir_id BLOB NOT NULL CHECK (length(parent_dir_id) = 16),
    dirent_id BLOB NOT NULL CHECK (length(dirent_id) = 16),
    version INTEGER NOT NULL CHECK (version > 0),
    dirent_bytes BLOB NOT NULL CHECK (length(dirent_bytes) > 0),
    present INTEGER NOT NULL CHECK (present IN (0, 1)),
    PRIMARY KEY (host_id, party_id, parent_dir_id, dirent_id),
    FOREIGN KEY (host_id, party_id, parent_dir_id)
        REFERENCES kv_directories(host_id, party_id, dir_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE kv_large_files (
    id INTEGER PRIMARY KEY,
    host_id BLOB NOT NULL CHECK (length(host_id) = 33),
    party_id BLOB NOT NULL CHECK (length(party_id) = 33),
    node_id BLOB NOT NULL CHECK (length(node_id) = 17),
    size INTEGER NOT NULL CHECK (size >= 0),
    complete INTEGER NOT NULL CHECK (complete IN (0, 1))
) STRICT;

CREATE TABLE kv_large_file_chunks (
    file_id INTEGER NOT NULL,
    offset INTEGER NOT NULL CHECK (offset >= 0),
    content BLOB NOT NULL CHECK (length(content) > 0),
    PRIMARY KEY (file_id, offset),
    FOREIGN KEY (file_id) REFERENCES kv_large_files(id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- Beacon answers are routing hints, never trust anchors. Keeping them in the
-- replaceable soft-state database makes that distinction structural.
CREATE TABLE federation_discovery_hints (
    host_id BLOB PRIMARY KEY CHECK (length(host_id) = 33),
    address TEXT NOT NULL CHECK (length(address) BETWEEN 1 AND 512),
    observed_at INTEGER NOT NULL CHECK (observed_at >= 0),
    last_used_at INTEGER NOT NULL CHECK (last_used_at >= 0)
) STRICT, WITHOUT ROWID;
"#;
