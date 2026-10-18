-- Derived live heads only. Historical rows remain authoritative and retained.
CREATE TABLE kv_dirent_current (
    uid BLOB NOT NULL CHECK (length(uid) = 33),
    parent_id BLOB NOT NULL CHECK (length(parent_id) = 16),
    dirent_id BLOB NOT NULL CHECK (length(dirent_id) = 16),
    version INTEGER NOT NULL CHECK (version >= 1),
    name_mac BLOB NOT NULL CHECK (length(name_mac) = 32),
    creation_time INTEGER NOT NULL CHECK (creation_time >= 0),
    node_id BLOB NOT NULL CHECK (length(node_id) = 17 AND substr(node_id, 1, 1) != X'00'),
    PRIMARY KEY (uid, parent_id, dirent_id),
    FOREIGN KEY (uid, parent_id, dirent_id, version)
        REFERENCES kv_dirents(uid, parent_id, dirent_id, version) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX kv_dirent_current_name ON kv_dirent_current(uid, parent_id, name_mac, dirent_id);
CREATE INDEX kv_dirent_current_time ON kv_dirent_current(uid, parent_id, creation_time, dirent_id);

INSERT INTO kv_dirent_current
SELECT d.uid, d.parent_id, d.dirent_id, d.version, d.name_mac, d.creation_time, d.node_id
FROM kv_dirent_heads h JOIN kv_dirents d
  ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
WHERE substr(d.node_id, 1, 1) != X'00';

CREATE TRIGGER kv_current_head_insert AFTER INSERT ON kv_dirent_heads
BEGIN
    -- REPLACE need not fire the implicit old-head DELETE trigger when
    -- recursive_triggers is disabled. Clear its derived row explicitly.
    DELETE FROM kv_dirent_current WHERE uid=NEW.uid AND parent_id=NEW.parent_id AND dirent_id=NEW.dirent_id;
    INSERT OR REPLACE INTO kv_dirent_current
    SELECT d.uid, d.parent_id, d.dirent_id, d.version, d.name_mac, d.creation_time, d.node_id
    FROM kv_dirent_heads h JOIN kv_dirents d
      ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
    WHERE h.uid=NEW.uid AND h.parent_id=NEW.parent_id AND h.dirent_id=NEW.dirent_id
      AND substr(d.node_id, 1, 1) != X'00';
END;
CREATE TRIGGER kv_current_head_update AFTER UPDATE ON kv_dirent_heads
BEGIN
    DELETE FROM kv_dirent_current WHERE uid=OLD.uid AND parent_id=OLD.parent_id AND dirent_id=OLD.dirent_id;
    INSERT OR REPLACE INTO kv_dirent_current
    SELECT d.uid, d.parent_id, d.dirent_id, d.version, d.name_mac, d.creation_time, d.node_id
    FROM kv_dirent_heads h JOIN kv_dirents d
      ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
    WHERE h.uid=NEW.uid AND h.parent_id=NEW.parent_id AND h.dirent_id=NEW.dirent_id
      AND substr(d.node_id, 1, 1) != X'00';
END;
CREATE TRIGGER kv_current_head_delete AFTER DELETE ON kv_dirent_heads
BEGIN
    DELETE FROM kv_dirent_current WHERE uid=OLD.uid AND parent_id=OLD.parent_id AND dirent_id=OLD.dirent_id;
END;
CREATE TRIGGER kv_current_history_insert AFTER INSERT ON kv_dirents
BEGIN
    INSERT OR REPLACE INTO kv_dirent_current
    SELECT d.uid, d.parent_id, d.dirent_id, d.version, d.name_mac, d.creation_time, d.node_id
    FROM kv_dirent_heads h JOIN kv_dirents d
      ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
    WHERE h.uid=NEW.uid AND h.parent_id=NEW.parent_id AND h.dirent_id=NEW.dirent_id AND h.version=NEW.version
      AND substr(d.node_id, 1, 1) != X'00';
END;
-- Old-version edits cannot delete a newer projection. Rebuild both affected
-- head keys so deferred primary-key/head relocation works in either order.
CREATE TRIGGER kv_current_history_update
AFTER UPDATE OF uid, parent_id, dirent_id, version, name_mac, creation_time, node_id ON kv_dirents
BEGIN
    DELETE FROM kv_dirent_current WHERE uid=OLD.uid AND parent_id=OLD.parent_id AND dirent_id=OLD.dirent_id AND version=OLD.version;
    INSERT OR REPLACE INTO kv_dirent_current
    SELECT d.uid, d.parent_id, d.dirent_id, d.version, d.name_mac, d.creation_time, d.node_id
    FROM kv_dirent_heads h JOIN kv_dirents d
      ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
    WHERE h.uid=OLD.uid AND h.parent_id=OLD.parent_id AND h.dirent_id=OLD.dirent_id
      AND substr(d.node_id, 1, 1) != X'00';
    INSERT OR REPLACE INTO kv_dirent_current
    SELECT d.uid, d.parent_id, d.dirent_id, d.version, d.name_mac, d.creation_time, d.node_id
    FROM kv_dirent_heads h JOIN kv_dirents d
      ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version
    WHERE h.uid=NEW.uid AND h.parent_id=NEW.parent_id AND h.dirent_id=NEW.dirent_id
      AND substr(d.node_id, 1, 1) != X'00';
END;
