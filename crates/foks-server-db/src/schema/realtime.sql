-- Ciphertext-only realtime state, scoped by this database's host.
CREATE TABLE rt_channel_sets (
    team_id BLOB NOT NULL REFERENCES teams(team_id),
    app_id INTEGER NOT NULL CHECK (app_id = 1),
    version INTEGER NOT NULL CHECK (version >= 1),
    mtime INTEGER NOT NULL CHECK (mtime >= 0),
    PRIMARY KEY (team_id, app_id)
) STRICT;
CREATE TABLE rt_channels (
    channel_id BLOB PRIMARY KEY CHECK (length(channel_id) = 16),
    short_id INTEGER NOT NULL UNIQUE CHECK (short_id >= 0),
    team_id BLOB NOT NULL,
    app_id INTEGER NOT NULL,
    format INTEGER NOT NULL DEFAULT 1 CHECK (format IN (1, 2)),
    metadata BLOB NOT NULL CHECK (length(metadata) <= 16384),
    -- Immutable configuration above; append activity below.
    last_message BLOB CHECK (last_message IS NULL OR length(last_message) <= 256),
    last_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_sequence >= 0),
    FOREIGN KEY (team_id, app_id) REFERENCES rt_channel_sets(team_id, app_id)
) STRICT;
CREATE INDEX rt_channels_team ON rt_channels(team_id, app_id, channel_id);
-- Format activation is intentionally excluded: extended chat starts in new channels.
CREATE TRIGGER rt_channel_format_immutable BEFORE UPDATE OF format ON rt_channels
WHEN NEW.format != OLD.format BEGIN
    SELECT RAISE(ABORT, 'realtime channel format is immutable');
END;
CREATE TABLE rt_messages (
    message_id BLOB PRIMARY KEY CHECK (length(message_id) = 16),
    channel_id BLOB NOT NULL REFERENCES rt_channels(channel_id),
    sequence INTEGER NOT NULL CHECK (sequence >= 1),
    sender BLOB NOT NULL REFERENCES users(uid),
    envelope BLOB NOT NULL CHECK (length(envelope) <= 1048576),
    exact_message BLOB NOT NULL CHECK (length(exact_message) <= 1048576),
    insert_time INTEGER NOT NULL CHECK (insert_time >= 0),
    UNIQUE (channel_id, sequence)
) STRICT;
CREATE TABLE rt_user_inboxes (
    uid BLOB NOT NULL REFERENCES users(uid),
    app_id INTEGER NOT NULL CHECK (app_id = 1),
    version INTEGER NOT NULL CHECK (version >= 0),
    reconcile_memberships BLOB CHECK (reconcile_memberships IS NULL OR length(reconcile_memberships) <= 1048576),
    reconcile_after BLOB CHECK (reconcile_after IS NULL OR length(reconcile_after) = 16),
    reconcile_dirty INTEGER NOT NULL DEFAULT 1 CHECK (reconcile_dirty IN (0,1)),
    PRIMARY KEY (uid, app_id)
) STRICT;
CREATE TABLE rt_user_channels (
    uid BLOB NOT NULL,
    app_id INTEGER NOT NULL,
    channel_id BLOB NOT NULL REFERENCES rt_channels(channel_id),
    inbox_version INTEGER NOT NULL CHECK (inbox_version >= 1),
    read_through INTEGER NOT NULL CHECK (read_through >= 0),
    accessible INTEGER NOT NULL DEFAULT 1 CHECK (accessible IN (0, 1)),
    PRIMARY KEY (uid, channel_id),
    UNIQUE (uid, app_id, inbox_version),
    FOREIGN KEY (uid, app_id) REFERENCES rt_user_inboxes(uid, app_id)
) STRICT;
