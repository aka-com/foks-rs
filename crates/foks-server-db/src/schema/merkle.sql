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
    CHECK (epoch != 0 OR root_node = zeroblob(32))
) STRICT;

CREATE TRIGGER merkle_roots_require_node
BEFORE INSERT ON merkle_roots
WHEN NEW.epoch != 0
  AND NOT EXISTS (SELECT 1 FROM merkle_nodes WHERE node_hash = NEW.root_node)
BEGIN
    SELECT RAISE(ABORT, 'missing Merkle root node');
END;

CREATE TRIGGER merkle_roots_require_node_on_update
BEFORE UPDATE OF epoch, root_node ON merkle_roots
WHEN NEW.epoch != 0
  AND NOT EXISTS (SELECT 1 FROM merkle_nodes WHERE node_hash = NEW.root_node)
BEGIN
    SELECT RAISE(ABORT, 'missing Merkle root node');
END;

CREATE TRIGGER merkle_nodes_restrict_delete
BEFORE DELETE ON merkle_nodes
WHEN EXISTS (SELECT 1 FROM merkle_roots WHERE epoch != 0 AND root_node = OLD.node_hash)
BEGIN
    SELECT RAISE(ABORT, 'Merkle node is referenced by a root');
END;

CREATE TRIGGER merkle_nodes_restrict_hash_update
BEFORE UPDATE OF node_hash ON merkle_nodes
WHEN EXISTS (SELECT 1 FROM merkle_roots WHERE epoch != 0 AND root_node = OLD.node_hash)
BEGIN
    SELECT RAISE(ABORT, 'Merkle node is referenced by a root');
END;

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
