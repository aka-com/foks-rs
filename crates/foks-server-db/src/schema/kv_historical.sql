-- Version57: bound historical name selection without deleting any versions.
CREATE INDEX kv_dirent_historical_name
    ON kv_dirents(uid,parent_id,name_mac,directory_version,version DESC,dirent_id ASC);
