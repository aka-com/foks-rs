CREATE TABLE issued_certificates (
    serial BLOB PRIMARY KEY CHECK (length(serial) BETWEEN 1 AND 20),
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    device_id BLOB NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    credential_id BLOB NOT NULL CHECK (length(credential_id) IN (33, 34)),
    not_before INTEGER NOT NULL CHECK (not_before >= 0),
    not_after INTEGER NOT NULL CHECK (not_after > not_before),
    exact_certificate BLOB NOT NULL,
    UNIQUE (serial, uid, credential_id)
) STRICT;
