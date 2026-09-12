pub(crate) const APPLICATION_ID: i64 = 0x464f_4b53; // `FOKS`
pub(crate) const VERSION: u32 = 36;

pub(crate) const REVISION_TABLES: &[&str] = &[
    "hosts",
    "host_lookups",
    "host_services",
    "merkle_roots",
    "merkle_heads",
    "users",
    "user_devices",
    "user_shared_keys",
    "user_local_security",
    "user_generic_chains",
    "teams",
    "team_members",
    "team_shared_keys",
    "signup_operations",
    "adhoc_team_operations",
    "team_mutation_operations",
    "mutation_operations",
    "mutation_children",
    "kv_adapter_submissions",
    "kv_adapter_clocks",
    "invitation_delivery_steps",
    "federation_saga_operations",
    "scheduled_jobs",
    "sso_flows",
    "chat_operations",
    "chat_anchors",
    "chat_submissions",
];

pub(crate) const INITIAL: &str = r#"
CREATE TABLE invitation_delivery_steps (
    operation_id BLOB PRIMARY KEY REFERENCES mutation_operations(operation_id) ON DELETE CASCADE,
    phase INTEGER NOT NULL CHECK(phase BETWEEN 0 AND 3)
) STRICT, WITHOUT ROWID;
CREATE TABLE sso_flows (
    operation_id BLOB PRIMARY KEY CHECK(length(operation_id)=16),
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    uid BLOB NOT NULL CHECK(length(uid)=33),
    device_id BLOB NOT NULL CHECK(length(device_id) IN (33,34)),
    purpose INTEGER NOT NULL CHECK(purpose IN (0,1,2)),
    state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 9),
    material_hash BLOB NOT NULL CHECK(length(material_hash)=32),
    config_hash BLOB NOT NULL CHECK(length(config_hash)=32),
    expires_at INTEGER NOT NULL CHECK(expires_at>=0),
    final_operation BLOB REFERENCES mutation_operations(operation_id),
    commitment BLOB CHECK(commitment IS NULL OR length(commitment)=32),
    CHECK(final_operation IS NULL OR purpose=0),
    CHECK(state NOT IN (3,4,7) OR commitment IS NOT NULL)
) STRICT, WITHOUT ROWID;
CREATE TRIGGER sso_binding_requires_evidence BEFORE UPDATE OF state ON sso_flows
WHEN NEW.state IN (3,4,7) AND NEW.commitment IS NULL
BEGIN SELECT RAISE(ABORT,'SSO binding requires retained commitment'); END;
CREATE TRIGGER sso_commitment_immutable BEFORE UPDATE OF commitment ON sso_flows
WHEN OLD.commitment IS NOT NULL AND (NEW.commitment IS NULL OR NEW.commitment != OLD.commitment)
BEGIN SELECT RAISE(ABORT,'immutable SSO binding commitment'); END;
CREATE INDEX sso_flows_by_account ON sso_flows(host_id,uid,state);
CREATE TABLE chat_operations (
    operation_id BLOB PRIMARY KEY CHECK(length(operation_id)=16),
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    uid BLOB NOT NULL CHECK(length(uid)=33),
    team_id BLOB NOT NULL CHECK(length(team_id)=33),
    channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
    kind INTEGER NOT NULL CHECK(kind IN (0,1)),
    state INTEGER NOT NULL CHECK(state IN (0,1,2,3,4)),
    request_hash BLOB NOT NULL CHECK(length(request_hash)=32),
    scan_cursor INTEGER NOT NULL CHECK(scan_cursor>=0),
    receipt BLOB CHECK(receipt IS NULL OR length(receipt) BETWEEN 1 AND 256),
    rejection_code INTEGER,
    CHECK ((state=3) = (rejection_code IS NOT NULL)),
    CHECK ((state=2) = (receipt IS NOT NULL))
) STRICT, WITHOUT ROWID;
CREATE INDEX chat_pending ON chat_operations(host_id,uid,team_id,state);
CREATE TABLE chat_submissions (
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    uid BLOB NOT NULL CHECK(length(uid)=33),
    team_id BLOB NOT NULL CHECK(length(team_id)=33),
    submission_id BLOB NOT NULL CHECK(length(submission_id)=16),
    input_mac BLOB NOT NULL CHECK(length(input_mac)=32),
    operation_id BLOB NOT NULL REFERENCES chat_operations(operation_id),
    PRIMARY KEY(host_id, uid, team_id, submission_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE chat_anchors (
    host_id BLOB NOT NULL CHECK(length(host_id)=33),
    uid BLOB NOT NULL CHECK(length(uid)=33),
    team_id BLOB NOT NULL CHECK(length(team_id)=33),
    channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
    sequence INTEGER NOT NULL CHECK(sequence>0),
    message_id BLOB NOT NULL CHECK(length(message_id)=16),
    digest BLOB NOT NULL CHECK(length(digest)=32),
    PRIMARY KEY(host_id,uid,team_id,channel_id,sequence),
    UNIQUE(host_id,uid,team_id,channel_id,message_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE hard_state_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    database_id BLOB NOT NULL UNIQUE CHECK (length(database_id) = 16),
    hard_state_revision INTEGER NOT NULL CHECK (hard_state_revision >= 0),
    write_token BLOB NOT NULL CHECK (length(write_token) = 16)
) STRICT, WITHOUT ROWID;

CREATE TABLE hosts (
    host_id BLOB PRIMARY KEY CHECK (length(host_id) > 0),
    canonical_name TEXT NOT NULL CHECK (length(canonical_name) > 0),
    genesis_key BLOB NOT NULL CHECK (length(genesis_key) > 0),
    chain_seqno INTEGER NOT NULL CHECK (chain_seqno >= 0),
    chain_tail_hash BLOB NOT NULL CHECK (length(chain_tail_hash) = 32),
    chain_bytes BLOB NOT NULL CHECK (length(chain_bytes) > 0),
    public_zone_bytes BLOB NOT NULL CHECK (length(public_zone_bytes) > 0)
) STRICT, WITHOUT ROWID;

CREATE TABLE host_lookups (
    lookup_name TEXT PRIMARY KEY CHECK (length(lookup_name) > 0),
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE RESTRICT
) STRICT, WITHOUT ROWID;

CREATE TABLE host_services (
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE CASCADE,
    service_type INTEGER NOT NULL CHECK (service_type IN (1, 2, 5, 10, 12, 16)),
    endpoint_bytes BLOB NOT NULL CHECK (length(endpoint_bytes) > 0),
    valid_at_chain_seqno INTEGER NOT NULL CHECK (valid_at_chain_seqno >= 0),
    PRIMARY KEY (host_id, service_type)
) STRICT, WITHOUT ROWID;

CREATE TABLE merkle_roots (
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    root_hash BLOB NOT NULL CHECK (length(root_hash) = 32),
    root_bytes BLOB CHECK (root_bytes IS NULL OR length(root_bytes) > 0),
    PRIMARY KEY (host_id, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE merkle_heads (
    host_id BLOB PRIMARY KEY REFERENCES hosts(host_id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    root_hash BLOB NOT NULL CHECK (length(root_hash) = 32),
    evidence_kind INTEGER NOT NULL CHECK (evidence_kind IN (1, 2, 3)),
    anchor_epoch INTEGER CHECK (anchor_epoch IS NULL OR anchor_epoch >= 0),
    evidence_bytes BLOB NOT NULL CHECK (length(evidence_bytes) > 0),
    FOREIGN KEY (host_id, epoch) REFERENCES merkle_roots(host_id, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE users (
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE CASCADE,
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    chain_seqno INTEGER NOT NULL CHECK (chain_seqno > 0),
    chain_tail_hash BLOB NOT NULL CHECK (length(chain_tail_hash) = 32),
    chain_bytes BLOB NOT NULL CHECK (length(chain_bytes) > 0),
    evidence_bytes BLOB NOT NULL CHECK (length(evidence_bytes) > 0),
    username BLOB NOT NULL CHECK (length(username) BETWEEN 3 AND 25),
    username_utf8 BLOB NOT NULL CHECK (length(username_utf8) > 0),
    username_sequence INTEGER NOT NULL CHECK (username_sequence > 0),
    merkle_epoch INTEGER NOT NULL CHECK (merkle_epoch >= 0),
    merkle_root_hash BLOB NOT NULL CHECK (length(merkle_root_hash) = 32),
    merkle_root_bytes BLOB NOT NULL CHECK (length(merkle_root_bytes) > 0),
    PRIMARY KEY (host_id, uid),
    FOREIGN KEY (host_id, merkle_epoch) REFERENCES merkle_roots(host_id, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE user_devices (
    host_id BLOB NOT NULL,
    uid BLOB NOT NULL,
    device_id BLOB NOT NULL CHECK (length(device_id) IN (33, 34)),
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    role_visibility INTEGER NOT NULL CHECK (
        (role_type = 1 AND role_visibility BETWEEN -32768 AND 32767) OR
        (role_type IN (2, 3) AND role_visibility = 0)
    ),
    hepk_bytes BLOB NOT NULL CHECK (length(hepk_bytes) > 0),
    subkey_id BLOB CHECK (subkey_id IS NULL OR length(subkey_id) = 33),
    PRIMARY KEY (host_id, uid, device_id),
    FOREIGN KEY (host_id, uid) REFERENCES users(host_id, uid) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE user_shared_keys (
    host_id BLOB NOT NULL,
    uid BLOB NOT NULL,
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    role_visibility INTEGER NOT NULL CHECK (
        (role_type = 1 AND role_visibility BETWEEN -32768 AND 32767) OR
        (role_type IN (2, 3) AND role_visibility = 0)
    ),
    generation INTEGER NOT NULL CHECK (generation > 0),
    verify_key BLOB NOT NULL CHECK (length(verify_key) = 33),
    hepk_bytes BLOB NOT NULL CHECK (length(hepk_bytes) > 0),
    PRIMARY KEY (host_id, uid, role_type, role_visibility, generation),
    FOREIGN KEY (host_id, uid) REFERENCES users(host_id, uid) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- A local assertion derived from an exact signup request that omitted PPE.
-- It lets an unattended owner rotation distinguish a genuinely passphrase-
-- free Rust account from a legacy Go account whose server-only PPE has no
-- UserSettings link.
CREATE TABLE user_local_security (
    host_id BLOB NOT NULL,
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    passphrase_absence_attested INTEGER NOT NULL CHECK (passphrase_absence_attested IN (0, 1)),
    trusted_ppe_hash BLOB CHECK (trusted_ppe_hash IS NULL OR length(trusted_ppe_hash) = 32),
    CHECK (
        (passphrase_absence_attested = 1 AND trusted_ppe_hash IS NULL) OR
        (passphrase_absence_attested = 0 AND trusted_ppe_hash IS NOT NULL)
    ),
    PRIMARY KEY (host_id, uid),
    FOREIGN KEY (host_id, uid) REFERENCES users(host_id, uid) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE user_generic_chains (
    host_id BLOB NOT NULL,
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    chain_type INTEGER NOT NULL CHECK (chain_type IN (2, 4)),
    seqno INTEGER NOT NULL CHECK (seqno >= 0),
    tail_hash BLOB CHECK (
        (seqno = 0 AND tail_hash IS NULL) OR
        (seqno > 0 AND length(tail_hash) = 32)
    ),
    chain_bytes BLOB NOT NULL CHECK (length(chain_bytes) > 0),
    merkle_epoch INTEGER NOT NULL CHECK (merkle_epoch >= 0),
    merkle_root_hash BLOB NOT NULL CHECK (length(merkle_root_hash) = 32),
    PRIMARY KEY (host_id, uid, chain_type),
    FOREIGN KEY (host_id, uid) REFERENCES users(host_id, uid) ON DELETE CASCADE,
    FOREIGN KEY (host_id, merkle_epoch) REFERENCES merkle_roots(host_id, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE teams (
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE CASCADE,
    team_id BLOB NOT NULL CHECK (length(team_id) = 33),
    chain_seqno INTEGER NOT NULL CHECK (chain_seqno > 0),
    chain_tail_hash BLOB NOT NULL CHECK (length(chain_tail_hash) = 32),
    chain_bytes BLOB NOT NULL CHECK (length(chain_bytes) > 0),
    evidence_bytes BLOB NOT NULL CHECK (length(evidence_bytes) > 0),
    team_name BLOB NOT NULL CHECK (length(team_name) > 0),
    team_name_utf8 BLOB NOT NULL CHECK (length(team_name_utf8) > 0),
    team_name_sequence INTEGER NOT NULL CHECK (team_name_sequence >= 0),
    index_low_infinity INTEGER NOT NULL CHECK (index_low_infinity IN (0, 1)),
    index_low_base BLOB NOT NULL CHECK (length(index_low_base) <= 32),
    index_low_exponent INTEGER NOT NULL CHECK (index_low_exponent BETWEEN -4096 AND 4096),
    index_high_infinity INTEGER NOT NULL CHECK (index_high_infinity IN (0, 1)),
    index_high_base BLOB NOT NULL CHECK (length(index_high_base) <= 32),
    index_high_exponent INTEGER NOT NULL CHECK (index_high_exponent BETWEEN -4096 AND 4096),
    merkle_epoch INTEGER NOT NULL CHECK (merkle_epoch >= 0),
    merkle_root_hash BLOB NOT NULL CHECK (length(merkle_root_hash) = 32),
    merkle_root_bytes BLOB NOT NULL CHECK (length(merkle_root_bytes) > 0),
    CHECK (index_low_infinity = 0 OR (length(index_low_base) = 0 AND index_low_exponent = 0)),
    CHECK (index_high_infinity = 0 OR (length(index_high_base) = 0 AND index_high_exponent = 0)),
    PRIMARY KEY (host_id, team_id),
    FOREIGN KEY (host_id, merkle_epoch) REFERENCES merkle_roots(host_id, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE team_members (
    host_id BLOB NOT NULL,
    team_id BLOB NOT NULL,
    party_id BLOB NOT NULL CHECK (length(party_id) = 33),
    scoped_host_id BLOB NOT NULL CHECK (length(scoped_host_id) IN (0, 33)),
    source_role_type INTEGER NOT NULL CHECK (source_role_type BETWEEN 1 AND 3),
    source_role_visibility INTEGER NOT NULL CHECK (
        (source_role_type = 1 AND source_role_visibility BETWEEN -32768 AND 32767) OR
        (source_role_type IN (2, 3) AND source_role_visibility = 0)
    ),
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    role_visibility INTEGER NOT NULL CHECK (
        (role_type = 1 AND role_visibility BETWEEN -32768 AND 32767) OR
        (role_type IN (2, 3) AND role_visibility = 0)
    ),
    generation INTEGER NOT NULL CHECK (generation > 0),
    verify_key BLOB NOT NULL CHECK (length(verify_key) = 33),
    hepk_fingerprint BLOB NOT NULL CHECK (length(hepk_fingerprint) = 32),
    removal_key_commitment BLOB NOT NULL CHECK (length(removal_key_commitment) IN (0, 32)),
    index_range_present INTEGER NOT NULL CHECK (index_range_present IN (0, 1)),
    index_low_infinity INTEGER NOT NULL CHECK (index_low_infinity IN (0, 1)),
    index_low_base BLOB NOT NULL CHECK (length(index_low_base) <= 32),
    index_low_exponent INTEGER NOT NULL CHECK (index_low_exponent BETWEEN -4096 AND 4096),
    index_high_infinity INTEGER NOT NULL CHECK (index_high_infinity IN (0, 1)),
    index_high_base BLOB NOT NULL CHECK (length(index_high_base) <= 32),
    index_high_exponent INTEGER NOT NULL CHECK (index_high_exponent BETWEEN -4096 AND 4096),
    CHECK (index_low_infinity = 0 OR (length(index_low_base) = 0 AND index_low_exponent = 0)),
    CHECK (index_high_infinity = 0 OR (length(index_high_base) = 0 AND index_high_exponent = 0)),
    CHECK (
        index_range_present = 1 OR (
            index_low_infinity = 0 AND length(index_low_base) = 0 AND index_low_exponent = 0 AND
            index_high_infinity = 0 AND length(index_high_base) = 0 AND index_high_exponent = 0
        )
    ),
    PRIMARY KEY (
        host_id, team_id, party_id, scoped_host_id,
        source_role_type, source_role_visibility
    ),
    FOREIGN KEY (host_id, team_id) REFERENCES teams(host_id, team_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE team_shared_keys (
    host_id BLOB NOT NULL,
    team_id BLOB NOT NULL,
    role_type INTEGER NOT NULL CHECK (role_type BETWEEN 1 AND 3),
    role_visibility INTEGER NOT NULL CHECK (
        (role_type = 1 AND role_visibility BETWEEN -32768 AND 32767) OR
        (role_type IN (2, 3) AND role_visibility = 0)
    ),
    generation INTEGER NOT NULL CHECK (generation > 0),
    verify_key BLOB NOT NULL CHECK (length(verify_key) = 33),
    hepk_bytes BLOB NOT NULL CHECK (length(hepk_bytes) > 0),
    PRIMARY KEY (host_id, team_id, role_type, role_visibility),
    FOREIGN KEY (host_id, team_id) REFERENCES teams(host_id, team_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- Public crash-recovery journal for signed account mutations. Secret seeds,
-- reservation tokens, and self tokens are deliberately excluded.
CREATE TABLE signup_operations (
    operation_id BLOB PRIMARY KEY CHECK (length(operation_id) = 16),
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE RESTRICT,
    normalized_username BLOB NOT NULL CHECK (length(normalized_username) BETWEEN 3 AND 25),
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    device_id BLOB NOT NULL CHECK (length(device_id) = 33),
    request_hash BLOB NOT NULL CHECK (length(request_hash) = 32),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 3),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT, WITHOUT ROWID;

-- Public crash-recovery journal for ad-hoc team creation. PTK seeds and
-- hidden tree locations remain exclusively in the caller's encrypted store.
CREATE TABLE adhoc_team_operations (
    operation_id BLOB PRIMARY KEY CHECK (length(operation_id) = 16),
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE RESTRICT,
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    device_id BLOB NOT NULL CHECK (length(device_id) = 33),
    team_id BLOB NOT NULL CHECK (length(team_id) = 33),
    request_hash BLOB NOT NULL CHECK (length(request_hash) = 32),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 3),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (host_id, team_id)
) STRICT, WITHOUT ROWID;

-- Public crash-recovery journal for named-team creation and later edits.
-- Names, reservation tokens, PTK seeds, removal keys, and hidden locations
-- remain exclusively in the caller's encrypted store.
CREATE TABLE team_mutation_operations (
    operation_id BLOB PRIMARY KEY CHECK (length(operation_id) = 16),
    operation_kind INTEGER NOT NULL CHECK (operation_kind BETWEEN 1 AND 4),
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE RESTRICT,
    actor_id BLOB NOT NULL CHECK (length(actor_id) = 33),
    device_id BLOB NOT NULL CHECK (length(device_id) IN (33, 34)),
    team_id BLOB NOT NULL CHECK (length(team_id) = 33),
    expected_seqno INTEGER NOT NULL CHECK (expected_seqno > 0),
    request_hash BLOB NOT NULL CHECK (length(request_hash) = 32),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 7),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT, WITHOUT ROWID;

-- A live or verified operation reserves a team-chain position. A definitively
-- rejected or superseded operation remains available for audit without
-- preventing a corrected mutation against the still-current head.
CREATE UNIQUE INDEX team_mutation_reserved_position
ON team_mutation_operations (host_id, team_id, expected_seqno)
WHERE state IN (1, 2, 3, 4, 5);

-- Generic public write-ahead journal shared by all client mutations. Exact
-- retry material is referenced by an opaque key and must live in a separate
-- protected material store; it is never written to this hard-state database.
CREATE TABLE mutation_operations (
    operation_id BLOB PRIMARY KEY CHECK (length(operation_id) = 16),
    operation_kind INTEGER NOT NULL CHECK (operation_kind BETWEEN 1 AND 11),
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE RESTRICT,
    scope_id BLOB NOT NULL CHECK (length(scope_id) IN (0, 16, 33, 34)),
    subject_id BLOB NOT NULL CHECK (length(subject_id) IN (0, 16, 33, 34)),
    expected_version INTEGER CHECK (expected_version IS NULL OR expected_version >= 0),
    request_hash BLOB NOT NULL CHECK (length(request_hash) = 32),
    material_ref BLOB NOT NULL CHECK (length(material_ref) BETWEEN 1 AND 255),
    material_hash BLOB NOT NULL CHECK (length(material_hash) = 32),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 6),
    attempt_count INTEGER NOT NULL CHECK (attempt_count >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT, WITHOUT ROWID;

CREATE TABLE kv_adapter_clocks (
    host_id BLOB NOT NULL REFERENCES hosts(host_id),
    user_id BLOB NOT NULL CHECK(length(user_id)=33),
    process_id BLOB NOT NULL CHECK(length(process_id)=16),
    anchor_wall INTEGER NOT NULL CHECK(anchor_wall>=0),
    anchor_monotonic INTEGER NOT NULL CHECK(anchor_monotonic>=0),
    admission_floor INTEGER NOT NULL CHECK(admission_floor>=0),
    reject_issued_before INTEGER NOT NULL CHECK(reject_issued_before>=0),
    PRIMARY KEY(host_id,user_id)
) STRICT, WITHOUT ROWID;
CREATE TRIGGER kv_adapter_clock_irreversible BEFORE UPDATE ON kv_adapter_clocks
WHEN NEW.reject_issued_before < OLD.reject_issued_before
BEGIN SELECT RAISE(ABORT, 'adapter replay watermark cannot decrease'); END;

CREATE TABLE kv_adapter_submissions (
    handle_hash BLOB PRIMARY KEY CHECK(length(handle_hash)=32),
    handle TEXT NOT NULL UNIQUE CHECK(length(handle)=52),
    handle_version INTEGER NOT NULL CHECK(handle_version=1),
    schema_version INTEGER NOT NULL CHECK(schema_version=1),
    host_id BLOB NOT NULL REFERENCES hosts(host_id),
    user_id BLOB NOT NULL CHECK(length(user_id)=33),
    team_id BLOB NOT NULL CHECK(length(team_id) IN (0,33)),
    input_hash BLOB NOT NULL CHECK(length(input_hash)=32),
    internal_id BLOB UNIQUE REFERENCES mutation_operations(operation_id)
        DEFERRABLE INITIALLY DEFERRED,
    state INTEGER NOT NULL CHECK(state IN (0,1,2)),
    ancillary_committed INTEGER NOT NULL DEFAULT 0 CHECK(ancillary_committed IN (0,1)),
    node_id BLOB CHECK(node_id IS NULL OR length(node_id)=17),
    issued_at INTEGER NOT NULL CHECK(issued_at>=0),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    terminal_at INTEGER CHECK(terminal_at IS NULL OR terminal_at>=0),
    expires_at INTEGER CHECK(expires_at IS NULL OR expires_at>=terminal_at),
    CHECK(state != 0 OR (internal_id IS NOT NULL AND terminal_at IS NULL)),
    CHECK((terminal_at IS NULL) = (expires_at IS NULL)),
    CHECK(internal_id IS NOT NULL OR state IN (1,2))
) STRICT, WITHOUT ROWID;
CREATE INDEX kv_adapter_account ON kv_adapter_submissions(host_id,user_id,state);
CREATE INDEX kv_adapter_expiry ON kv_adapter_submissions(host_id,user_id,expires_at,handle_hash)
    WHERE internal_id IS NULL AND state IN (1,2);
CREATE INDEX kv_adapter_live ON kv_adapter_submissions(host_id,user_id,internal_id)
    WHERE internal_id IS NOT NULL;
CREATE INDEX kv_adapter_maintenance ON kv_adapter_submissions(handle_hash)
    WHERE internal_id IS NOT NULL OR terminal_at IS NULL;
CREATE INDEX kv_adapter_account_maintenance ON kv_adapter_submissions(host_id,user_id,handle_hash)
    WHERE internal_id IS NOT NULL OR terminal_at IS NULL;
CREATE TRIGGER kv_adapter_capacity BEFORE INSERT ON kv_adapter_submissions
BEGIN
    SELECT CASE WHEN (SELECT count(*) FROM kv_adapter_submissions
        WHERE host_id=NEW.host_id AND user_id=NEW.user_id) >= 65536
        THEN RAISE(ABORT, 'adapter retention-full') END;
    SELECT CASE WHEN (SELECT count(*) FROM kv_adapter_submissions
        WHERE host_id=NEW.host_id AND user_id=NEW.user_id AND state=0) >= 64
        THEN RAISE(ABORT, 'adapter active-full') END;
END;

CREATE TRIGGER kv_adapter_immutable_binding BEFORE UPDATE ON kv_adapter_submissions
WHEN NEW.handle_hash!=OLD.handle_hash OR NEW.handle!=OLD.handle
 OR NEW.handle_version!=OLD.handle_version OR NEW.schema_version!=OLD.schema_version
 OR NEW.host_id!=OLD.host_id OR NEW.user_id!=OLD.user_id OR NEW.team_id!=OLD.team_id
 OR NEW.input_hash!=OLD.input_hash OR NEW.issued_at!=OLD.issued_at OR NEW.created_at!=OLD.created_at
 OR (NEW.internal_id IS NOT OLD.internal_id AND NEW.internal_id IS NOT NULL)
 OR (OLD.state!=0 AND NEW.state!=OLD.state)
 OR NEW.ancillary_committed<OLD.ancillary_committed
 OR (OLD.node_id IS NOT NULL AND NEW.node_id IS NOT OLD.node_id)
 OR (OLD.terminal_at IS NOT NULL AND NEW.terminal_at IS NOT OLD.terminal_at)
 OR (OLD.expires_at IS NOT NULL AND NEW.expires_at IS NOT OLD.expires_at)
BEGIN
    SELECT RAISE(ABORT, 'immutable adapter evidence changed');
END;

-- Alternate writers cannot create an adapter parent without its reserved ledger slot.
CREATE TRIGGER kv_adapter_parent_binding BEFORE INSERT ON mutation_operations
WHEN NEW.operation_kind=8
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM kv_adapter_submissions
        WHERE internal_id=NEW.operation_id AND host_id=NEW.host_id AND user_id=NEW.scope_id
          AND team_id=NEW.subject_id AND input_hash=NEW.request_hash AND state=0)
        THEN RAISE(ABORT, 'adapter parent requires bound ledger admission') END;
END;
CREATE TRIGGER kv_adapter_terminal_evidence AFTER UPDATE OF state ON mutation_operations
WHEN NEW.operation_kind=8 AND NEW.state IN (5,6)
BEGIN
    UPDATE kv_adapter_submissions SET state=CASE NEW.state WHEN 6 THEN 1 ELSE 2 END
      WHERE internal_id=NEW.operation_id AND state=0;
END;

-- A high-level adapter intent owns the lower-level KV records it caused.
-- Completion labels identify final namespace attempts, including rejected CAS retries.
CREATE TABLE mutation_children (
    parent_id BLOB NOT NULL REFERENCES mutation_operations(operation_id) ON DELETE RESTRICT,
    child_id BLOB PRIMARY KEY REFERENCES mutation_operations(operation_id) ON DELETE RESTRICT,
    completion INTEGER NOT NULL CHECK (completion IN (0,1)),
    CHECK (parent_id != child_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX mutation_children_parent ON mutation_children(parent_id);
CREATE TRIGGER mutation_children_immutable BEFORE UPDATE ON mutation_children
BEGIN SELECT RAISE(ABORT, 'adapter child binding is immutable'); END;
CREATE TRIGGER mutation_children_capacity BEFORE INSERT ON mutation_children
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM mutation_operations p JOIN mutation_operations c
        ON c.operation_id=NEW.child_id WHERE p.operation_id=NEW.parent_id AND p.operation_kind=8 AND p.state=2
        AND c.operation_kind IN (5,6,7) AND c.state=1 AND c.host_id=p.host_id
        AND c.scope_id=CASE WHEN length(p.subject_id)=0 THEN p.scope_id ELSE p.subject_id END
        AND (NEW.completion=0 OR c.operation_kind=5))
        THEN RAISE(ABORT, 'adapter child scope or state mismatch') END;
    SELECT CASE WHEN (SELECT count(*) FROM mutation_children WHERE parent_id=NEW.parent_id)>=256
        THEN RAISE(ABORT, 'adapter child limit exceeded') END;
END;

CREATE INDEX mutation_operations_pending
ON mutation_operations (host_id, state, updated_at)
WHERE state IN (1, 2, 3, 4);

-- Device and PUK operations all append to the user chain. At most one live
-- request may reserve a given authenticated chain position, even when an
-- alternate owner is reconciling an unavailable original signer.
CREATE UNIQUE INDEX user_mutation_reserved_chain_position
ON mutation_operations (host_id, scope_id, expected_version)
WHERE operation_kind IN (2, 3, 4, 9, 10) AND state IN (1, 2, 3, 4);

CREATE UNIQUE INDEX account_convenience_active_preparation
ON mutation_operations(host_id,scope_id) WHERE operation_kind=9 AND state IN (1,2);

CREATE UNIQUE INDEX bot_enrollment_active_preparation
ON mutation_operations(host_id,scope_id) WHERE operation_kind=10 AND state IN (1,2);

-- Bounds survive concurrent processes. Unknown submissions are never evicted.
CREATE TRIGGER account_convenience_capacity BEFORE INSERT ON mutation_operations
WHEN NEW.operation_kind IN (9,10,11) BEGIN
    SELECT CASE WHEN (SELECT count(*) FROM mutation_operations WHERE host_id=NEW.host_id AND scope_id=NEW.scope_id AND operation_kind IN (9,10,11) AND state IN (1,2,3,4)) >= 32
        THEN RAISE(ABORT, 'account pending operation capacity') END;
    SELECT CASE WHEN (SELECT count(*) FROM mutation_operations WHERE host_id=NEW.host_id AND scope_id=NEW.scope_id AND operation_kind IN (9,10,11)) >= 4096
        THEN RAISE(ABORT, 'account retained operation capacity') END;
END;

-- Cross-host coordination contains only public identities, capability hashes,
-- and local journal references. Bearer tokens and removal keys remain in the
-- caller's protected memory or credential store.
CREATE TABLE federation_saga_operations (
    operation_id BLOB PRIMARY KEY CHECK (length(operation_id) = 16),
    local_host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE RESTRICT
        CHECK (length(local_host_id) = 33 AND substr(local_host_id, 1, 1) = x'02'),
    remote_host_id BLOB NOT NULL
        CHECK (length(remote_host_id) = 33 AND substr(remote_host_id, 1, 1) = x'02'),
    actor_id BLOB NOT NULL
        CHECK (length(actor_id) = 33 AND substr(actor_id, 1, 1) = x'01'),
    local_team_id BLOB NOT NULL
        CHECK (length(local_team_id) = 33 AND substr(local_team_id, 1, 1) = x'03'),
    remote_party_id BLOB NOT NULL CHECK (
        length(remote_party_id) = 33
        AND substr(remote_party_id, 1, 1) IN (x'03', x'14')
    ),
    permission_hash BLOB NOT NULL CHECK (length(permission_hash) = 32),
    destination_role_type INTEGER NOT NULL CHECK (destination_role_type BETWEEN 1 AND 3),
    destination_visibility INTEGER NOT NULL CHECK (
        (destination_role_type = 1 AND destination_visibility BETWEEN -32768 AND 32767)
        OR (destination_role_type IN (2, 3) AND destination_visibility = 0)
    ),
    removal_key_commitment BLOB NOT NULL CHECK (length(removal_key_commitment) = 32),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 6),
    expected_local_seqno INTEGER CHECK (expected_local_seqno IS NULL OR expected_local_seqno > 0),
    local_mutation_id BLOB CHECK (local_mutation_id IS NULL OR length(local_mutation_id) = 16),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK ((expected_local_seqno IS NULL) = (local_mutation_id IS NULL))
) STRICT, WITHOUT ROWID;

CREATE UNIQUE INDEX federation_saga_active_binding
ON federation_saga_operations (
    local_host_id, local_team_id, remote_host_id, remote_party_id
)
WHERE state BETWEEN 1 AND 4;

CREATE INDEX federation_saga_pending
ON federation_saga_operations (local_host_id, state, updated_at)
WHERE state BETWEEN 1 AND 4;

-- Durable application scheduling metadata. Credentials and job payloads stay
-- outside this public hard-state database; the scope only identifies the
-- application-owned account or reconciliation target.
CREATE TABLE scheduled_jobs (
    job_id BLOB PRIMARY KEY CHECK (length(job_id) = 16),
    job_kind INTEGER NOT NULL CHECK (job_kind IN (1, 2, 3, 4, 5)),
    host_id BLOB NOT NULL REFERENCES hosts(host_id) ON DELETE CASCADE,
    scope_id BLOB NOT NULL CHECK (length(scope_id) BETWEEN 0 AND 1024),
    interval_micros INTEGER NOT NULL CHECK (interval_micros > 0),
    next_run_at INTEGER NOT NULL CHECK (next_run_at >= 0),
    failure_count INTEGER NOT NULL CHECK (failure_count >= 0),
    lease_until INTEGER CHECK (lease_until IS NULL OR lease_until >= 0),
    last_completed_at INTEGER CHECK (last_completed_at IS NULL OR last_completed_at >= 0),
    last_error TEXT,
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT, WITHOUT ROWID;

CREATE INDEX scheduled_jobs_due
ON scheduled_jobs (next_run_at, job_id);
"#;
