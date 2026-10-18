-- Older predecessors also predate the schema57 historical-name index.
DROP INDEX IF EXISTS kv_dirent_historical_name;
-- Reconstruct the schema55 predecessor before testing a real upgrade.
DROP TRIGGER kv_current_head_insert;
DROP TRIGGER kv_current_head_update;
DROP TRIGGER kv_current_head_delete;
DROP TRIGGER kv_current_history_insert;
DROP TRIGGER kv_current_history_update;
DROP TABLE kv_dirent_current;
