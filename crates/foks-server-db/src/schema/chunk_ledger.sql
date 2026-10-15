-- Physical object accounting is independent of logical KV quota.
CREATE TABLE chunk_object_totals (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 present_bytes INTEGER NOT NULL DEFAULT 0 CHECK(present_bytes>=0),
 reserved_bytes INTEGER NOT NULL DEFAULT 0 CHECK(reserved_bytes>=0),
 uncertain_bytes INTEGER NOT NULL DEFAULT 0 CHECK(uncertain_bytes>=0),
 objects INTEGER NOT NULL DEFAULT 0 CHECK(objects>=0),
 unresolved_attempts INTEGER NOT NULL DEFAULT 0 CHECK(unresolved_attempts>=0),
 admission_rejections INTEGER NOT NULL DEFAULT 0 CHECK(admission_rejections>=0),
 inventory_complete INTEGER NOT NULL DEFAULT 0 CHECK(inventory_complete IN(0,1)),
 inventory_generation INTEGER NOT NULL DEFAULT 0 CHECK(inventory_generation>=0),
 inventory_cursor TEXT,
 inventory_running INTEGER NOT NULL DEFAULT 0 CHECK(inventory_running IN(0,1))
) STRICT;
INSERT INTO chunk_object_totals(singleton) VALUES(1);
CREATE TABLE chunk_objects (
 object_id BLOB PRIMARY KEY CHECK(length(object_id)=32),
 namespace_id BLOB CHECK(namespace_id IS NULL OR length(namespace_id)=16),
 object_key TEXT NOT NULL UNIQUE CHECK(length(object_key) BETWEEN 1 AND 1024),
 digest BLOB CHECK(digest IS NULL OR length(digest)=32),
 encoded_bytes INTEGER NOT NULL CHECK(encoded_bytes>=0),
 key_format INTEGER NOT NULL DEFAULT 1 CHECK(key_format=1),
 state INTEGER NOT NULL CHECK(state IN(1,2,3)), -- reserved, uncertain, present
 references_count INTEGER NOT NULL DEFAULT 0 CHECK(references_count>=0),
 inventory_seen INTEGER NOT NULL DEFAULT 0 CHECK(inventory_seen>=0)
) STRICT;
CREATE UNIQUE INDEX chunk_objects_digest ON chunk_objects(digest) WHERE digest IS NOT NULL;
CREATE INDEX chunk_objects_state ON chunk_objects(state,object_id);
CREATE TABLE chunk_object_attempts (
 attempt_id BLOB PRIMARY KEY CHECK(length(attempt_id)=16),
 object_id BLOB NOT NULL REFERENCES chunk_objects(object_id),
 namespace_id BLOB NOT NULL CHECK(length(namespace_id)=16),
 epoch INTEGER NOT NULL DEFAULT 1 CHECK(epoch=1),
 outcome INTEGER NOT NULL CHECK(outcome IN(1,2,3)), -- admitted, uncertain, success
 created_at INTEGER NOT NULL CHECK(created_at>=0)
) STRICT;
CREATE UNIQUE INDEX chunk_object_one_unresolved ON chunk_object_attempts(object_id) WHERE outcome IN(1,2);
CREATE INDEX chunk_object_attempt_age ON chunk_object_attempts(outcome,created_at);
CREATE TRIGGER chunk_objects_totals_insert AFTER INSERT ON chunk_objects BEGIN
 UPDATE chunk_object_totals SET objects=objects+1,
 reserved_bytes=reserved_bytes+CASE WHEN NEW.state=1 THEN NEW.encoded_bytes ELSE 0 END,
 uncertain_bytes=uncertain_bytes+CASE WHEN NEW.state=2 THEN NEW.encoded_bytes ELSE 0 END,
 present_bytes=present_bytes+CASE WHEN NEW.state=3 THEN NEW.encoded_bytes ELSE 0 END WHERE singleton=1;
END;
CREATE TRIGGER chunk_objects_totals_update AFTER UPDATE OF state,encoded_bytes ON chunk_objects BEGIN
 UPDATE chunk_object_totals SET
 reserved_bytes=reserved_bytes-CASE WHEN OLD.state=1 THEN OLD.encoded_bytes ELSE 0 END+CASE WHEN NEW.state=1 THEN NEW.encoded_bytes ELSE 0 END,
 uncertain_bytes=uncertain_bytes-CASE WHEN OLD.state=2 THEN OLD.encoded_bytes ELSE 0 END+CASE WHEN NEW.state=2 THEN NEW.encoded_bytes ELSE 0 END,
 present_bytes=present_bytes-CASE WHEN OLD.state=3 THEN OLD.encoded_bytes ELSE 0 END+CASE WHEN NEW.state=3 THEN NEW.encoded_bytes ELSE 0 END WHERE singleton=1;
END;
CREATE TRIGGER chunk_objects_totals_delete AFTER DELETE ON chunk_objects BEGIN
 UPDATE chunk_object_totals SET objects=objects-1,
 reserved_bytes=reserved_bytes-CASE WHEN OLD.state=1 THEN OLD.encoded_bytes ELSE 0 END,
 uncertain_bytes=uncertain_bytes-CASE WHEN OLD.state=2 THEN OLD.encoded_bytes ELSE 0 END,
 present_bytes=present_bytes-CASE WHEN OLD.state=3 THEN OLD.encoded_bytes ELSE 0 END WHERE singleton=1;
END;
CREATE TRIGGER chunk_attempts_totals_insert AFTER INSERT ON chunk_object_attempts BEGIN
 UPDATE chunk_object_totals SET unresolved_attempts=unresolved_attempts+CASE WHEN NEW.outcome IN(1,2) THEN 1 ELSE 0 END WHERE singleton=1;
END;
CREATE TRIGGER chunk_attempts_totals_update AFTER UPDATE OF outcome ON chunk_object_attempts BEGIN
 UPDATE chunk_object_totals SET unresolved_attempts=unresolved_attempts-CASE WHEN OLD.outcome IN(1,2) THEN 1 ELSE 0 END+CASE WHEN NEW.outcome IN(1,2) THEN 1 ELSE 0 END WHERE singleton=1;
END;
CREATE TRIGGER chunk_attempts_totals_delete AFTER DELETE ON chunk_object_attempts BEGIN
 UPDATE chunk_object_totals SET unresolved_attempts=unresolved_attempts-CASE WHEN OLD.outcome IN(1,2) THEN 1 ELSE 0 END WHERE singleton=1;
END;
-- A conflicting encoded length for one digest aborts the migration transaction.
CREATE TEMP TABLE chunk_ledger_migration_check(value INTEGER CHECK(value=0));
INSERT INTO chunk_ledger_migration_check SELECT count(*) FROM (
 SELECT external_digest FROM kv_file_chunks WHERE external_digest IS NOT NULL
 GROUP BY external_digest HAVING min(external_encoded_bytes)!=max(external_encoded_bytes));
DROP TABLE chunk_ledger_migration_check;
INSERT INTO chunk_objects(object_id,namespace_id,object_key,digest,encoded_bytes,state)
 SELECT external_digest,(SELECT namespace_id FROM chunk_storage_binding WHERE singleton=1),
 lower(hex(external_digest)),external_digest,min(external_encoded_bytes),3
 FROM kv_file_chunks WHERE external_digest IS NOT NULL GROUP BY external_digest;
ALTER TABLE kv_file_chunks ADD COLUMN external_object_id BLOB REFERENCES chunk_objects(object_id)
 CHECK(external_object_id IS NULL OR length(external_object_id)=32);
UPDATE kv_file_chunks SET external_object_id=external_digest WHERE external_digest IS NOT NULL;
CREATE INDEX kv_chunks_object_id ON kv_file_chunks(external_object_id) WHERE external_object_id IS NOT NULL;
UPDATE chunk_objects SET references_count=(SELECT count(*) FROM kv_file_chunks WHERE external_object_id=object_id);
CREATE TRIGGER chunk_reference_check_insert BEFORE INSERT ON kv_file_chunks BEGIN
 SELECT CASE WHEN (NEW.external_digest IS NULL)!=(NEW.external_object_id IS NULL) OR
 (NEW.external_object_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM chunk_objects WHERE object_id=NEW.external_object_id AND digest=NEW.external_digest AND encoded_bytes=NEW.external_encoded_bytes AND state=3))
 THEN RAISE(ABORT,'external chunk object ledger mismatch') END;
END;
CREATE TRIGGER chunk_reference_check_update BEFORE UPDATE OF external_digest,external_object_id,external_encoded_bytes ON kv_file_chunks BEGIN
 SELECT CASE WHEN (NEW.external_digest IS NULL)!=(NEW.external_object_id IS NULL) OR
 (NEW.external_object_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM chunk_objects WHERE object_id=NEW.external_object_id AND digest=NEW.external_digest AND encoded_bytes=NEW.external_encoded_bytes AND state=3))
 THEN RAISE(ABORT,'external chunk object ledger mismatch') END;
END;
CREATE TRIGGER chunk_reference_insert AFTER INSERT ON kv_file_chunks WHEN NEW.external_object_id IS NOT NULL BEGIN
 UPDATE chunk_objects SET references_count=references_count+1 WHERE object_id=NEW.external_object_id;
END;
CREATE TRIGGER chunk_reference_delete AFTER DELETE ON kv_file_chunks WHEN OLD.external_object_id IS NOT NULL BEGIN
 UPDATE chunk_objects SET references_count=references_count-1 WHERE object_id=OLD.external_object_id;
END;
CREATE TRIGGER chunk_reference_update AFTER UPDATE OF external_object_id ON kv_file_chunks BEGIN
 UPDATE chunk_objects SET references_count=references_count-1 WHERE object_id=OLD.external_object_id;
 UPDATE chunk_objects SET references_count=references_count+1 WHERE object_id=NEW.external_object_id;
END;
