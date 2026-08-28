CREATE TABLE signup_policy (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    invite_regime INTEGER NOT NULL CHECK (invite_regime IN (1, 2))
) STRICT;

INSERT INTO signup_policy(singleton, invite_regime) VALUES (1, 2);

CREATE TABLE signup_invites (
    invite_id BLOB PRIMARY KEY CHECK (length(invite_id) = 16),
    code_hash BLOB NOT NULL UNIQUE CHECK (length(code_hash) = 32),
    kind INTEGER NOT NULL CHECK (kind IN (1, 2)),
    issuer_uid BLOB CHECK (issuer_uid IS NULL OR length(issuer_uid) = 33),
    state INTEGER NOT NULL CHECK (state IN (1, 2)),
    max_uses INTEGER CHECK (max_uses IS NULL OR max_uses >= 1),
    use_count INTEGER NOT NULL DEFAULT 0 CHECK (use_count >= 0),
    expires_at INTEGER CHECK (expires_at IS NULL OR expires_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    disabled_at INTEGER CHECK (disabled_at IS NULL OR disabled_at >= created_at),
    CHECK ((state = 1 AND disabled_at IS NULL) OR (state = 2 AND disabled_at IS NOT NULL)),
    CHECK ((kind = 1 AND max_uses = 1) OR kind = 2),
    CHECK (max_uses IS NULL OR use_count <= max_uses),
    FOREIGN KEY (issuer_uid) REFERENCES users(uid) ON DELETE RESTRICT
) STRICT;

CREATE INDEX signup_invites_available
    ON signup_invites(code_hash, kind, state, expires_at, use_count);

CREATE TABLE signup_invite_redemptions (
    invite_id BLOB NOT NULL REFERENCES signup_invites(invite_id) ON DELETE RESTRICT,
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE RESTRICT,
    redeemed_at INTEGER NOT NULL CHECK (redeemed_at >= 0),
    PRIMARY KEY (invite_id, uid),
    UNIQUE (uid)
) STRICT;
