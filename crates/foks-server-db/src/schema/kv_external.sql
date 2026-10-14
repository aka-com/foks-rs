-- Version 50 stores external chunks without duplicating their payload in SQLite.
-- NULL external fields discriminate legacy inline rows. Logical charged lengths
-- retain exactly the old quota/usage policy across representation conversions.
ALTER TABLE kv_file_chunks ADD COLUMN external_digest BLOB
    CHECK (external_digest IS NULL OR length(external_digest) = 32);
ALTER TABLE kv_file_chunks ADD COLUMN external_ciphertext_bytes INTEGER
    CHECK (external_ciphertext_bytes IS NULL OR external_ciphertext_bytes BETWEEN 1 AND 9437184);
ALTER TABLE kv_file_chunks ADD COLUMN external_encoded_bytes INTEGER
    CHECK (external_encoded_bytes IS NULL OR external_encoded_bytes BETWEEN 1 AND 9438208)
    CHECK (
      (external_digest IS NULL AND external_ciphertext_bytes IS NULL AND external_encoded_bytes IS NULL)
      OR
      (external_digest IS NOT NULL AND external_ciphertext_bytes IS NOT NULL
       AND external_encoded_bytes IS NOT NULL AND length(ciphertext) = 0 AND length(exact_upload_chunk) = 0)
    );
CREATE INDEX kv_chunks_external ON kv_file_chunks(uid, file_id, clear_offset)
    WHERE external_digest IS NOT NULL;
CREATE INDEX kv_chunks_inline ON kv_file_chunks(uid, file_id, clear_offset)
    WHERE external_digest IS NULL;

DROP TRIGGER kv_file_chunks_account_insert;
DROP TRIGGER kv_file_chunks_account_delete;
DROP TRIGGER kv_file_chunks_account_update;
DROP TRIGGER kv_file_chunks_upload_insert;
DROP TRIGGER kv_file_chunks_upload_delete;
DROP TRIGGER kv_file_chunks_upload_update;

CREATE TRIGGER kv_file_chunks_account_insert AFTER INSERT ON kv_file_chunks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (coalesce(NEW.external_ciphertext_bytes, length(NEW.ciphertext)) + coalesce(NEW.external_encoded_bytes, length(NEW.exact_upload_chunk))), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_file_chunks_account_delete AFTER DELETE ON kv_file_chunks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (coalesce(OLD.external_ciphertext_bytes, length(OLD.ciphertext)) + coalesce(OLD.external_encoded_bytes, length(OLD.exact_upload_chunk))), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_file_chunks_account_update AFTER UPDATE OF uid, ciphertext, exact_upload_chunk, external_ciphertext_bytes, external_encoded_bytes ON kv_file_chunks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (coalesce(OLD.external_ciphertext_bytes, length(OLD.ciphertext)) + coalesce(OLD.external_encoded_bytes, length(OLD.exact_upload_chunk))), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (coalesce(NEW.external_ciphertext_bytes, length(NEW.ciphertext)) + coalesce(NEW.external_encoded_bytes, length(NEW.exact_upload_chunk))), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_file_chunks_upload_insert AFTER INSERT ON kv_file_chunks
BEGIN
    UPDATE kv_file_uploads SET chunk_count = chunk_count + 1, chunk_bytes = chunk_bytes + coalesce(NEW.external_ciphertext_bytes, length(NEW.ciphertext)) WHERE uid = NEW.uid AND file_id = NEW.file_id;
END;

CREATE TRIGGER kv_file_chunks_upload_delete AFTER DELETE ON kv_file_chunks
BEGIN
    UPDATE kv_file_uploads SET chunk_count = chunk_count - 1, chunk_bytes = chunk_bytes - coalesce(OLD.external_ciphertext_bytes, length(OLD.ciphertext)) WHERE uid = OLD.uid AND file_id = OLD.file_id;
END;

CREATE TRIGGER kv_file_chunks_upload_update AFTER UPDATE OF uid, file_id, ciphertext, external_ciphertext_bytes ON kv_file_chunks
BEGIN
    UPDATE kv_file_uploads SET chunk_count = chunk_count - 1, chunk_bytes = chunk_bytes - coalesce(OLD.external_ciphertext_bytes, length(OLD.ciphertext)) WHERE uid = OLD.uid AND file_id = OLD.file_id;
    UPDATE kv_file_uploads SET chunk_count = chunk_count + 1, chunk_bytes = chunk_bytes + coalesce(NEW.external_ciphertext_bytes, length(NEW.ciphertext)) WHERE uid = NEW.uid AND file_id = NEW.file_id;
END;
