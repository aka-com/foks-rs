-- Remove schema58 semantic Usage before constructing a genuine predecessor.
DROP TRIGGER kv_nodes_usage_exact;
DROP TRIGGER kv_nodes_usage_collision;
DROP TRIGGER kv_nodes_usage_identity;
DROP TRIGGER kv_uploads_usage_collision;
DROP TRIGGER kv_uploads_usage_identity;
DROP TRIGGER kv_nodes_usage_insert;
DROP TRIGGER kv_nodes_usage_delete;
DROP TRIGGER kv_nodes_usage_update;
DROP TRIGGER kv_uploads_usage_insert;
DROP TRIGGER kv_uploads_usage_delete;
DROP TRIGGER kv_uploads_usage_update;
ALTER TABLE kv_nodes DROP COLUMN ciphertext_bytes;
ALTER TABLE kv_namespaces DROP COLUMN usage_small_count;
ALTER TABLE kv_namespaces DROP COLUMN usage_small_bytes;
ALTER TABLE kv_namespaces DROP COLUMN usage_large_count;
ALTER TABLE kv_namespaces DROP COLUMN usage_large_bytes;
ALTER TABLE kv_namespaces DROP COLUMN usage_large_chunks;
ALTER TABLE kv_namespaces DROP COLUMN usage_unvalidated_small_count;
