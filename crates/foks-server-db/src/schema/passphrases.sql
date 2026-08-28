CREATE TABLE passphrase_salts (
    uid BLOB PRIMARY KEY REFERENCES users(uid) ON DELETE CASCADE,
    salt BLOB NOT NULL CHECK (length(salt) = 16),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE TABLE passphrase_boxes (
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    verify_key BLOB NOT NULL CHECK (length(verify_key) = 33),
    exact_skmwk_box BLOB NOT NULL,
    exact_passphrase_box BLOB NOT NULL,
    exact_puk_box BLOB,
    puk_generation INTEGER CHECK (
        (exact_puk_box IS NULL AND puk_generation IS NULL)
        OR (exact_puk_box IS NOT NULL AND puk_generation >= 1)
    ),
    stretch_version INTEGER NOT NULL CHECK (stretch_version = 1),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    PRIMARY KEY (uid, generation)
) STRICT;

CREATE INDEX passphrase_boxes_latest ON passphrase_boxes(uid, generation DESC);

CREATE TABLE passphrase_login_challenges (
    challenge_hash BLOB PRIMARY KEY CHECK (length(challenge_hash) = 32),
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    host_id BLOB NOT NULL CHECK (length(host_id) = 33),
    key_generation BLOB NOT NULL CHECK (length(key_generation) = 16),
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
    consumed INTEGER NOT NULL CHECK (consumed IN (0, 1))
) STRICT;

CREATE INDEX passphrase_login_challenges_uid
    ON passphrase_login_challenges(uid, expires_at);

CREATE TABLE bad_passphrase_attempts (
    id INTEGER PRIMARY KEY,
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    attempted_at INTEGER NOT NULL CHECK (attempted_at >= 0)
) STRICT;

CREATE INDEX bad_passphrase_attempts_uid
    ON bad_passphrase_attempts(uid, attempted_at);

