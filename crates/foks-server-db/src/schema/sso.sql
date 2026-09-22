-- Provider policy survives disabling configuration; linked users never become device-only.
CREATE TABLE sso_policy (
    host BLOB PRIMARY KEY CHECK(length(host) = 33),
    config_hash BLOB NOT NULL CHECK(length(config_hash) = 32),
    rollout_id BLOB NOT NULL CHECK(length(rollout_id)=16),
    issuer TEXT NOT NULL CHECK(length(issuer) BETWEEN 1 AND 4096),
    mode INTEGER NOT NULL CHECK(mode IN (0,1)),
    blocked_reason INTEGER CHECK(blocked_reason BETWEEN 1 AND 4),
    revision INTEGER NOT NULL CHECK(revision>0),
    authorization_epoch INTEGER NOT NULL CHECK(authorization_epoch>0),
    UNIQUE(host,issuer)
) STRICT;
CREATE UNIQUE INDEX sso_single_host_policy ON sso_policy((1));
CREATE TABLE sso_migration_cohort (
    host BLOB NOT NULL REFERENCES sso_policy(host),
    uid BLOB NOT NULL REFERENCES users(uid),
    PRIMARY KEY(host,uid)
) STRICT;
CREATE TRIGGER sso_cohort_immutable_update BEFORE UPDATE ON sso_migration_cohort
BEGIN SELECT RAISE(ABORT,'immutable SSO migration cohort'); END;
CREATE TRIGGER sso_cohort_immutable_delete BEFORE DELETE ON sso_migration_cohort
BEGIN SELECT RAISE(ABORT,'immutable SSO migration cohort'); END;
-- IDs and admission identities are one-way digests. All browser/token material is encrypted.
CREATE TABLE sso_sessions (
    host BLOB NOT NULL REFERENCES sso_policy(host),
    session_hash BLOB NOT NULL CHECK(length(session_hash) = 32),
    config_hash BLOB NOT NULL CHECK(length(config_hash) = 32),
    source_hash BLOB NOT NULL CHECK(length(source_hash) = 32),
    uid BLOB CHECK(uid IS NULL OR length(uid) = 33),
    state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 7),
    revision INTEGER NOT NULL CHECK(revision > 0),
    authorization_epoch INTEGER NOT NULL CHECK(authorization_epoch>0),
    interrupted INTEGER NOT NULL DEFAULT 0 CHECK(interrupted IN (0,1)),
    expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > 0),
    ciphertext BLOB NOT NULL CHECK(length(ciphertext) BETWEEN 57 AND 2097152),
    PRIMARY KEY(host, session_hash)
) STRICT;
CREATE INDEX sso_session_expiry ON sso_sessions(host, expires_at_ms);
CREATE INDEX sso_session_source ON sso_sessions(host, source_hash, expires_at_ms);
-- Persistent identity linkage is distinct from expiring browser flows.
CREATE TABLE sso_access (
    host BLOB NOT NULL REFERENCES sso_policy(host),
    uid BLOB NOT NULL REFERENCES users(uid),
    issuer TEXT NOT NULL CHECK(length(issuer) BETWEEN 1 AND 4096),
    subject TEXT NOT NULL CHECK(length(subject) BETWEEN 1 AND 255),
    config_hash BLOB NOT NULL CHECK(length(config_hash)=32),
    revision INTEGER NOT NULL CHECK(revision>0),
    authorization_epoch INTEGER NOT NULL CHECK(authorization_epoch>0),
    authorization_generation INTEGER NOT NULL CHECK(authorization_generation>0),
    interrupted INTEGER NOT NULL DEFAULT 0 CHECK(interrupted IN (0,1)),
    state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 3),
    expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>0),
    ciphertext BLOB NOT NULL CHECK(length(ciphertext) BETWEEN 57 AND 2097152),
    PRIMARY KEY(host,uid), UNIQUE(host,issuer,subject),
    FOREIGN KEY(host,issuer) REFERENCES sso_policy(host,issuer)
) STRICT;

-- Provider-independent, one-use account proof challenges. No claimed-UID admission quota.
CREATE TABLE sso_identity_challenges (
    challenge BLOB PRIMARY KEY CHECK(length(challenge)=32),
    claim_hash BLOB NOT NULL CHECK(length(claim_hash)=32),
    source_hash BLOB NOT NULL CHECK(length(source_hash)=32),
    expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>0)
) STRICT;
CREATE INDEX sso_identity_challenge_expiry ON sso_identity_challenges(expires_at_ms);
CREATE INDEX sso_identity_challenge_source ON sso_identity_challenges(source_hash);
CREATE TABLE sso_binding_receipts (
    host BLOB NOT NULL REFERENCES sso_policy(host),
    uid BLOB NOT NULL REFERENCES users(uid),
    commitment BLOB NOT NULL CHECK(length(commitment)=32),
    purpose INTEGER NOT NULL CHECK(purpose BETWEEN 0 AND 2),
    authorization_epoch INTEGER NOT NULL CHECK(authorization_epoch>0),
    authorization_generation INTEGER NOT NULL CHECK(authorization_generation>0),
    expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>0),
    PRIMARY KEY(host,uid,commitment)
) STRICT;
CREATE INDEX sso_binding_receipt_expiry ON sso_binding_receipts(host,uid,expires_at_ms);
