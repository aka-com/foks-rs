CREATE TABLE team_names (
    normalized_name BLOB PRIMARY KEY CHECK (length(normalized_name) BETWEEN 1 AND 255),
    reservation_token BLOB UNIQUE CHECK (reservation_token IS NULL OR length(reservation_token) = 17),
    reservation_sequence INTEGER NOT NULL CHECK (reservation_sequence >= 1),
    expires_at INTEGER CHECK (expires_at IS NULL OR expires_at >= 0),
    team_id BLOB UNIQUE CHECK (team_id IS NULL OR length(team_id) = 33),
    UNIQUE (normalized_name, team_id),
    CHECK ((team_id IS NULL AND reservation_token IS NOT NULL AND expires_at IS NOT NULL)
        OR (team_id IS NOT NULL AND reservation_token IS NULL AND expires_at IS NULL))
) STRICT;

CREATE TABLE teams (
    team_id BLOB PRIMARY KEY CHECK (length(team_id) = 33),
    team_kind INTEGER NOT NULL CHECK (team_kind IN (3, 20)),
    host_id BLOB NOT NULL CHECK (length(host_id) = 33),
    normalized_name BLOB UNIQUE,
    team_name_utf8 BLOB NOT NULL,
    team_name_sequence INTEGER NOT NULL CHECK (team_name_sequence >= 0),
    team_name_commitment_key BLOB CHECK (team_name_commitment_key IS NULL OR length(team_name_commitment_key) = 16),
    member_load_floor_type INTEGER NOT NULL CHECK (member_load_floor_type BETWEEN 1 AND 3),
    member_load_floor_visibility INTEGER NOT NULL,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK ((team_kind = 3 AND normalized_name IS NOT NULL AND team_name_sequence >= 1
            AND team_name_commitment_key IS NOT NULL)
        OR (team_kind = 20 AND normalized_name IS NULL AND team_name_sequence = 0
            AND team_name_commitment_key IS NULL)),
    FOREIGN KEY (normalized_name, team_id) REFERENCES team_names(normalized_name, team_id)
) STRICT;

CREATE TABLE team_local_view_permissions (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    target_id BLOB NOT NULL CHECK (length(target_id) = 33),
    minimum_role_type INTEGER NOT NULL CHECK (minimum_role_type BETWEEN 1 AND 3),
    minimum_role_visibility INTEGER NOT NULL,
    PRIMARY KEY (team_id, target_id)
) STRICT;

CREATE TABLE team_chain_links (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    seqno INTEGER NOT NULL CHECK (seqno >= 1),
    link_hash BLOB NOT NULL UNIQUE CHECK (length(link_hash) = 32),
    exact_link BLOB NOT NULL,
    root_epoch INTEGER NOT NULL CHECK (root_epoch >= 1),
    PRIMARY KEY (team_id, seqno),
    UNIQUE (team_id, seqno, link_hash)
) STRICT;

CREATE TABLE team_chain_heads (
    team_id BLOB PRIMARY KEY REFERENCES teams(team_id) ON DELETE CASCADE,
    seqno INTEGER NOT NULL CHECK (seqno >= 1),
    link_hash BLOB NOT NULL CHECK (length(link_hash) = 32),
    FOREIGN KEY (team_id, seqno, link_hash)
        REFERENCES team_chain_links(team_id, seqno, link_hash)
) STRICT;

CREATE TABLE team_tree_locations (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    seqno INTEGER NOT NULL CHECK (seqno >= 1),
    location BLOB NOT NULL UNIQUE CHECK (length(location) = 32),
    PRIMARY KEY (team_id, seqno)
) STRICT;

CREATE TABLE team_members (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    party_id BLOB NOT NULL CHECK (length(party_id) = 33),
    scoped_host_id BLOB CHECK (scoped_host_id IS NULL OR length(scoped_host_id) = 33),
    source_role_type INTEGER NOT NULL CHECK (source_role_type BETWEEN 1 AND 3),
    source_visibility INTEGER NOT NULL,
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    visibility INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    verify_key BLOB NOT NULL CHECK (length(verify_key) IN (33, 34)),
    hepk_fingerprint BLOB NOT NULL CHECK (length(hepk_fingerprint) = 32),
    removal_key_commitment BLOB CHECK (removal_key_commitment IS NULL OR length(removal_key_commitment) = 32),
    PRIMARY KEY (team_id, party_id, source_role_type, source_visibility)
) STRICT;

CREATE TABLE team_shared_keys (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    visibility INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    verify_key BLOB NOT NULL CHECK (length(verify_key) IN (33, 34)),
    exact_hepk BLOB NOT NULL,
    start_epoch INTEGER NOT NULL DEFAULT 1 CHECK (start_epoch >= 1),
    PRIMARY KEY (team_id, role_type, visibility, generation)
) STRICT;

CREATE TABLE team_parcels (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    party_id BLOB NOT NULL CHECK (length(party_id) = 33),
    sender_id BLOB NOT NULL CHECK (length(sender_id) IN (33, 34)),
    target_role_type INTEGER NOT NULL CHECK (target_role_type BETWEEN 1 AND 3),
    target_visibility INTEGER NOT NULL,
    role_type INTEGER NOT NULL,
    visibility INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    exact_parcel BLOB NOT NULL,
    PRIMARY KEY (
        team_id, party_id, target_role_type, target_visibility,
        role_type, visibility, generation
    ),
    FOREIGN KEY (team_id, role_type, visibility, generation)
        REFERENCES team_shared_keys(team_id, role_type, visibility, generation)
) STRICT;

CREATE TABLE team_seed_chain_boxes (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    role_type INTEGER NOT NULL,
    visibility INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    exact_box BLOB NOT NULL,
    PRIMARY KEY (team_id, role_type, visibility, generation)
) STRICT;

CREATE TABLE team_removal_boxes (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    member_id BLOB NOT NULL CHECK (length(member_id) = 33),
    member_host_id BLOB NOT NULL CHECK (length(member_host_id) = 33),
    source_role_type INTEGER NOT NULL CHECK (source_role_type BETWEEN 1 AND 3),
    source_visibility INTEGER NOT NULL,
    exact_box BLOB NOT NULL,
    PRIMARY KEY (team_id, member_id, member_host_id, source_role_type, source_visibility)
) STRICT;

CREATE TABLE team_removal_proofs (
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    commitment BLOB NOT NULL CHECK (length(commitment) = 32),
    member_id BLOB NOT NULL CHECK (length(member_id) = 33),
    member_host_id BLOB NOT NULL CHECK (length(member_host_id) = 33),
    source_role_type INTEGER NOT NULL CHECK (source_role_type BETWEEN 1 AND 3),
    source_visibility INTEGER NOT NULL,
    exact_removal BLOB NOT NULL,
    PRIMARY KEY (team_id, commitment),
    FOREIGN KEY (team_id, member_id, member_host_id, source_role_type, source_visibility)
        REFERENCES team_removal_boxes(
            team_id, member_id, member_host_id, source_role_type, source_visibility
        )
) STRICT;
