ALTER TABLE chunk_storage_binding ADD COLUMN authority_identity BLOB
    CHECK (authority_identity IS NULL OR (typeof(authority_identity)='blob' AND length(authority_identity)=32 AND state IN (2,4)));
