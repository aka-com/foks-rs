-- Internal placement identity. A legacy database must be explicitly adopted.
CREATE TABLE chunk_storage_revision (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision >= 0)
);
INSERT INTO chunk_storage_revision VALUES (1, 0);
CREATE TABLE chunk_storage_binding (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    state INTEGER NOT NULL CHECK (state IN (1,2,3,4)),
    namespace_id BLOB,
    host_id BLOB,
    locator TEXT,
    generation INTEGER NOT NULL DEFAULT 1 CHECK (generation = 1),
    scan_revision INTEGER NOT NULL DEFAULT 0 CHECK (scan_revision >= 0),
    after_uid BLOB NOT NULL DEFAULT X'',
    after_file_id BLOB NOT NULL DEFAULT X'',
    after_offset INTEGER NOT NULL DEFAULT -1,
    verified_objects INTEGER NOT NULL DEFAULT 0 CHECK (typeof(verified_objects) = 'integer' AND verified_objects >= 0),
    verified_bytes INTEGER NOT NULL DEFAULT 0 CHECK (typeof(verified_bytes) = 'integer' AND verified_bytes >= 0),
    CHECK ((state = 1 AND namespace_id IS NULL AND host_id IS NULL AND locator IS NULL)
        OR (state IN (2,3,4) AND namespace_id IS NOT NULL AND host_id IS NOT NULL AND length(namespace_id) = 16 AND length(host_id) = 33
            AND locator IS NOT NULL AND length(locator) BETWEEN 1 AND 4096)),
    CHECK ((length(after_uid) = 0 AND length(after_file_id) = 0 AND after_offset = -1)
        OR (length(after_uid) = 33 AND length(after_file_id) = 16 AND after_offset >= 0))
);
INSERT INTO chunk_storage_binding(singleton,state)
    SELECT 1,1 WHERE EXISTS(SELECT 1 FROM kv_file_chunks WHERE external_digest IS NOT NULL);
CREATE TRIGGER chunk_storage_revision_insert AFTER INSERT ON kv_file_chunks
WHEN NEW.external_digest IS NOT NULL BEGIN
    UPDATE chunk_storage_revision SET revision = revision + 1 WHERE singleton = 1;
END;
CREATE TRIGGER chunk_storage_revision_delete AFTER DELETE ON kv_file_chunks
WHEN OLD.external_digest IS NOT NULL BEGIN
    UPDATE chunk_storage_revision SET revision = revision + 1 WHERE singleton = 1;
END;
CREATE TRIGGER chunk_storage_revision_update AFTER UPDATE ON kv_file_chunks
WHEN OLD.external_digest IS NOT NULL OR NEW.external_digest IS NOT NULL BEGIN
    UPDATE chunk_storage_revision SET revision = revision + 1 WHERE singleton = 1;
END;
