CREATE TABLE subchain_tree_location_seeds (
    entity_id BLOB PRIMARY KEY CHECK (length(entity_id) = 33),
    seed BLOB NOT NULL CHECK (length(seed) = 32)
) STRICT;

CREATE TABLE generic_chain_links (
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 33),
    chain_type INTEGER NOT NULL CHECK (chain_type IN (2, 4)),
    seqno INTEGER NOT NULL CHECK (seqno >= 1),
    link_hash BLOB NOT NULL UNIQUE CHECK (length(link_hash) = 32),
    exact_link BLOB NOT NULL,
    root_epoch INTEGER NOT NULL CHECK (root_epoch >= 1),
    PRIMARY KEY (entity_id, chain_type, seqno),
    UNIQUE (entity_id, chain_type, seqno, link_hash)
) STRICT;

CREATE TABLE generic_chain_heads (
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 33),
    chain_type INTEGER NOT NULL CHECK (chain_type IN (2, 4)),
    seqno INTEGER NOT NULL CHECK (seqno >= 1),
    link_hash BLOB NOT NULL CHECK (length(link_hash) = 32),
    PRIMARY KEY (entity_id, chain_type),
    FOREIGN KEY (entity_id, chain_type, seqno, link_hash)
        REFERENCES generic_chain_links(entity_id, chain_type, seqno, link_hash)
) STRICT;

-- A link at sequence N discloses the hidden location for N+1.
CREATE TABLE generic_tree_locations (
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 33),
    chain_type INTEGER NOT NULL CHECK (chain_type IN (2, 4)),
    seqno INTEGER NOT NULL CHECK (seqno >= 2),
    location BLOB NOT NULL UNIQUE CHECK (length(location) = 32),
    PRIMARY KEY (entity_id, chain_type, seqno)
) STRICT;
