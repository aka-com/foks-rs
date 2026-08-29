CREATE TABLE federation_user_view_permissions (
    target_user_id BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    viewer_party_id BLOB NOT NULL CHECK (length(viewer_party_id) = 33),
    viewer_host_id BLOB NOT NULL CHECK (length(viewer_host_id) = 33),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    token_nonce BLOB NOT NULL CHECK (length(token_nonce) = 24),
    token_ciphertext BLOB NOT NULL CHECK (length(token_ciphertext) = 33),
    key_generation BLOB NOT NULL REFERENCES capability_key_generations(generation_id)
        CHECK (length(key_generation) = 16),
    state INTEGER NOT NULL CHECK (state IN (0, 1)),
    issued_at INTEGER NOT NULL CHECK (issued_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= issued_at),
    expires_at INTEGER NOT NULL CHECK (expires_at > issued_at),
    revoked_at INTEGER CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
    PRIMARY KEY (target_user_id, viewer_party_id, viewer_host_id),
    CHECK ((state = 1 AND revoked_at IS NULL) OR (state = 0 AND revoked_at IS NOT NULL))
) STRICT, WITHOUT ROWID;

CREATE INDEX federation_user_view_permissions_active
    ON federation_user_view_permissions(target_user_id, state, expires_at);

CREATE TABLE federation_team_view_permissions (
    target_team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    viewer_party_id BLOB NOT NULL CHECK (length(viewer_party_id) = 33),
    viewer_host_id BLOB NOT NULL CHECK (length(viewer_host_id) = 33),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    token_nonce BLOB NOT NULL CHECK (length(token_nonce) = 24),
    token_ciphertext BLOB NOT NULL CHECK (length(token_ciphertext) = 33),
    key_generation BLOB NOT NULL REFERENCES capability_key_generations(generation_id)
        CHECK (length(key_generation) = 16),
    grantor_party_id BLOB NOT NULL CHECK (length(grantor_party_id) = 33),
    state INTEGER NOT NULL CHECK (state IN (0, 1)),
    issued_at INTEGER NOT NULL CHECK (issued_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= issued_at),
    expires_at INTEGER NOT NULL CHECK (expires_at > issued_at),
    revoked_at INTEGER CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
    PRIMARY KEY (target_team_id, viewer_party_id, viewer_host_id),
    CHECK ((state = 1 AND revoked_at IS NULL) OR (state = 0 AND revoked_at IS NOT NULL))
) STRICT, WITHOUT ROWID;

CREATE INDEX federation_team_view_permissions_active
    ON federation_team_view_permissions(target_team_id, state, expires_at);

CREATE TABLE team_remote_member_view_tokens (
    target_team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    member_party_id BLOB NOT NULL CHECK (length(member_party_id) = 33),
    member_host_id BLOB NOT NULL CHECK (length(member_host_id) = 33),
    ptk_generation INTEGER NOT NULL CHECK (ptk_generation >= 1),
    ptk_role_type INTEGER NOT NULL CHECK (ptk_role_type BETWEEN 1 AND 3),
    ptk_visibility INTEGER NOT NULL,
    exact_secret_box BLOB NOT NULL,
    join_request_token BLOB NOT NULL CHECK (length(join_request_token) = 17),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    PRIMARY KEY (target_team_id, member_party_id, member_host_id),
    FOREIGN KEY (target_team_id, ptk_role_type, ptk_visibility, ptk_generation)
        REFERENCES team_shared_keys(team_id, role_type, visibility, generation)
) STRICT, WITHOUT ROWID;
