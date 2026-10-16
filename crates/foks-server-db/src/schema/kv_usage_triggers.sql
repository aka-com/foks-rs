-- Exact-byte changes cannot retain a previously derived length. Typed callers
-- clear then stamp in their transaction; trigger ordering is irrelevant.
CREATE TRIGGER kv_nodes_usage_exact BEFORE UPDATE OF exact_node ON kv_nodes
WHEN NEW.exact_node != OLD.exact_node AND NEW.ciphertext_bytes IS NOT NULL
BEGIN SELECT RAISE(ABORT,'KV exact node requires usage revalidation'); END;

-- REPLACE can bypass DELETE triggers with recursive_triggers=OFF. Typed writes
-- pre-read under the same transaction; conflicting raw INSERT/UPSERT is refused.
CREATE TRIGGER kv_nodes_usage_collision BEFORE INSERT ON kv_nodes
WHEN EXISTS(SELECT 1 FROM kv_nodes WHERE uid=NEW.uid AND node_id=NEW.node_id)
BEGIN SELECT RAISE(ABORT,'KV node key already exists'); END;
CREATE TRIGGER kv_uploads_usage_collision BEFORE INSERT ON kv_file_uploads
WHEN EXISTS(SELECT 1 FROM kv_file_uploads WHERE uid=NEW.uid AND file_id=NEW.file_id)
BEGIN SELECT RAISE(ABORT,'KV upload key already exists'); END;

CREATE TRIGGER kv_nodes_usage_insert AFTER INSERT ON kv_nodes
BEGIN
    UPDATE kv_namespaces SET usage_small_count=usage_small_count+1,usage_small_bytes=usage_small_bytes+coalesce(NEW.ciphertext_bytes,0),usage_unvalidated_small_count=usage_unvalidated_small_count+(NEW.ciphertext_bytes IS NULL) WHERE namespace_id=NEW.uid;
END;

CREATE TRIGGER kv_nodes_usage_delete AFTER DELETE ON kv_nodes
BEGIN
    UPDATE kv_namespaces SET usage_small_count=usage_small_count-1,usage_small_bytes=usage_small_bytes-coalesce(OLD.ciphertext_bytes,0),usage_unvalidated_small_count=usage_unvalidated_small_count-(OLD.ciphertext_bytes IS NULL) WHERE namespace_id=OLD.uid;
END;

CREATE TRIGGER kv_nodes_usage_update AFTER UPDATE OF uid,ciphertext_bytes ON kv_nodes
BEGIN
    UPDATE kv_namespaces SET usage_small_count=usage_small_count-1,usage_small_bytes=usage_small_bytes-coalesce(OLD.ciphertext_bytes,0),usage_unvalidated_small_count=usage_unvalidated_small_count-(OLD.ciphertext_bytes IS NULL) WHERE namespace_id=OLD.uid;
    UPDATE kv_namespaces SET usage_small_count=usage_small_count+1,usage_small_bytes=usage_small_bytes+coalesce(NEW.ciphertext_bytes,0),usage_unvalidated_small_count=usage_unvalidated_small_count+(NEW.ciphertext_bytes IS NULL) WHERE namespace_id=NEW.uid;
END;

CREATE TRIGGER kv_uploads_usage_insert AFTER INSERT ON kv_file_uploads
BEGIN
    UPDATE kv_namespaces SET usage_large_count=usage_large_count+1,usage_large_bytes=usage_large_bytes+NEW.chunk_bytes,usage_large_chunks=usage_large_chunks+NEW.chunk_count WHERE namespace_id=NEW.uid;
END;

CREATE TRIGGER kv_uploads_usage_delete AFTER DELETE ON kv_file_uploads
BEGIN
    UPDATE kv_namespaces SET usage_large_count=usage_large_count-1,usage_large_bytes=usage_large_bytes-OLD.chunk_bytes,usage_large_chunks=usage_large_chunks-OLD.chunk_count WHERE namespace_id=OLD.uid;
END;

CREATE TRIGGER kv_uploads_usage_update AFTER UPDATE OF uid,chunk_count,chunk_bytes ON kv_file_uploads
BEGIN
    UPDATE kv_namespaces SET usage_large_count=usage_large_count-1,usage_large_bytes=usage_large_bytes-OLD.chunk_bytes,usage_large_chunks=usage_large_chunks-OLD.chunk_count WHERE namespace_id=OLD.uid;
    UPDATE kv_namespaces SET usage_large_count=usage_large_count+1,usage_large_bytes=usage_large_bytes+NEW.chunk_bytes,usage_large_chunks=usage_large_chunks+NEW.chunk_count WHERE namespace_id=NEW.uid;
END;

-- Upload identities are immutable in every typed writer. Moving parent keys
-- under deferred child FKs cannot preserve the existing per-upload totals.
CREATE TRIGGER kv_uploads_usage_identity BEFORE UPDATE OF uid,file_id ON kv_file_uploads
WHEN NEW.uid != OLD.uid OR NEW.file_id != OLD.file_id
BEGIN SELECT RAISE(ABORT,'KV upload identity is immutable'); END;

CREATE TRIGGER kv_nodes_usage_identity BEFORE UPDATE OF uid,node_id ON kv_nodes
WHEN (NEW.uid != OLD.uid OR NEW.node_id != OLD.node_id)
 AND EXISTS(SELECT 1 FROM kv_nodes WHERE uid=NEW.uid AND node_id=NEW.node_id)
BEGIN SELECT RAISE(ABORT,'KV node destination key already exists'); END;
