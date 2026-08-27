CREATE TABLE issued_certificates (
    serial BLOB PRIMARY KEY CHECK (length(serial) BETWEEN 1 AND 20),
    uid BLOB NOT NULL REFERENCES users(uid) ON DELETE CASCADE,
    device_id BLOB NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    not_before INTEGER NOT NULL CHECK (not_before >= 0),
    not_after INTEGER NOT NULL CHECK (not_after > not_before),
    exact_certificate BLOB NOT NULL,
    UNIQUE (serial, uid, device_id)
) STRICT;
