-- Shared by fresh schema creation and the atomic version-49 writer upgrade,
-- which runs it after removing duplicate addresses. One row holds the number
-- of waitlist entries, so the cap check of an unauthenticated join reads one
-- row instead of counting the table. The triggers keep it equal to
-- count(*) FROM waitlist_entries in the transaction of every insert and
-- delete.
CREATE TABLE waitlist_count (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 0),
    entries INTEGER NOT NULL CHECK(entries >= 0)
) STRICT;
INSERT INTO waitlist_count(singleton, entries)
    SELECT 0, count(*) FROM waitlist_entries;
CREATE TRIGGER waitlist_count_insert AFTER INSERT ON waitlist_entries BEGIN
    UPDATE waitlist_count SET entries = entries + 1 WHERE singleton = 0;
END;
CREATE TRIGGER waitlist_count_delete AFTER DELETE ON waitlist_entries BEGIN
    UPDATE waitlist_count SET entries = entries - 1 WHERE singleton = 0;
END;
