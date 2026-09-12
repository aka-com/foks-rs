-- Provider policy survives disabling configuration; linked users never become device-only.
CREATE TABLE sso_policy (
    host BLOB PRIMARY KEY CHECK(length(host) = 33),
    config_hash BLOB NOT NULL CHECK(length(config_hash) = 32),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1))
) STRICT;
-- IDs and admission identities are one-way digests. All browser/token material is encrypted.
CREATE TABLE sso_sessions (
    host BLOB NOT NULL REFERENCES sso_policy(host),
    session_hash BLOB NOT NULL CHECK(length(session_hash) = 32),
    config_hash BLOB NOT NULL CHECK(length(config_hash) = 32),
    admission_hash BLOB NOT NULL CHECK(length(admission_hash) = 32),
    uid BLOB CHECK(uid IS NULL OR length(uid) = 33),
    state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 7),
    revision INTEGER NOT NULL CHECK(revision > 0),
    expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > 0),
    ciphertext BLOB NOT NULL CHECK(length(ciphertext) BETWEEN 57 AND 2097152),
    PRIMARY KEY(host, session_hash)
) STRICT;
CREATE INDEX sso_session_expiry ON sso_sessions(host, expires_at_ms);
CREATE INDEX sso_session_admission ON sso_sessions(host, admission_hash, expires_at_ms);
-- Persistent identity linkage is distinct from expiring browser flows.
CREATE TABLE sso_access (
    host BLOB NOT NULL REFERENCES sso_policy(host),
    uid BLOB NOT NULL REFERENCES users(uid),
    issuer TEXT NOT NULL CHECK(length(issuer) BETWEEN 1 AND 4096),
    subject TEXT NOT NULL CHECK(length(subject) BETWEEN 1 AND 255),
    config_hash BLOB NOT NULL CHECK(length(config_hash)=32),
    revision INTEGER NOT NULL CHECK(revision>0),
    state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 3),
    expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>0),
    ciphertext BLOB NOT NULL CHECK(length(ciphertext) BETWEEN 57 AND 2097152),
    PRIMARY KEY(host,uid), UNIQUE(host,issuer,subject)
) STRICT;
