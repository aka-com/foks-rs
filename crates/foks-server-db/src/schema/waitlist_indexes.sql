-- Shared by fresh schema creation and the atomic version-49 writer upgrade.
-- One entry per address under ASCII case folding: a repeated join returns the
-- stored identifier instead of adding a row.
CREATE UNIQUE INDEX waitlist_entries_email
    ON waitlist_entries(email COLLATE NOCASE);
CREATE INDEX waitlist_entries_created ON waitlist_entries(created_at);
