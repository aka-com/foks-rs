-- Provenance is recorded with admission, before any physical publication.
-- These are policies, not termination certificates. In particular a successful
-- retrying remote call does not prove that all earlier PUTs have terminated.
ALTER TABLE chunk_object_attempts ADD COLUMN publication_policy INTEGER NOT NULL
 DEFAULT 0 CHECK(publication_policy IN(0,1,2));
-- 0 unknown/legacy, 1 synchronous filesystem v1, 2 retrying S3 v1.
CREATE INDEX chunk_attempts_publication_policy
 ON chunk_object_attempts(publication_policy,outcome);
CREATE TRIGGER chunk_attempt_policy_immutable
 BEFORE UPDATE OF publication_policy ON chunk_object_attempts
 WHEN NEW.publication_policy != OLD.publication_policy BEGIN
 SELECT RAISE(ABORT,'publication provenance is immutable');
END;
