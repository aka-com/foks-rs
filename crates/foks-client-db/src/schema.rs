pub(crate) const APPLICATION_ID: i64 = 0x464f_4b53; // `FOKS`
pub(crate) const VERSION: u32 = 6;

pub(crate) const INITIAL: &str = r#"
CREATE TABLE hosts (
    host_id BLOB PRIMARY KEY CHECK (length(host_id) > 0),
    canonical_name TEXT NOT NULL CHECK (length(canonical_name) > 0),
    genesis_key BLOB NOT NULL CHECK (length(genesis_key) > 0),
    chain_seqno INTEGER NOT NULL CHECK (chain_seqno >= 0),
    chain_tail_hash BLOB NOT NULL CHECK (length(chain_tail_hash) = 32),
    chain_bytes BLOB NOT NULL CHECK (length(chain_bytes) > 0),
    public_zone_bytes BLOB NOT NULL CHECK (length(public_zone_bytes) > 0)
) STRICT, WITHOUT ROWID;

CREATE TABLE host_lookups (
    lookup_name TEXT PRIMARY KEY CHECK (length(lookup_name) > 0),
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE RESTRICT
) STRICT, WITHOUT ROWID;

CREATE TABLE host_services (
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE CASCADE,
    service_type INTEGER NOT NULL CHECK (service_type >= 0),
    endpoint_bytes BLOB NOT NULL CHECK (length(endpoint_bytes) > 0),
    valid_at_chain_seqno INTEGER NOT NULL CHECK (valid_at_chain_seqno >= 0),
    PRIMARY KEY (host_id, service_type)
) STRICT, WITHOUT ROWID;

CREATE TABLE merkle_roots (
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    root_hash BLOB NOT NULL CHECK (length(root_hash) = 32),
    root_bytes BLOB CHECK (root_bytes IS NULL OR length(root_bytes) > 0),
    PRIMARY KEY (host_id, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE merkle_heads (
    host_id BLOB PRIMARY KEY REFERENCES hosts(host_id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    root_hash BLOB NOT NULL CHECK (length(root_hash) = 32),
    evidence_kind INTEGER NOT NULL CHECK (evidence_kind IN (1, 2)),
    anchor_epoch INTEGER CHECK (anchor_epoch IS NULL OR anchor_epoch >= 0),
    evidence_bytes BLOB NOT NULL CHECK (length(evidence_bytes) > 0),
    FOREIGN KEY (host_id, epoch) REFERENCES merkle_roots(host_id, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE users (
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE CASCADE,
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    chain_seqno INTEGER NOT NULL CHECK (chain_seqno > 0),
    chain_tail_hash BLOB NOT NULL CHECK (length(chain_tail_hash) = 32),
    chain_bytes BLOB NOT NULL CHECK (length(chain_bytes) > 0),
    evidence_bytes BLOB NOT NULL CHECK (length(evidence_bytes) > 0),
    username BLOB NOT NULL CHECK (length(username) BETWEEN 3 AND 25),
    username_utf8 BLOB NOT NULL CHECK (length(username_utf8) > 0),
    username_sequence INTEGER NOT NULL CHECK (username_sequence > 0),
    merkle_epoch INTEGER NOT NULL CHECK (merkle_epoch >= 0),
    merkle_root_hash BLOB NOT NULL CHECK (length(merkle_root_hash) = 32),
    merkle_root_bytes BLOB NOT NULL CHECK (length(merkle_root_bytes) > 0),
    PRIMARY KEY (host_id, uid),
    FOREIGN KEY (host_id, merkle_epoch) REFERENCES merkle_roots(host_id, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE user_devices (
    host_id BLOB NOT NULL,
    uid BLOB NOT NULL,
    device_id BLOB NOT NULL CHECK (length(device_id) IN (33, 34)),
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    role_visibility INTEGER NOT NULL CHECK (
        (role_type = 1 AND role_visibility BETWEEN -32768 AND 32767) OR
        (role_type IN (2, 3) AND role_visibility = 0)
    ),
    hepk_bytes BLOB NOT NULL CHECK (length(hepk_bytes) > 0),
    subkey_id BLOB CHECK (subkey_id IS NULL OR length(subkey_id) = 33),
    PRIMARY KEY (host_id, uid, device_id),
    FOREIGN KEY (host_id, uid) REFERENCES users(host_id, uid) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE user_shared_keys (
    host_id BLOB NOT NULL,
    uid BLOB NOT NULL,
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    role_visibility INTEGER NOT NULL CHECK (
        (role_type = 1 AND role_visibility BETWEEN -32768 AND 32767) OR
        (role_type IN (2, 3) AND role_visibility = 0)
    ),
    generation INTEGER NOT NULL CHECK (generation > 0),
    verify_key BLOB NOT NULL CHECK (length(verify_key) = 33),
    hepk_bytes BLOB NOT NULL CHECK (length(hepk_bytes) > 0),
    PRIMARY KEY (host_id, uid, role_type, role_visibility, generation),
    FOREIGN KEY (host_id, uid) REFERENCES users(host_id, uid) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
"#;
