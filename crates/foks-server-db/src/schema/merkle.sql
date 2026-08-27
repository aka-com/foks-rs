CREATE TABLE merkle_nodes (
    node_hash BLOB PRIMARY KEY CHECK (length(node_hash) = 32),
    exact_node BLOB NOT NULL
) STRICT;

CREATE TABLE merkle_leaves (
    leaf_key BLOB PRIMARY KEY CHECK (length(leaf_key) = 32),
    leaf_value BLOB NOT NULL CHECK (length(leaf_value) = 32)
) STRICT;

CREATE TABLE merkle_roots (
    epoch INTEGER PRIMARY KEY CHECK (epoch >= 0),
    root_hash BLOB NOT NULL UNIQUE CHECK (length(root_hash) = 32),
    root_node BLOB NOT NULL CHECK (length(root_node) = 32),
    exact_root BLOB NOT NULL,
    exact_signed_root BLOB NOT NULL,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    UNIQUE (epoch, root_hash),
    FOREIGN KEY (root_node) REFERENCES merkle_nodes(node_hash)
) STRICT;

CREATE TABLE merkle_root_heads (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    epoch INTEGER NOT NULL UNIQUE,
    root_hash BLOB NOT NULL UNIQUE,
    FOREIGN KEY (epoch, root_hash) REFERENCES merkle_roots(epoch, root_hash)
) STRICT;

CREATE TABLE merkle_back_pointers (
    root_epoch INTEGER NOT NULL REFERENCES merkle_roots(epoch) ON DELETE CASCADE,
    target_epoch INTEGER NOT NULL,
    target_hash BLOB NOT NULL CHECK (length(target_hash) = 32),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (root_epoch, ordinal),
    UNIQUE (root_epoch, target_epoch),
    CHECK (target_epoch < root_epoch),
    FOREIGN KEY (target_epoch, target_hash) REFERENCES merkle_roots(epoch, root_hash)
) STRICT;
