-- Test-only reconstruction of schema 49; apply before removing accounting.
DROP TRIGGER kv_file_chunks_account_insert;
DROP TRIGGER kv_file_chunks_account_delete;
DROP TRIGGER kv_file_chunks_account_update;
DROP TRIGGER kv_file_chunks_upload_insert;
DROP TRIGGER kv_file_chunks_upload_delete;
DROP TRIGGER kv_file_chunks_upload_update;
DROP INDEX kv_chunks_external;
DROP INDEX kv_chunks_inline;
ALTER TABLE kv_file_chunks DROP COLUMN external_encoded_bytes;
ALTER TABLE kv_file_chunks DROP COLUMN external_ciphertext_bytes;
ALTER TABLE kv_file_chunks DROP COLUMN external_digest;
CREATE TRIGGER kv_file_chunks_account_insert AFTER INSERT ON kv_file_chunks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.ciphertext) + length(NEW.exact_upload_chunk)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_file_chunks_account_delete AFTER DELETE ON kv_file_chunks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.ciphertext) + length(OLD.exact_upload_chunk)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_file_chunks_account_update AFTER UPDATE OF uid, ciphertext, exact_upload_chunk ON kv_file_chunks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.ciphertext) + length(OLD.exact_upload_chunk)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.ciphertext) + length(NEW.exact_upload_chunk)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_file_chunks_upload_insert AFTER INSERT ON kv_file_chunks
BEGIN
    UPDATE kv_file_uploads SET chunk_count = chunk_count + 1, chunk_bytes = chunk_bytes + length(NEW.ciphertext) WHERE uid = NEW.uid AND file_id = NEW.file_id;
END;

CREATE TRIGGER kv_file_chunks_upload_delete AFTER DELETE ON kv_file_chunks
BEGIN
    UPDATE kv_file_uploads SET chunk_count = chunk_count - 1, chunk_bytes = chunk_bytes - length(OLD.ciphertext) WHERE uid = OLD.uid AND file_id = OLD.file_id;
END;

CREATE TRIGGER kv_file_chunks_upload_update AFTER UPDATE OF uid, file_id, ciphertext ON kv_file_chunks
BEGIN
    UPDATE kv_file_uploads SET chunk_count = chunk_count - 1, chunk_bytes = chunk_bytes - length(OLD.ciphertext) WHERE uid = OLD.uid AND file_id = OLD.file_id;
    UPDATE kv_file_uploads SET chunk_count = chunk_count + 1, chunk_bytes = chunk_bytes + length(NEW.ciphertext) WHERE uid = NEW.uid AND file_id = NEW.file_id;
END;
