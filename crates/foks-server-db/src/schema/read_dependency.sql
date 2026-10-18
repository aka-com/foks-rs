-- One private read coordinate per standalone tenant database, not per user.
CREATE TABLE metadata_read_dependency (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    incarnation BLOB NOT NULL CHECK(typeof(incarnation)='blob' AND length(incarnation)=32 AND incarnation!=zeroblob(32)),
    revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775807)
) STRICT;
CREATE TRIGGER metadata_read_no_replace BEFORE INSERT ON metadata_read_dependency
WHEN EXISTS(SELECT 1 FROM metadata_read_dependency) BEGIN
    SELECT RAISE(ABORT,'read dependency row cannot be replaced');
END;
CREATE TRIGGER metadata_read_no_delete BEFORE DELETE ON metadata_read_dependency BEGIN
    SELECT RAISE(ABORT,'read dependency row cannot be deleted');
END;
CREATE TRIGGER metadata_read_monotone BEFORE UPDATE ON metadata_read_dependency
WHEN NEW.singleton!=OLD.singleton OR (NEW.incarnation=OLD.incarnation AND NEW.revision<OLD.revision) BEGIN
    SELECT RAISE(ABORT,'read dependency coordinate cannot rewind');
END;
