-- Host authority is checked against the singleton host_metadata in every transaction.
CREATE TABLE host_admin_grants (
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    uid BLOB NOT NULL REFERENCES users(uid),
    revision INTEGER NOT NULL CHECK(revision>0),
    active INTEGER NOT NULL CHECK(active IN (0,1)),
    created_at_us INTEGER NOT NULL CHECK(created_at_us>=0),
    changed_at_us INTEGER NOT NULL CHECK(changed_at_us>=created_at_us),
    reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 512),
    PRIMARY KEY(host_id,uid)
) STRICT;
CREATE TABLE web_login_tickets (
    ticket_hash BLOB PRIMARY KEY CHECK(length(ticket_hash)=32),
    record_id BLOB NOT NULL UNIQUE CHECK(length(record_id)=16),
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    uid BLOB NOT NULL REFERENCES users(uid),
    credential_id BLOB NOT NULL CHECK(length(credential_id)=33),
    certificate_expires_at_us INTEGER NOT NULL CHECK(certificate_expires_at_us>0),
    instance_epoch BLOB NOT NULL CHECK(length(instance_epoch)=16),
    sso_config_hash BLOB CHECK(sso_config_hash IS NULL OR length(sso_config_hash)=32),
    sso_policy_epoch INTEGER CHECK(sso_policy_epoch IS NULL OR sso_policy_epoch>0),
    sso_authorization_generation INTEGER CHECK(sso_authorization_generation IS NULL OR sso_authorization_generation>0),
    created_at_us INTEGER NOT NULL CHECK(created_at_us>=0),
    expires_at_us INTEGER NOT NULL CHECK(expires_at_us>created_at_us AND expires_at_us<=certificate_expires_at_us),
    deadline_elapsed_us INTEGER NOT NULL CHECK(deadline_elapsed_us>0),
    state INTEGER NOT NULL CHECK(state IN (0,1,2)),
    CHECK ((sso_config_hash IS NULL AND sso_policy_epoch IS NULL AND sso_authorization_generation IS NULL)
        OR (sso_config_hash IS NOT NULL AND sso_policy_epoch IS NOT NULL AND sso_authorization_generation IS NOT NULL))
) STRICT;
CREATE INDEX web_ticket_owner ON web_login_tickets(uid,state,expires_at_us);
CREATE INDEX web_ticket_deadline ON web_login_tickets(instance_epoch,deadline_elapsed_us);
CREATE TABLE web_login_confirmations (
    binding_hash BLOB PRIMARY KEY CHECK(length(binding_hash)=32),
    csrf_hash BLOB NOT NULL CHECK(length(csrf_hash)=32),
    ticket_hash BLOB NOT NULL REFERENCES web_login_tickets(ticket_hash),
    instance_epoch BLOB NOT NULL CHECK(length(instance_epoch)=16),
    expires_at_us INTEGER NOT NULL CHECK(expires_at_us>0),
    deadline_elapsed_us INTEGER NOT NULL CHECK(deadline_elapsed_us>0)
) STRICT;
CREATE INDEX web_confirmation_ticket ON web_login_confirmations(ticket_hash);
CREATE TABLE web_admin_sessions (
    session_hash BLOB PRIMARY KEY CHECK(length(session_hash)=32),
    record_id BLOB NOT NULL UNIQUE CHECK(length(record_id)=16),
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    uid BLOB NOT NULL REFERENCES users(uid),
    credential_id BLOB NOT NULL CHECK(length(credential_id)=33),
    certificate_expires_at_us INTEGER NOT NULL CHECK(certificate_expires_at_us>0),
    instance_epoch BLOB NOT NULL CHECK(length(instance_epoch)=16),
    sso_config_hash BLOB CHECK(sso_config_hash IS NULL OR length(sso_config_hash)=32),
    sso_policy_epoch INTEGER CHECK(sso_policy_epoch IS NULL OR sso_policy_epoch>0),
    sso_authorization_generation INTEGER CHECK(sso_authorization_generation IS NULL OR sso_authorization_generation>0),
    created_at_us INTEGER NOT NULL CHECK(created_at_us>=0),
    expires_at_us INTEGER NOT NULL CHECK(expires_at_us>created_at_us AND expires_at_us<=certificate_expires_at_us),
    deadline_elapsed_us INTEGER NOT NULL CHECK(deadline_elapsed_us>0),
    revoked_at_us INTEGER CHECK(revoked_at_us IS NULL OR revoked_at_us>=created_at_us),
    csrf_hash BLOB NOT NULL CHECK(length(csrf_hash)=32),
    CHECK ((sso_config_hash IS NULL AND sso_policy_epoch IS NULL AND sso_authorization_generation IS NULL)
        OR (sso_config_hash IS NOT NULL AND sso_policy_epoch IS NOT NULL AND sso_authorization_generation IS NOT NULL))
) STRICT;
CREATE INDEX web_session_owner ON web_admin_sessions(uid,record_id);
CREATE INDEX web_session_deadline ON web_admin_sessions(instance_epoch,deadline_elapsed_us);
CREATE TABLE web_admin_nonces (
    nonce_hash BLOB PRIMARY KEY CHECK(length(nonce_hash)=32),
    session_hash BLOB NOT NULL REFERENCES web_admin_sessions(session_hash),
    instance_epoch BLOB NOT NULL CHECK(length(instance_epoch)=16),
    admit_before_elapsed_us INTEGER NOT NULL CHECK(admit_before_elapsed_us>0),
    receipt_retain_until_elapsed_us INTEGER NOT NULL CHECK(receipt_retain_until_elapsed_us>=admit_before_elapsed_us),
    result_id BLOB CHECK(result_id IS NULL OR length(result_id)=16)
) STRICT;
CREATE INDEX web_nonce_session ON web_admin_nonces(session_hash);
CREATE TABLE admin_audit (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    actor_uid BLOB CHECK(actor_uid IS NULL OR length(actor_uid)=33),
    actor_credential_id BLOB CHECK(actor_credential_id IS NULL OR length(actor_credential_id)=33),
    action INTEGER NOT NULL CHECK(action BETWEEN 1 AND 9),
    target_id BLOB NOT NULL CHECK(length(target_id) IN (0,16,33)),
    resource_revision INTEGER CHECK(resource_revision IS NULL OR resource_revision>0),
    occurred_at_us INTEGER NOT NULL CHECK(occurred_at_us>=0),
    CHECK((actor_uid IS NULL)=(actor_credential_id IS NULL))
) STRICT;
CREATE INDEX admin_audit_time ON admin_audit(occurred_at_us,event_id);
