CREATE TABLE host_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    host_id BLOB NOT NULL CHECK (length(host_id) = 33),
    canonical_name TEXT NOT NULL CHECK (length(canonical_name) BETWEEN 1 AND 255),
    bootstrap_blob BLOB NOT NULL,
    key_manifest_blob BLOB NOT NULL
) STRICT;

CREATE TABLE hostchain_links (
    seqno INTEGER PRIMARY KEY CHECK (seqno >= 1),
    link_hash BLOB NOT NULL UNIQUE CHECK (length(link_hash) = 32),
    exact_link BLOB NOT NULL
) STRICT;

CREATE TABLE services (
    service_type INTEGER PRIMARY KEY CHECK (service_type IN (1, 2, 5, 10, 12, 16)),
    endpoint TEXT NOT NULL,
    advertised_blob BLOB NOT NULL
) STRICT;
