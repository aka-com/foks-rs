pub(crate) const APPLICATION_ID: i64 = 0x464b_5653; // `FKVS`
pub(crate) const VERSION: u32 = 6;

pub(crate) const KNOWN_STORES_SCHEMA: &str = r#"
CREATE TABLE known_stores (
    kind INTEGER NOT NULL CHECK (kind IN (1, 2)),
    account_alias TEXT NOT NULL CHECK (length(account_alias) BETWEEN 1 AND 255),
    team_alias TEXT NOT NULL,
    team_id_hex TEXT,
    team_kind TEXT CHECK (team_kind IN ('named', 'ad-hoc')),
    display_name TEXT,
    active INTEGER CHECK (active IN (0, 1)),
    last_seen_at INTEGER NOT NULL CHECK (last_seen_at >= 0),
    CHECK (
        (kind = 1 AND team_alias = '' AND team_id_hex IS NULL AND team_kind IS NULL
                  AND display_name IS NULL AND active IS NULL)
        OR
        (kind = 2 AND length(team_alias) BETWEEN 1 AND 255 AND team_id_hex IS NOT NULL
                  AND team_kind IS NOT NULL AND active IS NOT NULL)
    ),
    PRIMARY KEY (kind, account_alias, team_alias)
) STRICT, WITHOUT ROWID;
"#;

pub(crate) const CHAT_SCHEMA: &str = r#"
CREATE TABLE chat_inbox_state (
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    uid BLOB NOT NULL CHECK(length(uid)=33),
    app_id INTEGER NOT NULL CHECK(app_id=1),
    cursor INTEGER NOT NULL CHECK(cursor>=0),
    head INTEGER NOT NULL CHECK(head>=cursor),
    degraded INTEGER NOT NULL CHECK(degraded IN (0,1)),
    PRIMARY KEY(host_id,uid,app_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE chat_inbox_channels (
    host_id BLOB NOT NULL,
    uid BLOB NOT NULL,
    app_id INTEGER NOT NULL,
    channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
    team_id BLOB NOT NULL CHECK(length(team_id)=33),
    inbox_version INTEGER NOT NULL CHECK(inbox_version>0),
    read_through INTEGER NOT NULL CHECK(read_through>=0),
    pending_read INTEGER CHECK(pending_read IS NULL OR pending_read>0),
    hidden INTEGER NOT NULL CHECK(hidden IN (0,1)),
    muted INTEGER NOT NULL CHECK(muted IN (0,1)),
    metadata BLOB NOT NULL CHECK(length(metadata) BETWEEN 1 AND 32768),
    PRIMARY KEY(host_id,uid,app_id,channel_id),
    UNIQUE(host_id,uid,app_id,inbox_version),
    FOREIGN KEY(host_id,uid,app_id) REFERENCES chat_inbox_state(host_id,uid,app_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX chat_inbox_team ON chat_inbox_channels(host_id,uid,app_id,team_id,inbox_version DESC);
"#;

pub(crate) const KV_SCHEMA: &str = r#"
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
    readable INTEGER NOT NULL CHECK (readable IN (0, 1)),
    PRIMARY KEY (host_id, party_id, parent_dir_id, dirent_id),
    UNIQUE (host_id, party_id, parent_dir_id, name),
    FOREIGN KEY (host_id, party_id, parent_dir_id)
        REFERENCES kv_directories(host_id, party_id, dir_id) ON DELETE CASCADE,
    FOREIGN KEY (large_file_id) REFERENCES kv_large_files(id)
) STRICT, WITHOUT ROWID;

-- Ciphertext-only rollback anchors survive permission-driven cache pruning.
CREATE TABLE kv_directory_history (
    host_id BLOB NOT NULL,
    party_id BLOB NOT NULL,
    dir_id BLOB NOT NULL CHECK (length(dir_id) = 16),
    version INTEGER NOT NULL CHECK (version > 0),
    dir_bytes BLOB NOT NULL CHECK (length(dir_bytes) > 0),
    PRIMARY KEY (host_id, party_id, dir_id),
    FOREIGN KEY (host_id, party_id) REFERENCES kv_parties(host_id, party_id) ON DELETE CASCADE
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
    FOREIGN KEY (host_id, party_id) REFERENCES kv_parties(host_id, party_id) ON DELETE CASCADE
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
