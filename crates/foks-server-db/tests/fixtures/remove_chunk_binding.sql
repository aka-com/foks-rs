-- Test-only reconstruction of schema 50.
DROP TRIGGER IF EXISTS chunk_storage_revision_insert;
DROP TRIGGER IF EXISTS chunk_storage_revision_delete;
DROP TRIGGER IF EXISTS chunk_storage_revision_update;
DROP TABLE IF EXISTS chunk_storage_binding;
DROP TABLE IF EXISTS chunk_storage_revision;
