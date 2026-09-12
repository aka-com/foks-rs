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
