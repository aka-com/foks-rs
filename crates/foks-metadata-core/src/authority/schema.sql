-- Private qualification authority; never installed by server-db migrations.
CREATE TABLE metadata_authority (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 identity TEXT NOT NULL CHECK(length(identity)<=2048),
 activation TEXT NOT NULL CHECK(length(activation)<=2048),
 owner BLOB NOT NULL CHECK(length(owner)=16),
 epoch INTEGER NOT NULL CHECK(epoch>=1),
 revision INTEGER NOT NULL CHECK(revision>=1),
 high_water INTEGER NOT NULL CHECK(high_water>=0),
 retained_rows INTEGER NOT NULL DEFAULT 0 CHECK(retained_rows>=0 AND retained_rows<=max_rows),
 reserved_bytes INTEGER NOT NULL DEFAULT 0 CHECK(reserved_bytes>=0 AND reserved_bytes<=max_bytes),
 max_rows INTEGER NOT NULL CHECK(max_rows BETWEEN 1 AND 256),
 max_bytes INTEGER NOT NULL CHECK(max_bytes BETWEEN 4608 AND 1048576),
 clock_floor INTEGER NOT NULL CHECK(clock_floor>=0)
) STRICT;
CREATE TABLE metadata_principals (
 principal BLOB PRIMARY KEY CHECK(length(principal)=32),
 namespace BLOB NOT NULL CHECK(length(namespace)=33),
 role BLOB NOT NULL CHECK(length(role)<=64),
 enabled INTEGER NOT NULL CHECK(enabled IN (0,1))
) STRICT, WITHOUT ROWID;
CREATE TABLE metadata_intents (
 epoch INTEGER NOT NULL CHECK(epoch>=1),
 sequence INTEGER NOT NULL CHECK(sequence>=1),
 digest BLOB NOT NULL CHECK(length(digest)=32),
 principal BLOB NOT NULL CHECK(length(principal)=32),
 namespace BLOB NOT NULL CHECK(length(namespace)=33),
 revision INTEGER NOT NULL CHECK(revision>=1),
 expires INTEGER NOT NULL CHECK(expires>=0),
 terminal INTEGER NOT NULL DEFAULT 0 CHECK(terminal IN (0,1)),
 outcome BLOB CHECK(length(outcome)<=4096),
 CHECK((terminal=0 AND outcome IS NULL) OR (terminal=1 AND outcome IS NOT NULL)),
 PRIMARY KEY(epoch,sequence)
) STRICT, WITHOUT ROWID;
CREATE INDEX metadata_terminal ON metadata_intents(terminal,epoch,sequence);
CREATE TRIGGER metadata_intent_reserve AFTER INSERT ON metadata_intents BEGIN
 UPDATE metadata_authority SET retained_rows=retained_rows+1,reserved_bytes=reserved_bytes+4608 WHERE singleton=1;
END;
CREATE TRIGGER metadata_intent_release AFTER DELETE ON metadata_intents BEGIN
 UPDATE metadata_authority SET retained_rows=retained_rows-1,reserved_bytes=reserved_bytes-4608 WHERE singleton=1;
END;
