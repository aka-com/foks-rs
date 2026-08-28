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

CREATE TABLE host_key_generations (
    generation_id BLOB PRIMARY KEY CHECK (length(generation_id) = 16),
    purpose INTEGER NOT NULL CHECK (purpose = 1),
    encrypted_file_name TEXT NOT NULL UNIQUE
        CHECK (length(encrypted_file_name) BETWEEN 1 AND 96),
    public_entity_id BLOB NOT NULL UNIQUE CHECK (length(public_entity_id) = 33),
    state INTEGER NOT NULL CHECK (state IN (1, 2, 3, 4)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    activated_hostchain_seqno INTEGER CHECK (activated_hostchain_seqno >= 1),
    CHECK ((state = 1 AND activated_hostchain_seqno IS NULL)
        OR (state != 1 AND activated_hostchain_seqno IS NOT NULL))
) STRICT;

CREATE UNIQUE INDEX host_key_one_active
ON host_key_generations(purpose) WHERE state = 2;

CREATE TABLE host_rotation_operations (
    operation_id BLOB PRIMARY KEY CHECK (length(operation_id) = 16),
    purpose INTEGER NOT NULL CHECK (purpose = 1),
    old_generation_id BLOB NOT NULL REFERENCES host_key_generations(generation_id),
    new_generation_id BLOB NOT NULL REFERENCES host_key_generations(generation_id),
    phase INTEGER NOT NULL CHECK (phase IN (1, 2, 3)),
    add_link_seqno INTEGER CHECK (add_link_seqno >= 2),
    add_published_at INTEGER CHECK (add_published_at >= created_at),
    revoke_link_seqno INTEGER CHECK (revoke_link_seqno >= 3),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (old_generation_id != new_generation_id),
    CHECK ((phase = 1 AND add_link_seqno IS NULL AND add_published_at IS NULL
            AND revoke_link_seqno IS NULL)
        OR (phase = 2 AND add_link_seqno IS NOT NULL AND add_published_at IS NOT NULL
            AND revoke_link_seqno IS NULL)
        OR (phase = 3 AND add_link_seqno IS NOT NULL AND add_published_at IS NOT NULL
            AND revoke_link_seqno IS NOT NULL))
) STRICT;

CREATE UNIQUE INDEX host_rotation_one_in_progress
ON host_rotation_operations(purpose) WHERE phase != 3;

CREATE TABLE services (
    service_type INTEGER PRIMARY KEY CHECK (service_type IN (1, 2, 5, 10, 12, 16)),
    endpoint TEXT NOT NULL,
    advertised_blob BLOB NOT NULL
) STRICT;
