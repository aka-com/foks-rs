CREATE TABLE recovery_challenges (
    challenge_hash BLOB PRIMARY KEY CHECK (length(challenge_hash) = 32),
    entity_id BLOB NOT NULL CHECK (length(entity_id) IN (33, 34)),
    host_id BLOB NOT NULL CHECK (length(host_id) = 33),
    key_generation BLOB NOT NULL CHECK (length(key_generation) = 16),
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
    consumed INTEGER NOT NULL DEFAULT 0 CHECK (consumed IN (0, 1))
) STRICT;

CREATE INDEX recovery_challenges_entity
    ON recovery_challenges(entity_id, expires_at);

CREATE TABLE team_view_challenges (
    challenge_hash BLOB PRIMARY KEY CHECK (length(challenge_hash) = 32),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    -- A local team can act as a roster member with one of its current PTKs.
    -- `member_id` is therefore a PartyID, not necessarily a UID. Its live
    -- user/team key binding is checked when the challenge is activated.
    member_id BLOB NOT NULL CHECK (length(member_id) = 33),
    member_host_id BLOB NOT NULL CHECK (length(member_host_id) = 33),
    source_role_type INTEGER NOT NULL CHECK (source_role_type BETWEEN 1 AND 3),
    source_visibility INTEGER NOT NULL,
    source_generation INTEGER NOT NULL CHECK (source_generation >= 1),
    key_generation BLOB NOT NULL REFERENCES capability_key_generations(generation_id)
        CHECK (length(key_generation) = 16),
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
    consumed INTEGER NOT NULL DEFAULT 0 CHECK (consumed IN (0, 1)),
    activation_hash BLOB CHECK (activation_hash IS NULL OR length(activation_hash) = 32),
    CHECK ((consumed = 0 AND activation_hash IS NULL)
        OR (consumed = 1 AND activation_hash IS NOT NULL))
) STRICT;

CREATE INDEX team_view_challenges_scope
    ON team_view_challenges(team_id, member_id, expires_at);

CREATE TABLE team_view_tokens (
    token_hash BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    member_id BLOB NOT NULL CHECK (length(member_id) = 33),
    member_host_id BLOB NOT NULL CHECK (length(member_host_id) = 33),
    source_role_type INTEGER NOT NULL CHECK (source_role_type BETWEEN 1 AND 3),
    source_visibility INTEGER NOT NULL,
    source_generation INTEGER NOT NULL CHECK (source_generation >= 1),
    effective_role_type INTEGER NOT NULL CHECK (effective_role_type BETWEEN 1 AND 3),
    effective_visibility INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0)
) STRICT;

CREATE INDEX team_view_tokens_scope
    ON team_view_tokens(team_id, member_id, expires_at);

CREATE TABLE team_admin_tokens (
    token_hash BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    team_id BLOB NOT NULL REFERENCES teams(team_id) ON DELETE CASCADE,
    -- TeamAdmin tokens are capabilities held by an authenticated user. The
    -- holder need not be a direct member: activation proves possession of the
    -- target team's current admin/owner PTK.
    holder_id BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    ptk_role_type INTEGER NOT NULL CHECK (ptk_role_type BETWEEN 2 AND 3),
    ptk_visibility INTEGER NOT NULL DEFAULT 0 CHECK (ptk_visibility = 0),
    ptk_generation INTEGER NOT NULL CHECK (ptk_generation >= 1),
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
    activation_hash BLOB CHECK (activation_hash IS NULL OR length(activation_hash) = 32)
) STRICT;

CREATE INDEX team_admin_tokens_scope
    ON team_admin_tokens(team_id, holder_id, expires_at);
