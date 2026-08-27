CREATE TABLE names (
    normalized_name BLOB PRIMARY KEY CHECK (length(normalized_name) BETWEEN 1 AND 255),
    reservation_token BLOB UNIQUE CHECK (reservation_token IS NULL OR length(reservation_token) = 17),
    reservation_sequence INTEGER NOT NULL CHECK (reservation_sequence >= 1),
    expires_at INTEGER CHECK (expires_at IS NULL OR expires_at >= 0),
    uid BLOB UNIQUE CHECK (uid IS NULL OR length(uid) = 33),
    UNIQUE (normalized_name, uid),
    CHECK ((uid IS NULL AND reservation_token IS NOT NULL AND expires_at IS NOT NULL)
        OR (uid IS NOT NULL AND reservation_token IS NULL AND expires_at IS NULL))
) STRICT;

CREATE TABLE users (
    uid BLOB PRIMARY KEY CHECK (length(uid) = 33),
    normalized_name BLOB NOT NULL UNIQUE,
    username_utf8 BLOB NOT NULL,
    username_sequence INTEGER NOT NULL CHECK (username_sequence >= 1),
    username_commitment_key BLOB NOT NULL CHECK (length(username_commitment_key) = 16),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    FOREIGN KEY (normalized_name, uid) REFERENCES names(normalized_name, uid)
) STRICT;

CREATE TABLE devices (
    device_id BLOB PRIMARY KEY CHECK (length(device_id) IN (33, 34)),
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    active INTEGER NOT NULL CHECK (active IN (0, 1)),
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    visibility INTEGER NOT NULL,
    subkey_id BLOB CHECK (subkey_id IS NULL OR length(subkey_id) IN (33, 34)),
    hepk_fingerprint BLOB NOT NULL CHECK (length(hepk_fingerprint) = 32),
    exact_hepk BLOB NOT NULL,
    exact_name BLOB NOT NULL
) STRICT;

CREATE TABLE user_chain_links (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    seqno INTEGER NOT NULL CHECK (seqno >= 1),
    link_hash BLOB NOT NULL UNIQUE CHECK (length(link_hash) = 32),
    exact_link BLOB NOT NULL,
    root_epoch INTEGER NOT NULL CHECK (root_epoch >= 1),
    PRIMARY KEY (uid, seqno),
    UNIQUE (uid, seqno, link_hash)
) STRICT;

CREATE TABLE user_chain_heads (
    uid BLOB PRIMARY KEY REFERENCES users(uid) ON DELETE CASCADE,
    seqno INTEGER NOT NULL CHECK (seqno >= 1),
    link_hash BLOB NOT NULL CHECK (length(link_hash) = 32),
    FOREIGN KEY (uid, seqno, link_hash) REFERENCES user_chain_links(uid, seqno, link_hash)
) STRICT;

CREATE TABLE tree_locations (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    seqno INTEGER NOT NULL CHECK (seqno >= 1),
    location BLOB NOT NULL UNIQUE CHECK (length(location) = 32),
    PRIMARY KEY (uid, seqno)
) STRICT;

CREATE TABLE shared_keys (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 0 AND 3),
    visibility INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    verify_key BLOB NOT NULL CHECK (length(verify_key) IN (33, 34)),
    exact_hepk BLOB NOT NULL,
    PRIMARY KEY (uid, role_type, visibility, generation)
) STRICT;

CREATE TABLE parcels (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    device_id BLOB NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    sender_id BLOB NOT NULL REFERENCES devices(device_id),
    role_type INTEGER NOT NULL,
    visibility INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    exact_parcel BLOB NOT NULL,
    PRIMARY KEY (uid, device_id, role_type, visibility, generation),
    FOREIGN KEY (uid, role_type, visibility, generation)
        REFERENCES shared_keys(uid, role_type, visibility, generation)
) STRICT;

CREATE TABLE seed_chain_boxes (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 0 AND 3),
    visibility INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    exact_box BLOB NOT NULL,
    PRIMARY KEY (uid, role_type, visibility, generation)
) STRICT;
