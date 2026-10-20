-- Version 58: public ciphertext Usage, distinct from quota and physical bytes.
ALTER TABLE kv_nodes ADD COLUMN ciphertext_bytes INTEGER CHECK(ciphertext_bytes >= 0);
ALTER TABLE kv_namespaces ADD COLUMN usage_small_count INTEGER NOT NULL DEFAULT 0 CHECK(usage_small_count >= 0);
ALTER TABLE kv_namespaces ADD COLUMN usage_small_bytes INTEGER NOT NULL DEFAULT 0 CHECK(usage_small_bytes >= 0);
ALTER TABLE kv_namespaces ADD COLUMN usage_large_count INTEGER NOT NULL DEFAULT 0 CHECK(usage_large_count >= 0);
ALTER TABLE kv_namespaces ADD COLUMN usage_large_bytes INTEGER NOT NULL DEFAULT 0 CHECK(usage_large_bytes >= 0);
ALTER TABLE kv_namespaces ADD COLUMN usage_large_chunks INTEGER NOT NULL DEFAULT 0 CHECK(usage_large_chunks >= 0);
ALTER TABLE kv_namespaces ADD COLUMN usage_unvalidated_small_count INTEGER NOT NULL DEFAULT 0 CHECK(usage_unvalidated_small_count >= 0);
