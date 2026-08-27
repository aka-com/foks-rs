use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{Database, Error, Result};

type StoredBootstrap = (Vec<u8>, String, Vec<u8>, Vec<u8>);

impl Database {
    pub fn store_host_bootstrap(
        &mut self,
        host_id: &[u8],
        canonical_name: &str,
        bootstrap_blob: &[u8],
        key_manifest_blob: &[u8],
    ) -> Result<bool> {
        if host_id.len() != 33 || canonical_name.is_empty() || canonical_name.len() > 255 {
            return Err(Error::Invalid("host bootstrap"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<StoredBootstrap> = transaction
            .query_row(
                "SELECT host_id, canonical_name, bootstrap_blob, key_manifest_blob
                 FROM host_metadata WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing
                == (
                    host_id.to_vec(),
                    canonical_name.to_owned(),
                    bootstrap_blob.to_vec(),
                    key_manifest_blob.to_vec(),
                )
            {
                return Ok(false);
            }
            return Err(Error::Invalid("conflicting host bootstrap"));
        }
        transaction.execute(
            "INSERT INTO host_metadata
             (singleton, host_id, canonical_name, bootstrap_blob, key_manifest_blob)
             VALUES (1, ?1, ?2, ?3, ?4)",
            params![host_id, canonical_name, bootstrap_blob, key_manifest_blob],
        )?;
        transaction.commit()?;
        Ok(true)
    }
}
