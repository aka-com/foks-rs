-- Version 49: materialized quota accounting. All mutations, including cascades
-- and maintenance, update these counters in the same transaction as their rows.
-- STRICT integer columns reject overflow; nonnegative checks detect underflow.
ALTER TABLE kv_namespaces ADD COLUMN accounted_bytes INTEGER NOT NULL DEFAULT 0 CHECK (accounted_bytes >= 0);
ALTER TABLE kv_namespaces ADD COLUMN accounted_objects INTEGER NOT NULL DEFAULT 0 CHECK (accounted_objects >= 0);
ALTER TABLE kv_namespaces ADD COLUMN directory_count INTEGER NOT NULL DEFAULT 0 CHECK (directory_count >= 0);
ALTER TABLE kv_namespaces ADD COLUMN dirent_count INTEGER NOT NULL DEFAULT 0 CHECK (dirent_count >= 0);
ALTER TABLE kv_file_uploads ADD COLUMN chunk_count INTEGER NOT NULL DEFAULT 0 CHECK (chunk_count >= 0);
ALTER TABLE kv_file_uploads ADD COLUMN chunk_bytes INTEGER NOT NULL DEFAULT 0 CHECK (chunk_bytes >= 0);

UPDATE kv_namespaces SET
    accounted_bytes = coalesce((SELECT sum(length(exact_directory)) FROM kv_directories WHERE uid = namespace_id), 0) +
        coalesce((SELECT sum(length(exact_root)) FROM kv_roots WHERE uid = namespace_id), 0) +
        coalesce((SELECT sum(length(exact_node)) FROM kv_nodes WHERE uid = namespace_id), 0) +
        coalesce((SELECT sum(length(exact_dirent)) FROM kv_dirents WHERE uid = namespace_id), 0) +
        coalesce((SELECT sum(length(exact_metadata)) FROM kv_file_uploads WHERE uid = namespace_id), 0) +
        coalesce((SELECT sum(length(ciphertext) + length(exact_upload_chunk)) FROM kv_file_chunks WHERE uid = namespace_id), 0) +
        coalesce((SELECT sum(64) FROM kv_locks WHERE uid = namespace_id), 0),
    accounted_objects = (SELECT count(*) FROM kv_directories WHERE uid = namespace_id) +
        (SELECT count(*) FROM kv_roots WHERE uid = namespace_id) +
        (SELECT count(*) FROM kv_nodes WHERE uid = namespace_id) +
        (SELECT count(*) FROM kv_dirents WHERE uid = namespace_id) +
        (SELECT count(*) FROM kv_file_uploads WHERE uid = namespace_id) +
        (SELECT count(*) FROM kv_file_chunks WHERE uid = namespace_id) +
        (SELECT count(*) FROM kv_locks WHERE uid = namespace_id),
    directory_count = (SELECT count(*) FROM kv_directory_heads WHERE uid = namespace_id),
    dirent_count = (SELECT count(*) FROM kv_dirent_heads WHERE uid = namespace_id);
UPDATE kv_file_uploads SET
    chunk_count = (SELECT count(*) FROM kv_file_chunks c WHERE c.uid = kv_file_uploads.uid AND c.file_id = kv_file_uploads.file_id),
    chunk_bytes = coalesce((SELECT sum(length(ciphertext)) FROM kv_file_chunks c WHERE c.uid = kv_file_uploads.uid AND c.file_id = kv_file_uploads.file_id), 0);

CREATE TRIGGER kv_directories_account_insert AFTER INSERT ON kv_directories
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_directory)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_directories_account_delete AFTER DELETE ON kv_directories
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_directory)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_directories_account_update AFTER UPDATE OF uid, exact_directory ON kv_directories
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_directory)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_directory)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_roots_account_insert AFTER INSERT ON kv_roots
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_root)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_roots_account_delete AFTER DELETE ON kv_roots
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_root)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_roots_account_update AFTER UPDATE OF uid, exact_root ON kv_roots
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_root)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_root)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_nodes_account_insert AFTER INSERT ON kv_nodes
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_node)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_nodes_account_delete AFTER DELETE ON kv_nodes
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_node)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_nodes_account_update AFTER UPDATE OF uid, exact_node ON kv_nodes
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_node)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_node)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_dirents_account_insert AFTER INSERT ON kv_dirents
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_dirent)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_dirents_account_delete AFTER DELETE ON kv_dirents
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_dirent)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_dirents_account_update AFTER UPDATE OF uid, exact_dirent ON kv_dirents
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_dirent)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_dirent)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_file_uploads_account_insert AFTER INSERT ON kv_file_uploads
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_metadata)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_file_uploads_account_delete AFTER DELETE ON kv_file_uploads
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_metadata)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_file_uploads_account_update AFTER UPDATE OF uid, exact_metadata ON kv_file_uploads
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (length(OLD.exact_metadata)), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (length(NEW.exact_metadata)), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

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

CREATE TRIGGER kv_locks_account_insert AFTER INSERT ON kv_locks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (64), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_locks_account_delete AFTER DELETE ON kv_locks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (64), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_locks_account_update AFTER UPDATE OF uid ON kv_locks
BEGIN
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes - (64), accounted_objects = accounted_objects - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET accounted_bytes = accounted_bytes + (64), accounted_objects = accounted_objects + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_directory_heads_account_insert AFTER INSERT ON kv_directory_heads
BEGIN
    UPDATE kv_namespaces SET directory_count = directory_count + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_directory_heads_account_delete AFTER DELETE ON kv_directory_heads
BEGIN
    UPDATE kv_namespaces SET directory_count = directory_count - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_directory_heads_account_update AFTER UPDATE OF uid ON kv_directory_heads
BEGIN
    UPDATE kv_namespaces SET directory_count = directory_count - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET directory_count = directory_count + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_dirent_heads_account_insert AFTER INSERT ON kv_dirent_heads
BEGIN
    UPDATE kv_namespaces SET dirent_count = dirent_count + (1) WHERE namespace_id = NEW.uid;
END;

CREATE TRIGGER kv_dirent_heads_account_delete AFTER DELETE ON kv_dirent_heads
BEGIN
    UPDATE kv_namespaces SET dirent_count = dirent_count - (1) WHERE namespace_id = OLD.uid;
END;

CREATE TRIGGER kv_dirent_heads_account_update AFTER UPDATE OF uid ON kv_dirent_heads
BEGIN
    UPDATE kv_namespaces SET dirent_count = dirent_count - (1) WHERE namespace_id = OLD.uid;
    UPDATE kv_namespaces SET dirent_count = dirent_count + (1) WHERE namespace_id = NEW.uid;
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
