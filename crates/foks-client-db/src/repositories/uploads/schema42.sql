CREATE TABLE kv_upload_receipt_limits (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 max_rows INTEGER NOT NULL CHECK(max_rows BETWEEN 1 AND 65536),
 max_bytes INTEGER NOT NULL CHECK(max_bytes BETWEEN 1024 AND 67108864),
 reserved_rows INTEGER NOT NULL DEFAULT 0 CHECK(reserved_rows BETWEEN 0 AND max_rows),
 reserved_bytes INTEGER NOT NULL DEFAULT 0 CHECK(reserved_bytes BETWEEN 0 AND max_bytes)
) STRICT;
INSERT INTO kv_upload_receipt_limits VALUES(1,8192,16777216,0,0);
CREATE TABLE kv_upload_handles (
 handle_hash BLOB PRIMARY KEY CHECK(length(handle_hash)=32),
 handle TEXT NOT NULL UNIQUE CHECK(length(handle) IN (32,85)),
 handle_version INTEGER NOT NULL CHECK(handle_version IN (0,1)),
 issued_at INTEGER CHECK(issued_at>=0),
 operation_id BLOB NOT NULL UNIQUE REFERENCES kv_uploads(operation_id) ON DELETE RESTRICT,
 intent_hash BLOB CHECK(length(intent_hash)=32),
 terminal_at INTEGER CHECK(terminal_at>=0),
 expires_at INTEGER CHECK(expires_at>=terminal_at),
 receipt_bytes INTEGER NOT NULL CHECK(receipt_bytes IN (0,1024)),
 CHECK((handle_version=0 AND issued_at IS NULL AND intent_hash IS NULL AND receipt_bytes=0) OR
       (handle_version=1 AND issued_at IS NOT NULL AND intent_hash IS NOT NULL AND receipt_bytes=1024)),
 CHECK((terminal_at IS NULL)=(expires_at IS NULL))
) STRICT, WITHOUT ROWID;
CREATE TRIGGER kv_upload_receipt_reserve AFTER INSERT ON kv_upload_handles WHEN NEW.handle_version=1
BEGIN UPDATE kv_upload_receipt_limits SET reserved_rows=reserved_rows+1,reserved_bytes=reserved_bytes+NEW.receipt_bytes WHERE singleton=1; END;
CREATE TRIGGER kv_upload_receipt_release AFTER DELETE ON kv_upload_handles WHEN OLD.handle_version=1
BEGIN UPDATE kv_upload_receipt_limits SET reserved_rows=reserved_rows-1,reserved_bytes=reserved_bytes-OLD.receipt_bytes WHERE singleton=1; END;
CREATE TABLE kv_upload_clocks (
 host_id BLOB NOT NULL CHECK(length(host_id)=33),
 user_id BLOB NOT NULL CHECK(length(user_id)=33),
 process_id BLOB NOT NULL CHECK(length(process_id)=16),
 anchor_wall INTEGER NOT NULL CHECK(anchor_wall>=0),
 anchor_monotonic INTEGER NOT NULL CHECK(anchor_monotonic>=0),
 validated_time_floor INTEGER NOT NULL CHECK(validated_time_floor>=0),
 reject_issued_before INTEGER NOT NULL CHECK(reject_issued_before>=0),
 PRIMARY KEY(host_id,user_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE kv_upload_cleanup (
 operation_id BLOB PRIMARY KEY REFERENCES kv_uploads(operation_id) ON DELETE CASCADE,
 phase INTEGER NOT NULL DEFAULT 0 CHECK(phase BETWEEN 0 AND 4),
 next_chunk INTEGER NOT NULL DEFAULT 0 CHECK(next_chunk BETWEEN 0 AND 256),
 parent_erased INTEGER NOT NULL DEFAULT 0 CHECK(parent_erased IN (0,1))
) STRICT, WITHOUT ROWID;
CREATE TABLE kv_upload_children (
 upload_id BLOB NOT NULL REFERENCES kv_uploads(operation_id) ON DELETE RESTRICT,
 child_id BLOB NOT NULL UNIQUE CHECK(length(child_id)=16),
 compacted INTEGER NOT NULL DEFAULT 0 CHECK(compacted IN (0,1)),
 protected_erased INTEGER NOT NULL DEFAULT 0 CHECK(protected_erased IN (0,1)),
 PRIMARY KEY(upload_id,child_id)
) STRICT, WITHOUT ROWID;
-- Preserve the association even when a terminal mutation is compacted by its
-- adapter owner. Retirement validates every surviving row and adapter receipt.
INSERT INTO kv_upload_children(upload_id,child_id) SELECT operation_id,namespace_child FROM kv_uploads WHERE namespace_child IS NOT NULL;
INSERT INTO kv_upload_children(upload_id,child_id) SELECT u.operation_id,c.child_id FROM kv_uploads u JOIN mutation_children c ON c.parent_id=u.adapter_parent WHERE c.child_id NOT IN (SELECT child_id FROM kv_upload_children);
INSERT INTO kv_upload_cleanup(operation_id) SELECT operation_id FROM kv_uploads;
CREATE INDEX kv_upload_terminal_retirement ON kv_upload_handles(expires_at,handle_hash) WHERE handle_version=1;

-- Exclude retained completed receipts from immediate maintenance discovery.
CREATE INDEX kv_upload_cleanup_pending ON kv_upload_cleanup(operation_id) WHERE phase<4;
CREATE INDEX kv_upload_handles_unstamped ON kv_upload_handles(operation_id) WHERE handle_version=1 AND terminal_at IS NULL;
