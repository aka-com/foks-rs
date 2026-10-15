//! Offline collection intents. Backend termination proof and installation
//! exclusion belong to the caller; a timeout never releases physical capacity.
use crate::{
    allocation::RowAllocationExt as _, chunk_ledger::ChunkObject, Database, Error, Result,
};
use rusqlite::{params, TransactionBehavior};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkDeletion {
    pub object: ChunkObject,
    pub token: [u8; 16],
    pub generation: u64,
}
impl Database {
    pub fn chunk_collection_candidates(&self) -> Result<u64> {
        let count:i64=self.connection.query_row("SELECT count(*) FROM chunk_objects o WHERE o.state IN(3,4) AND o.digest IS NOT NULL AND o.references_count=0 AND o.namespace_id=(SELECT namespace_id FROM chunk_storage_binding WHERE state=4) AND NOT EXISTS(SELECT 1 FROM chunk_object_attempts a WHERE a.object_id=o.object_id AND a.outcome IN(1,2))",[],|r|r.budgeted_get(0))?;
        crate::error::unsigned(count)
    }
    pub fn begin_chunk_collection(
        &mut self,
        batch_token: &[u8; 16],
        maximum_objects: usize,
        maximum_bytes: u64,
    ) -> Result<Vec<ChunkDeletion>> {
        if !(1..=128).contains(&maximum_objects) || maximum_bytes == 0 {
            return Err(Error::Invalid("collection batch bounds"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ready:bool=tx.query_row("SELECT inventory_complete=1 AND inventory_running=0 FROM chunk_object_totals WHERE singleton=1",[],|r|r.budgeted_get(0))?;
        if !ready {
            return Err(Error::ChunkAdmission(
                "collection requires complete inventory",
            ));
        }
        let mut result = Vec::new();
        let mut bytes = 0u64;
        {
            let mut q=tx.prepare("SELECT o.object_id,o.namespace_id,o.object_key,o.key_format,o.digest,o.encoded_bytes,o.state,o.delete_token,o.delete_generation FROM chunk_objects o WHERE o.state IN(3,4) AND o.digest IS NOT NULL AND o.references_count=0 AND o.namespace_id=(SELECT namespace_id FROM chunk_storage_binding WHERE state=4) AND NOT EXISTS(SELECT 1 FROM chunk_object_attempts a WHERE a.object_id=o.object_id AND a.outcome IN(1,2)) ORDER BY o.state DESC,o.object_id LIMIT ?1")?;
            let mut rows = q.query([maximum_objects as i64])?;
            while let Some(row) = rows.next()? {
                let id: Vec<u8> = row.budgeted_get(0)?;
                let ns: Vec<u8> = row.budgeted_get(1)?;
                let digest: Vec<u8> = row.budgeted_get(4)?;
                let object = ChunkObject {
                    object_id: id
                        .try_into()
                        .map_err(|_| Error::Invalid("collection object ID"))?,
                    namespace_id: Some(
                        ns.try_into()
                            .map_err(|_| Error::Invalid("collection namespace"))?,
                    ),
                    object_key: row.budgeted_get(2)?,
                    key_format: crate::error::unsigned(row.budgeted_get(3)?)?,
                    digest: digest
                        .try_into()
                        .map_err(|_| Error::Invalid("collection digest"))?,
                    encoded_bytes: crate::error::unsigned(row.budgeted_get(5)?)?,
                };
                object.validate_key()?;
                let next = bytes
                    .checked_add(object.encoded_bytes)
                    .ok_or(Error::IntegerRange)?;
                if next > maximum_bytes {
                    continue;
                }
                bytes = next;
                let state: i64 = row.budgeted_get(6)?;
                let generation = crate::error::unsigned(row.budgeted_get(8)?)?;
                let (token, generation) = if state == 4 {
                    let token: Vec<u8> = row.budgeted_get(7)?;
                    (
                        token
                            .try_into()
                            .map_err(|_| Error::Invalid("collection token"))?,
                        generation,
                    )
                } else {
                    use sha2::{Digest, Sha256};
                    let mut h = Sha256::new();
                    h.update(b"foks-delete-v1\0");
                    h.update(batch_token);
                    h.update(object.object_id);
                    let hash: [u8; 32] = h.finalize().into();
                    (
                        hash[..16].try_into().expect("fixed width"),
                        generation.checked_add(1).ok_or(Error::IntegerRange)?,
                    )
                };
                result.push(ChunkDeletion {
                    object,
                    token,
                    generation,
                });
            }
        }
        for deletion in &result {
            tx.execute("UPDATE chunk_objects SET state=4,delete_token=?2,delete_generation=?3 WHERE object_id=?1 AND state IN(3,4)",params![deletion.object.object_id.as_slice(),deletion.token.as_slice(),crate::error::sql_integer(deletion.generation)?])?;
        }
        tx.commit()?;
        Ok(result)
    }
    pub fn finish_chunk_collection(&mut self, deletion: &ChunkDeletion) -> Result<()> {
        deletion.object.validate_key()?;
        let ns = deletion
            .object
            .namespace_id
            .ok_or(Error::Invalid("missing collection namespace"))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE chunk_objects SET state=5 WHERE object_id=?1 AND namespace_id=?2 AND object_key=?3 AND digest=?4 AND encoded_bytes=?5 AND delete_token=?6 AND delete_generation=?7 AND state IN(4,5) AND references_count=0 AND namespace_id=(SELECT namespace_id FROM chunk_storage_binding WHERE state=4) AND NOT EXISTS(SELECT 1 FROM chunk_object_attempts a WHERE a.object_id=chunk_objects.object_id AND a.outcome IN(1,2))",params![deletion.object.object_id.as_slice(),ns.as_slice(),deletion.object.object_key,deletion.object.digest.as_slice(),crate::error::sql_integer(deletion.object.encoded_bytes)?,deletion.token.as_slice(),crate::error::sql_integer(deletion.generation)?])?;
        if changed != 1 {
            return Err(Error::Invalid("collection intent changed"));
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(dir.path().join("db"), Default::default()).unwrap();
        db.connection
            .execute(
                "INSERT INTO host_metadata VALUES(1,?1,'test',X'',X'')",
                [[4u8; 33].as_slice()],
            )
            .unwrap();
        let binding = db
            .prepare_chunk_storage_binding(&[5; 16], "test-store", false)
            .unwrap();
        db.activate_chunk_storage_binding(&binding).unwrap();
        db.confirm_empty_chunk_inventory().unwrap();
        (dir, db)
    }
    #[test]
    fn durable_intent_survives_restart_and_late_delete_cannot_target_new_incarnation() {
        let (dir, mut db) = fixture();
        let old = db.reserve_chunk_object(&[1; 32], 10, &[1; 16], 1).unwrap();
        db.settle_chunk_publication(&old, true).unwrap();
        let intents = db.begin_chunk_collection(&[7; 16], 128, 1024).unwrap();
        assert_eq!(intents.len(), 1);
        let usage = db.chunk_physical_usage().unwrap();
        assert_eq!(
            (usage.present_bytes, usage.deleting_bytes, usage.objects),
            (10, 10, 1)
        );
        assert!(db.validate_chunk_reservation(&old).is_err());
        drop(db);
        let mut db = Database::open(dir.path().join("db"), Default::default()).unwrap();
        assert_eq!(
            db.begin_chunk_collection(&[8; 16], 128, 1024).unwrap(),
            intents
        );
        let new = db.reserve_chunk_object(&[1; 32], 10, &[2; 16], 2).unwrap();
        assert_ne!(new.object_id, old.object_id);
        assert_ne!(new.object_key, old.object_key);
        db.settle_chunk_publication(&new, true).unwrap();
        let mut stale = intents[0].clone();
        stale.token = [0; 16];
        assert!(db.finish_chunk_collection(&stale).is_err());
        db.finish_chunk_collection(&intents[0]).unwrap();
        db.finish_chunk_collection(&intents[0]).unwrap();
        db.validate_chunk_reservation(&new).unwrap();
        assert_eq!(
            crate::chunk_ledger::require_present(&db.connection, &[1; 32], 10).unwrap(),
            new.object_id
        );
        let usage = db.chunk_physical_usage().unwrap();
        assert_eq!(
            (
                usage.present_bytes,
                usage.deleting_bytes,
                usage.objects,
                usage.tombstones
            ),
            (10, 0, 1, 1)
        );
    }
    #[test]
    fn references_unknown_inventory_and_ambiguous_publishers_are_never_eligible() {
        let (_dir, mut db) = fixture();
        let uncertain = db.reserve_chunk_object(&[1; 32], 10, &[1; 16], 1).unwrap();
        db.settle_chunk_publication(&uncertain, false).unwrap();
        db.confirm_chunk_presence(&uncertain).unwrap();
        let referenced = db.reserve_chunk_object(&[2; 32], 10, &[2; 16], 1).unwrap();
        db.settle_chunk_publication(&referenced, true).unwrap();
        db.connection
            .execute(
                "INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(?1,1,?1)",
                [[1u8; 33].as_slice()],
            )
            .unwrap();
        db.connection.execute("INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(?1,?2,X'',0,0)",params![[1u8;33].as_slice(),[2u8;16].as_slice()]).unwrap();
        db.connection.execute("INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk,external_digest,external_ciphertext_bytes,external_encoded_bytes,external_object_id) VALUES(?1,?2,0,X'',0,X'',?3,1,10,?4)",params![[1u8;33].as_slice(),[2u8;16].as_slice(),referenced.digest.as_slice(),referenced.object_id.as_slice()]).unwrap();
        let cp = db.begin_chunk_inventory(false).unwrap();
        db.record_chunk_inventory_page(
            &cp,
            &[
                crate::ChunkInventoryEntry {
                    key: uncertain.object_key,
                    encoded_bytes: 10,
                },
                crate::ChunkInventoryEntry {
                    key: referenced.object_key,
                    encoded_bytes: 10,
                },
                crate::ChunkInventoryEntry {
                    key: "unknown".into(),
                    encoded_bytes: 100,
                },
            ],
            None,
        )
        .unwrap();
        assert_eq!(db.chunk_collection_candidates().unwrap(), 0);
        assert!(db
            .begin_chunk_collection(&[3; 16], 128, 1024)
            .unwrap()
            .is_empty());
        db.connection
            .execute("DELETE FROM kv_namespaces", [])
            .unwrap();
        assert_eq!(db.chunk_collection_candidates().unwrap(), 1);
        let intents = db.begin_chunk_collection(&[3; 16], 128, 1024).unwrap();
        assert_eq!(intents[0].object.object_id, referenced.object_id);
        db.finish_chunk_collection(&intents[0]).unwrap();
        let usage = db.chunk_physical_usage().unwrap();
        assert_eq!(
            (
                usage.present_bytes,
                usage.unresolved_attempts,
                usage.unknown_objects
            ),
            (110, 1, 1)
        );
    }
}
