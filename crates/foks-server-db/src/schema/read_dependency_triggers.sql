-- Generated fixed hook manifest; matches READ_DEPENDENCY_TABLES. No connection-local functions.
CREATE TRIGGER metadata_read_issued_certificates_insert AFTER INSERT ON issued_certificates BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_issued_certificates_update AFTER UPDATE ON issued_certificates BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_issued_certificates_delete AFTER DELETE ON issued_certificates BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_devices_insert AFTER INSERT ON devices BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_devices_update AFTER UPDATE ON devices BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_devices_delete AFTER DELETE ON devices BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_shared_keys_insert AFTER INSERT ON shared_keys BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_shared_keys_update AFTER UPDATE ON shared_keys BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_shared_keys_delete AFTER DELETE ON shared_keys BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_sso_policy_insert AFTER INSERT ON sso_policy BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_sso_policy_update AFTER UPDATE ON sso_policy BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_sso_policy_delete AFTER DELETE ON sso_policy BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_users_insert AFTER INSERT ON users BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_users_update AFTER UPDATE ON users BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_users_delete AFTER DELETE ON users BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_user_chain_heads_insert AFTER INSERT ON user_chain_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_user_chain_heads_update AFTER UPDATE ON user_chain_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_user_chain_heads_delete AFTER DELETE ON user_chain_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_tree_locations_insert AFTER INSERT ON tree_locations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_tree_locations_update AFTER UPDATE ON tree_locations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_tree_locations_delete AFTER DELETE ON tree_locations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_merkle_root_heads_insert AFTER INSERT ON merkle_root_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_merkle_root_heads_update AFTER UPDATE ON merkle_root_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_merkle_root_heads_delete AFTER DELETE ON merkle_root_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_merkle_roots_insert AFTER INSERT ON merkle_roots BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_merkle_roots_update AFTER UPDATE ON merkle_roots BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_merkle_roots_delete AFTER DELETE ON merkle_roots BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_namespaces_insert AFTER INSERT ON kv_namespaces BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_namespaces_update AFTER UPDATE ON kv_namespaces BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_namespaces_delete AFTER DELETE ON kv_namespaces BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_roots_insert AFTER INSERT ON kv_roots BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_roots_update AFTER UPDATE ON kv_roots BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_roots_delete AFTER DELETE ON kv_roots BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_directories_insert AFTER INSERT ON kv_directories BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_directories_update AFTER UPDATE ON kv_directories BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_directories_delete AFTER DELETE ON kv_directories BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_directory_heads_insert AFTER INSERT ON kv_directory_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_directory_heads_update AFTER UPDATE ON kv_directory_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_directory_heads_delete AFTER DELETE ON kv_directory_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_nodes_insert AFTER INSERT ON kv_nodes BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_nodes_update AFTER UPDATE ON kv_nodes BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_nodes_delete AFTER DELETE ON kv_nodes BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirents_insert AFTER INSERT ON kv_dirents BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirents_update AFTER UPDATE ON kv_dirents BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirents_delete AFTER DELETE ON kv_dirents BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirent_heads_insert AFTER INSERT ON kv_dirent_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirent_heads_update AFTER UPDATE ON kv_dirent_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirent_heads_delete AFTER DELETE ON kv_dirent_heads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirent_current_insert AFTER INSERT ON kv_dirent_current BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirent_current_update AFTER UPDATE ON kv_dirent_current BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_dirent_current_delete AFTER DELETE ON kv_dirent_current BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_file_uploads_insert AFTER INSERT ON kv_file_uploads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_file_uploads_update AFTER UPDATE ON kv_file_uploads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_file_uploads_delete AFTER DELETE ON kv_file_uploads BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_file_chunks_insert AFTER INSERT ON kv_file_chunks BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_file_chunks_update AFTER UPDATE ON kv_file_chunks BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_kv_file_chunks_delete AFTER DELETE ON kv_file_chunks BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_chunk_objects_insert AFTER INSERT ON chunk_objects BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_chunk_objects_update AFTER UPDATE ON chunk_objects BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_chunk_objects_delete AFTER DELETE ON chunk_objects BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_chunk_storage_binding_insert AFTER INSERT ON chunk_storage_binding BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_chunk_storage_binding_update AFTER UPDATE ON chunk_storage_binding BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_chunk_storage_binding_delete AFTER DELETE ON chunk_storage_binding BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_host_metadata_insert AFTER INSERT ON host_metadata BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_host_metadata_update AFTER UPDATE ON host_metadata BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_host_metadata_delete AFTER DELETE ON host_metadata BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_host_key_generations_insert AFTER INSERT ON host_key_generations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_host_key_generations_update AFTER UPDATE ON host_key_generations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_host_key_generations_delete AFTER DELETE ON host_key_generations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_capability_key_generations_insert AFTER INSERT ON capability_key_generations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_capability_key_generations_update AFTER UPDATE ON capability_key_generations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER metadata_read_capability_key_generations_delete AFTER DELETE ON capability_key_generations BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM metadata_read_dependency WHERE singleton=1
        AND typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)
        AND typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775806)
        THEN RAISE(ABORT,'read dependency row missing, invalid or exhausted') END;
    UPDATE metadata_read_dependency SET revision=revision+1 WHERE singleton=1;
END;
