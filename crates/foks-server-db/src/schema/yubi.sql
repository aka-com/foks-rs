CREATE TABLE yubi_subkey_boxes (
    parent_id BLOB PRIMARY KEY REFERENCES devices(device_id) ON DELETE CASCADE
        CHECK (length(parent_id) = 34),
    subkey_id BLOB NOT NULL UNIQUE CHECK (length(subkey_id) = 33),
    exact_box BLOB NOT NULL,
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE TABLE yubi_pq_hints (
    parent_id BLOB PRIMARY KEY REFERENCES devices(device_id) ON DELETE CASCADE
        CHECK (length(parent_id) = 34),
    slot INTEGER NOT NULL CHECK (slot BETWEEN 130 AND 149),
    pq_key_id BLOB NOT NULL CHECK (length(pq_key_id) = 32)
) STRICT;

CREATE TABLE yubi_management_keys (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    parent_id BLOB NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE
        CHECK (length(parent_id) = 34),
    exact_box BLOB NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    visibility INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    PRIMARY KEY (uid, parent_id)
) STRICT;
