CREATE TABLE team_invitation_certificates (
    certificate_hash BLOB PRIMARY KEY CHECK(length(certificate_hash)=32),
    team_id BLOB NOT NULL REFERENCES teams(team_id),
    generation INTEGER NOT NULL CHECK(generation>=1),
    exact_certificate BLOB NOT NULL CHECK(length(exact_certificate)<=16384),
    created_at INTEGER NOT NULL CHECK(created_at>=0)
) STRICT;
CREATE INDEX team_invitation_certificates_generation ON team_invitation_certificates(team_id,generation,certificate_hash);
CREATE TABLE user_local_view_permissions (
    viewer_uid BLOB NOT NULL REFERENCES users(uid),
    target_id BLOB NOT NULL CHECK(length(target_id)=33),
    PRIMARY KEY(viewer_uid,target_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE team_local_join_requests (
    receipt BLOB PRIMARY KEY CHECK(length(receipt)=17),
    team_id BLOB NOT NULL REFERENCES teams(team_id),
    joiner_id BLOB NOT NULL CHECK(length(joiner_id)=33),
    source_role_type INTEGER NOT NULL CHECK(source_role_type BETWEEN 1 AND 3),
    source_visibility INTEGER NOT NULL,
    state INTEGER NOT NULL CHECK(state IN(0,1,2,3)),
    permission BLOB NOT NULL CHECK(length(permission)=17),
    created_ms INTEGER NOT NULL CHECK(created_ms>=0),
    decision_ms INTEGER,
    decision_sequence INTEGER,
    decision_link_hash BLOB CHECK(decision_link_hash IS NULL OR length(decision_link_hash)=32)
) STRICT;
CREATE UNIQUE INDEX team_local_join_pending ON team_local_join_requests(team_id,joiner_id,source_role_type,source_visibility) WHERE state=0;
CREATE INDEX team_local_join_inbox ON team_local_join_requests(team_id,state,created_ms DESC,receipt);
CREATE INDEX team_local_join_joiner ON team_local_join_requests(joiner_id,state);
CREATE TABLE team_remote_join_requests (
    receipt BLOB PRIMARY KEY CHECK(length(receipt)=17),
    team_id BLOB NOT NULL REFERENCES teams(team_id),
    certificate_hash BLOB NOT NULL REFERENCES team_invitation_certificates(certificate_hash),
    exact_request BLOB NOT NULL CHECK(length(exact_request)<=16384),
    state INTEGER NOT NULL CHECK(state IN(0,1,2,3)),
    created_ms INTEGER NOT NULL CHECK(created_ms>=0),
    decision_ms INTEGER,
    decision_sequence INTEGER,
    decision_link_hash BLOB CHECK(decision_link_hash IS NULL OR length(decision_link_hash)=32)
) STRICT;
CREATE INDEX team_remote_join_inbox ON team_remote_join_requests(team_id,state,created_ms DESC,receipt);
