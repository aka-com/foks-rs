DROP TRIGGER waitlist_count_insert;
DROP TRIGGER waitlist_count_delete;
DROP TABLE waitlist_count;
DROP INDEX waitlist_entries_email;
DROP INDEX waitlist_entries_created;
CREATE INDEX waitlist_entries_email_created
    ON waitlist_entries(email, created_at);
