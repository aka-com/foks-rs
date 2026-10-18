CREATE TABLE waitlist_entries (
    waitlist_id BLOB PRIMARY KEY NOT NULL CHECK(length(waitlist_id) = 13),
    email TEXT NOT NULL CHECK(length(email) BETWEEN 3 AND 320),
    created_at INTEGER NOT NULL CHECK(created_at >= 0)
) STRICT;

CREATE INDEX waitlist_entries_email_created
    ON waitlist_entries(email, created_at);
