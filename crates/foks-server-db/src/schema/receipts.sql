CREATE TABLE request_receipts (
    idempotency_key BLOB PRIMARY KEY CHECK (length(idempotency_key) BETWEEN 16 AND 64),
    request_hash BLOB NOT NULL CHECK (length(request_hash) = 32),
    response_blob BLOB NOT NULL,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at)
) STRICT;

CREATE INDEX request_receipts_expiry ON request_receipts(expires_at);
