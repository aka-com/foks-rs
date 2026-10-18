DROP TRIGGER IF EXISTS chunk_reference_check_insert;
DROP TRIGGER IF EXISTS chunk_reference_check_update;
DROP TRIGGER IF EXISTS chunk_reference_insert;
DROP TRIGGER IF EXISTS chunk_reference_delete;
DROP TRIGGER IF EXISTS chunk_reference_update;
DROP INDEX IF EXISTS kv_chunks_object_id;
ALTER TABLE kv_file_chunks DROP COLUMN external_object_id;
DROP TABLE chunk_object_attempts;
DROP TABLE chunk_objects;
DROP TABLE chunk_object_totals;

-- Test-only reconstruction of schema 50.
DROP TRIGGER IF EXISTS chunk_storage_revision_insert;
DROP TRIGGER IF EXISTS chunk_storage_revision_delete;
DROP TRIGGER IF EXISTS chunk_storage_revision_update;
DROP TABLE IF EXISTS chunk_storage_binding;
DROP TABLE IF EXISTS chunk_storage_revision;
