//! Conservative physical accounting. No timeout, missing object, or metadata
//! deletion releases capacity. This local ledger is not a distributed fence.
use crate::allocation::RowAllocationExt as _;
use crate::{
    error::{sql_integer, unsigned},
    Database, Error, ReadDatabase, Result,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

type StoredChunkAdmission = (Vec<u8>, Option<Vec<u8>>, i64, i64, String, i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkReservationAction {
    Publish,
    Verify,
    Present,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkReservation {
    pub object_id: [u8; 32],
    pub object_key: String,
    pub key_format: u64,
    pub namespace_id: [u8; 16],
    pub digest: [u8; 32],
    pub encoded_bytes: u64,
    pub attempt_id: Option<[u8; 16]>,
    pub action: ChunkReservationAction,
}
impl ChunkReservation {
    pub fn object(&self) -> ChunkObject {
        ChunkObject {
            object_id: self.object_id,
            namespace_id: Some(self.namespace_id),
            object_key: self.object_key.clone(),
            key_format: self.key_format,
            digest: self.digest,
            encoded_bytes: self.encoded_bytes,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkObject {
    pub object_id: [u8; 32],
    pub namespace_id: Option<[u8; 16]>,
    pub object_key: String,
    pub key_format: u64,
    pub digest: [u8; 32],
    pub encoded_bytes: u64,
}
impl ChunkObject {
    pub fn legacy(digest: [u8; 32], encoded_bytes: u64) -> Self {
        Self {
            object_id: digest,
            namespace_id: None,
            object_key: object_key(&digest),
            key_format: 1,
            digest,
            encoded_bytes,
        }
    }
    pub fn validate_key(&self) -> Result<()> {
        let expected = match self.key_format {
            1 => object_key(&self.digest),
            2 => format!("v2-{}", object_key(&self.object_id)),
            _ => return Err(Error::Invalid("unknown physical key format")),
        };
        if self.object_key != expected {
            return Err(Error::Invalid("physical key does not match incarnation"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkPhysicalUsage {
    pub present_bytes: u64,
    pub reserved_bytes: u64,
    pub uncertain_bytes: u64,
    pub objects: u64,
    pub unresolved_attempts: u64,
    pub admission_rejections: u64,
    pub inventory_complete: bool,
    pub orphan_objects: u64,
    pub orphan_bytes: u64,
    pub unknown_objects: u64,
    pub unknown_bytes: u64,
    pub missing_objects: u64,
    pub missing_referenced_objects: u64,
    pub oldest_attempt_micros: Option<u64>,
    pub deleting_bytes: u64,
    pub deleting_objects: u64,
    pub tombstones: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkInventoryCheckpoint {
    pub generation: u64,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkInventoryEntry {
    pub key: String,
    pub encoded_bytes: u64,
}

fn namespace(c: &Connection) -> Result<[u8; 16]> {
    let bytes: Option<Vec<u8>> = c
        .query_row(
            "SELECT namespace_id FROM chunk_storage_binding WHERE singleton=1 AND state=4",
            [],
            |r| r.budgeted_get(0),
        )
        .optional()?
        .flatten();
    bytes
        .ok_or(Error::Invalid(
            "physical accounting requires active chunk binding",
        ))?
        .try_into()
        .map_err(|_| Error::Invalid("chunk namespace ID"))
}
fn object_key(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
fn usage(c: &Connection) -> Result<ChunkPhysicalUsage> {
    let mut s=c.prepare("SELECT present_bytes,reserved_bytes,uncertain_bytes,objects,unresolved_attempts,inventory_complete,inventory_generation,admission_rejections FROM chunk_object_totals WHERE singleton=1")?;
    let mut rows = s.query([])?;
    let r = rows
        .next()?
        .ok_or(Error::Invalid("missing physical counters"))?;
    let mut value = ChunkPhysicalUsage {
        present_bytes: unsigned(r.budgeted_get(0)?)?,
        reserved_bytes: unsigned(r.budgeted_get(1)?)?,
        uncertain_bytes: unsigned(r.budgeted_get(2)?)?,
        objects: unsigned(r.budgeted_get(3)?)?,
        unresolved_attempts: unsigned(r.budgeted_get(4)?)?,
        inventory_complete: r.budgeted_get(5)?,
        ..Default::default()
    };
    let generation: i64 = r.budgeted_get(6)?;
    value.admission_rejections = unsigned(r.budgeted_get(7)?)?;
    let (objects,bytes,unknown):(i64,i64,i64)=c.query_row("SELECT count(*),coalesce(sum(encoded_bytes),0),coalesce(sum(digest IS NULL),0) FROM chunk_objects WHERE references_count=0 AND state!=5",[],|r|Ok((r.budgeted_get(0)?,r.budgeted_get(1)?,r.budgeted_get(2)?)))?;
    value.orphan_objects = unsigned(objects)?;
    value.orphan_bytes = unsigned(bytes)?;
    value.unknown_objects = unsigned(unknown)?;
    let (unknown_bytes,missing):(i64,i64)=c.query_row("SELECT coalesce(sum(CASE WHEN digest IS NULL THEN encoded_bytes ELSE 0 END),0),coalesce(sum(inventory_seen!=?1),0) FROM chunk_objects WHERE state!=5",[generation],|r|Ok((r.budgeted_get(0)?,r.budgeted_get(1)?)))?;
    value.unknown_bytes = unsigned(unknown_bytes)?;
    value.missing_objects = unsigned(missing)?;
    let missing: i64 = c.query_row(
        "SELECT count(*) FROM chunk_objects WHERE references_count>0 AND inventory_seen!=?1",
        [generation],
        |r| r.budgeted_get(0),
    )?;
    value.missing_referenced_objects = unsigned(missing)?;
    let oldest: Option<i64> = c.query_row(
        "SELECT min(created_at) FROM chunk_object_attempts WHERE outcome IN(1,2)",
        [],
        |r| r.budgeted_get(0),
    )?;
    value.oldest_attempt_micros = oldest.map(unsigned).transpose()?;
    let (bytes,objects,tombstones):(i64,i64,i64)=c.query_row("SELECT coalesce(sum(CASE WHEN state=4 THEN encoded_bytes ELSE 0 END),0),coalesce(sum(state=4),0),coalesce(sum(state=5),0) FROM chunk_objects",[],|r|Ok((r.budgeted_get(0)?,r.budgeted_get(1)?,r.budgeted_get(2)?)))?;
    value.deleting_bytes = unsigned(bytes)?;
    value.deleting_objects = unsigned(objects)?;
    value.tombstones = unsigned(tombstones)?;
    Ok(value)
}
impl ReadDatabase {
    pub fn chunk_physical_usage(&self) -> Result<ChunkPhysicalUsage> {
        usage(&self.connection)
    }
}
impl Database {
    pub fn chunk_inventory_empty_candidate(&self) -> Result<bool> {
        Ok(self.connection.query_row("SELECT inventory_complete=0 AND objects=0 AND unresolved_attempts=0 AND inventory_running=0 FROM chunk_object_totals WHERE singleton=1",[],|r|r.budgeted_get(0))?)
    }
    pub fn chunk_physical_usage(&self) -> Result<ChunkPhysicalUsage> {
        usage(&self.connection)
    }
    /// On a new owner, an admitted request may already have reached the store.
    pub fn recover_chunk_publications(&mut self) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE chunk_object_attempts SET outcome=2 WHERE outcome=1",
            [],
        )?;
        tx.execute("UPDATE chunk_objects SET state=2 WHERE state=1", [])?;
        tx.commit()?;
        Ok(())
    }
    pub fn reserve_chunk_object(
        &mut self,
        digest: &[u8; 32],
        encoded_bytes: u64,
        attempt_id: &[u8; 16],
        now: u64,
    ) -> Result<ChunkReservation> {
        if !(1..=9438208).contains(&encoded_bytes) {
            return Err(Error::Invalid("physical object size"));
        }
        let limits = self.config.clone();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ns = namespace(&tx)?;
        let totals:(i64,i64,i64,i64,i64,bool)=tx.query_row("SELECT present_bytes,reserved_bytes,uncertain_bytes,objects,unresolved_attempts,inventory_complete FROM chunk_object_totals WHERE singleton=1",[],|r|Ok((r.budgeted_get(0)?,r.budgeted_get(1)?,r.budgeted_get(2)?,r.budgeted_get(3)?,r.budgeted_get(4)?,r.budgeted_get(5)?)))?;
        if !totals.5 {
            return reject(tx, "chunk inventory is incomplete");
        }
        let stored:Option<StoredChunkAdmission>=tx.query_row("SELECT object_id,namespace_id,encoded_bytes,state,object_key,key_format FROM chunk_objects WHERE digest=?1 AND state IN(1,2,3)",[digest.as_slice()],|r|Ok((r.budgeted_get(0)?,r.budgeted_get(1)?,r.budgeted_get(2)?,r.budgeted_get(3)?,r.budgeted_get(4)?,r.budgeted_get(5)?))).optional()?;
        if let Some((id, stored_ns, size, state, key, key_format)) = stored {
            if stored_ns.as_deref() != Some(ns.as_slice()) || unsigned(size)? != encoded_bytes {
                return Err(Error::Invalid("physical object binding mismatch"));
            }
            let action = match state {
                3 => ChunkReservationAction::Present,
                1 | 2 => ChunkReservationAction::Verify,
                _ => return Err(Error::Invalid("physical object state")),
            };
            let result = ChunkReservation {
                object_id: id.try_into().map_err(|_| Error::Invalid("object ID"))?,
                object_key: key,
                key_format: unsigned(key_format)?,
                namespace_id: ns,
                digest: *digest,
                encoded_bytes,
                attempt_id: None,
                action,
            };
            tx.commit()?;
            return Ok(result);
        }
        let total = unsigned(totals.0)?
            .checked_add(unsigned(totals.1)?)
            .and_then(|x| x.checked_add(unsigned(totals.2).ok()?))
            .and_then(|x| x.checked_add(encoded_bytes))
            .ok_or(Error::IntegerRange)?;
        let possible_uncertain = unsigned(totals.1)?
            .checked_add(unsigned(totals.2)?)
            .and_then(|x| x.checked_add(encoded_bytes))
            .ok_or(Error::IntegerRange)?;
        if total > limits.maximum_chunk_physical_bytes
            || unsigned(totals.3)? >= limits.maximum_chunk_objects
            || unsigned(totals.4)? >= limits.maximum_chunk_unresolved_attempts
            || possible_uncertain > limits.maximum_chunk_uncertain_bytes
        {
            return reject(tx, "physical chunk capacity exhausted");
        }
        // Random admission IDs generate non-reusable physical incarnations. The
        // hash is internal naming only, not client cryptography.
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"foks-object-incarnation-v2\0");
        h.update(ns);
        h.update(attempt_id);
        let object_id: [u8; 32] = h.finalize().into();
        let key = format!("v2-{}", object_key(&object_id));
        tx.execute("INSERT INTO chunk_objects(object_id,namespace_id,object_key,digest,encoded_bytes,key_format,state,inventory_seen) VALUES(?1,?2,?3,?4,?5,2,1,(SELECT inventory_generation FROM chunk_object_totals WHERE singleton=1))",params![object_id.as_slice(),ns.as_slice(),key,digest.as_slice(),sql_integer(encoded_bytes)?])?;
        tx.execute("INSERT INTO chunk_object_attempts(attempt_id,object_id,namespace_id,outcome,created_at) VALUES(?1,?2,?3,1,?4)",params![attempt_id.as_slice(),object_id.as_slice(),ns.as_slice(),sql_integer(now)?])?;
        tx.commit()?;
        Ok(ChunkReservation {
            object_id,
            object_key: key,
            key_format: 2,
            namespace_id: ns,
            digest: *digest,
            encoded_bytes,
            attempt_id: Some(*attempt_id),
            action: ChunkReservationAction::Publish,
        })
    }
    pub fn settle_chunk_publication(
        &mut self,
        token: &ChunkReservation,
        success: bool,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_token(&tx, token, false)?;
        let attempt = token
            .attempt_id
            .ok_or(Error::Invalid("missing publication attempt"))?;
        let changed=tx.execute("UPDATE chunk_object_attempts SET outcome=?1 WHERE attempt_id=?2 AND object_id=?3 AND namespace_id=?4 AND outcome=1",params![if success{3}else{2},attempt.as_slice(),token.object_id.as_slice(),token.namespace_id.as_slice()])?;
        if changed != 1 {
            return Err(Error::Invalid("publication attempt changed"));
        }
        tx.execute("UPDATE chunk_objects SET state=CASE WHEN state=3 THEN 3 ELSE ?1 END WHERE object_id=?2",params![if success{3}else{2},token.object_id.as_slice()])?;
        tx.commit()?;
        Ok(())
    }
    /// A matching GET establishes presence, but does not terminate a prior PUT.
    pub fn confirm_chunk_presence(&mut self, token: &ChunkReservation) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_token(&tx, token, false)?;
        tx.execute(
            "UPDATE chunk_objects SET state=3,digest=?1 WHERE object_id=?2",
            params![token.digest.as_slice(), token.object_id.as_slice()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn validate_chunk_reservation(&self, token: &ChunkReservation) -> Result<()> {
        validate_token(&self.connection, token, true)
    }
    /// Only after a bounded listing proved empty under exclusive installation ownership.
    pub fn confirm_empty_chunk_inventory(&mut self) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let _ = namespace(&tx)?;
        let count: i64 = tx.query_row(
            "SELECT objects FROM chunk_object_totals WHERE singleton=1",
            [],
            |r| r.budgeted_get(0),
        )?;
        if count != 0 {
            return Err(Error::Invalid("nonempty ledger requires inventory"));
        }
        tx.execute("UPDATE chunk_object_totals SET inventory_complete=1,inventory_generation=inventory_generation+1,inventory_running=0,inventory_cursor=NULL WHERE singleton=1",[])?;
        tx.commit()?;
        Ok(())
    }
    pub fn begin_chunk_inventory(&mut self, restart: bool) -> Result<ChunkInventoryCheckpoint> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ns = namespace(&tx)?;
        tx.execute(
            "UPDATE chunk_objects SET namespace_id=?1 WHERE namespace_id IS NULL",
            [ns.as_slice()],
        )?;
        let mismatch: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM chunk_objects WHERE namespace_id!=?1)",
            [ns.as_slice()],
            |r| r.budgeted_get(0),
        )?;
        if mismatch {
            return Err(Error::Invalid("inventory namespace mismatch"));
        }
        let running: bool = tx.query_row(
            "SELECT inventory_running FROM chunk_object_totals WHERE singleton=1",
            [],
            |r| r.budgeted_get(0),
        )?;
        if !running || restart {
            tx.execute("UPDATE chunk_object_totals SET inventory_running=1,inventory_complete=0,inventory_generation=inventory_generation+1,inventory_cursor=NULL WHERE singleton=1",[])?;
        }
        let result = inventory_checkpoint(&tx)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn record_chunk_inventory_page(
        &mut self,
        expected: &ChunkInventoryCheckpoint,
        entries: &[ChunkInventoryEntry],
        next: Option<&str>,
    ) -> Result<ChunkInventoryCheckpoint> {
        if entries.len() > 256 || next.is_some_and(|s| s.len() > 4096) {
            return Err(Error::Invalid("inventory page bounds"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ns = namespace(&tx)?;
        if inventory_checkpoint(&tx)? != *expected {
            return Err(Error::Invalid("inventory cursor changed"));
        }
        for entry in entries {
            if entry.key.is_empty() || entry.key.len() > 1024 || entry.key == "_foks_namespace_v1" {
                return Err(Error::Invalid("inventory object key"));
            }
            let stored: Option<(Vec<u8>, i64, i64)> = tx
                .query_row(
                    "SELECT object_id,encoded_bytes,state FROM chunk_objects WHERE object_key=?1",
                    [&entry.key],
                    |r| Ok((r.budgeted_get(0)?, r.budgeted_get(1)?, r.budgeted_get(2)?)),
                )
                .optional()?;
            if let Some((id, size, state)) = stored {
                if state == 5 {
                    return Err(Error::Invalid(
                        "retired object reappeared; collection safety violated",
                    ));
                }
                if unsigned(size)? != entry.encoded_bytes {
                    return Err(Error::Invalid("inventory object length mismatch"));
                }
                tx.execute(
                    "UPDATE chunk_objects SET inventory_seen=?1 WHERE object_id=?2",
                    params![sql_integer(expected.generation)?, id],
                )?;
            } else {
                use sha2::{Digest, Sha256};
                let mut h = Sha256::new();
                h.update(b"foks-unknown-inventory-v1\0");
                h.update(entry.key.as_bytes());
                let id: [u8; 32] = h.finalize().into();
                tx.execute("INSERT INTO chunk_objects(object_id,namespace_id,object_key,encoded_bytes,state,inventory_seen) VALUES(?1,?2,?3,?4,3,?5)",params![id.as_slice(),ns.as_slice(),entry.key,sql_integer(entry.encoded_bytes)?,sql_integer(expected.generation)?])?;
            }
        }
        tx.execute(
            "UPDATE chunk_object_totals SET inventory_cursor=?1 WHERE singleton=1",
            [next],
        )?;
        if next.is_none() {
            let missing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM chunk_objects WHERE references_count>0 AND inventory_seen!=?1)",[sql_integer(expected.generation)?],|r|r.budgeted_get(0))?;
            if missing {
                return Err(Error::Invalid("inventory is missing referenced objects"));
            }
            tx.execute("UPDATE chunk_object_totals SET inventory_complete=1,inventory_running=0 WHERE singleton=1",[])?;
        }
        let result = ChunkInventoryCheckpoint {
            generation: expected.generation,
            cursor: next.map(str::to_owned),
        };
        tx.commit()?;
        Ok(result)
    }
    pub(crate) fn clear_private_chunk_ledger(&mut self) -> Result<()> {
        self.connection
            .execute("DELETE FROM chunk_object_attempts", [])?;
        self.connection.execute("DELETE FROM chunk_objects", [])?;
        self.connection.execute("UPDATE chunk_object_totals SET inventory_complete=0,inventory_running=0,inventory_cursor=NULL WHERE singleton=1",[])?;
        Ok(())
    }
}
fn inventory_checkpoint(c: &Connection) -> Result<ChunkInventoryCheckpoint> {
    let (generation,cursor,running):(i64,Option<String>,bool)=c.query_row("SELECT inventory_generation,inventory_cursor,inventory_running FROM chunk_object_totals WHERE singleton=1",[],|r|Ok((r.budgeted_get(0)?,r.budgeted_get(1)?,r.budgeted_get(2)?)))?;
    if !running {
        return Err(Error::Invalid("inventory is not running"));
    }
    Ok(ChunkInventoryCheckpoint {
        generation: unsigned(generation)?,
        cursor,
    })
}
fn validate_token(c: &Connection, t: &ChunkReservation, present: bool) -> Result<()> {
    t.object().validate_key()?;
    if namespace(c)? != t.namespace_id {
        return Err(Error::Invalid("publication namespace changed"));
    }
    let row:Option<(Option<Vec<u8>>,i64,i64,String)>=c.query_row("SELECT digest,encoded_bytes,state,object_key FROM chunk_objects WHERE object_id=?1 AND namespace_id=?2",params![t.object_id.as_slice(),t.namespace_id.as_slice()],|r|Ok((r.budgeted_get(0)?,r.budgeted_get(1)?,r.budgeted_get(2)?,r.budgeted_get(3)?))).optional()?;
    if !row.is_some_and(|(digest, len, state, key)| {
        len >= 0
            && len as u64 == t.encoded_bytes
            && digest.as_deref().is_none_or(|d| d == t.digest)
            && key == t.object_key
            && matches!(state, 1..=3)
            && (!present || state == 3 && digest.is_some())
    }) {
        return Err(Error::Invalid("publication token mismatch"));
    }
    Ok(())
}
/// Used inside the same transaction that creates an external reference.
pub(crate) fn require_present(c: &Connection, digest: &[u8; 32], length: u64) -> Result<[u8; 32]> {
    let ns = namespace(c)?;
    let id:Option<Vec<u8>>=c.query_row("SELECT object_id FROM chunk_objects WHERE digest=?1 AND namespace_id=?2 AND encoded_bytes=?3 AND state IN(1,2,3) AND state=3",params![digest.as_slice(),ns.as_slice(),sql_integer(length)?],|r|r.budgeted_get(0)).optional()?;
    id.ok_or(Error::Invalid("external object is not durably accounted"))?
        .try_into()
        .map_err(|_| Error::Invalid("object ID"))
}

fn reject<T>(tx: rusqlite::Transaction<'_>, reason: &'static str) -> Result<T> {
    tx.execute("UPDATE chunk_object_totals SET admission_rejections=admission_rejections+1 WHERE singleton=1",[])?;
    tx.commit()?;
    Err(Error::ChunkAdmission(reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(config: crate::Config) -> (tempfile::TempDir, Database) {
        let temp = tempfile::tempdir().unwrap();
        let mut db = Database::open(temp.path().join("db"), config).unwrap();
        db.connection
            .execute(
                "INSERT INTO host_metadata VALUES(1,?1,'test',X'',X'')",
                [[4u8; 33].as_slice()],
            )
            .unwrap();
        let binding = db
            .prepare_chunk_storage_binding(&[5; 16], "store", false)
            .unwrap();
        db.activate_chunk_storage_binding(&binding).unwrap();
        db.confirm_empty_chunk_inventory().unwrap();
        (temp, db)
    }
    fn reference(db: &Database, digest: &[u8; 32]) {
        db.connection
            .execute(
                "INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(?1,1,?1)",
                [[1u8; 33].as_slice()],
            )
            .unwrap();
        db.connection.execute("INSERT INTO kv_file_uploads(uid,file_id,exact_metadata,created_at,updated_at) VALUES(?1,?2,X'',0,0)",params![[1u8;33].as_slice(),[2u8;16].as_slice()]).unwrap();
        db.connection.execute("INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk,external_digest,external_ciphertext_bytes,external_encoded_bytes,external_object_id) VALUES(?1,?2,0,X'',0,X'',?3,1,10,(SELECT object_id FROM chunk_objects WHERE digest=?3 AND state=3))",params![[1u8;33].as_slice(),[2u8;16].as_slice(),digest.as_slice()]).unwrap();
    }
    #[test]
    fn ambiguity_stays_charged_and_exact_positive_reconciliation_does_not_free_attempt() {
        let (_tmp, mut db) = fixture(crate::Config {
            maximum_chunk_unresolved_attempts: 1,
            ..Default::default()
        });
        let token = db.reserve_chunk_object(&[1; 32], 10, &[1; 16], 1).unwrap();
        assert_eq!(db.chunk_physical_usage().unwrap().reserved_bytes, 10);
        assert_eq!(
            db.reserve_chunk_object(&[1; 32], 10, &[2; 16], 2)
                .unwrap()
                .action,
            ChunkReservationAction::Verify
        );
        db.settle_chunk_publication(&token, false).unwrap();
        let retry = db.reserve_chunk_object(&[1; 32], 10, &[3; 16], 3).unwrap();
        assert_eq!(retry.action, ChunkReservationAction::Verify);
        let usage = db.chunk_physical_usage().unwrap();
        assert_eq!(
            (
                usage.uncertain_bytes,
                usage.objects,
                usage.unresolved_attempts
            ),
            (10, 1, 1)
        );
        db.confirm_chunk_presence(&retry).unwrap();
        let usage = db.chunk_physical_usage().unwrap();
        assert_eq!(
            (
                usage.present_bytes,
                usage.uncertain_bytes,
                usage.unresolved_attempts
            ),
            (10, 0, 1)
        );
        assert!(matches!(
            db.reserve_chunk_object(&[2; 32], 10, &[4; 16], 4),
            Err(Error::ChunkAdmission(_))
        ));
        let again = db.reserve_chunk_object(&[1; 32], 10, &[5; 16], 5).unwrap();
        assert_eq!(again.action, ChunkReservationAction::Present);
        assert_eq!(db.chunk_physical_usage().unwrap().admission_rejections, 1);
    }
    #[test]
    fn success_and_cascade_preserve_physical_bytes_while_refcounts_change() {
        let (_tmp, mut db) = fixture(crate::Config::default());
        let token = db.reserve_chunk_object(&[1; 32], 10, &[1; 16], 1).unwrap();
        db.settle_chunk_publication(&token, true).unwrap();
        reference(&db, &[1; 32]);
        let usage = db.chunk_physical_usage().unwrap();
        assert_eq!(
            (
                usage.present_bytes,
                usage.orphan_objects,
                usage.unresolved_attempts
            ),
            (10, 0, 0)
        );
        db.connection
            .execute("DELETE FROM kv_namespaces", [])
            .unwrap();
        let usage = db.chunk_physical_usage().unwrap();
        assert_eq!(
            (
                usage.present_bytes,
                usage.orphan_objects,
                usage.orphan_bytes
            ),
            (10, 1, 10)
        );
        let refs: i64 = db
            .connection
            .query_row("SELECT references_count FROM chunk_objects", [], |r| {
                r.budgeted_get(0)
            })
            .unwrap();
        assert_eq!(refs, 0);
    }
    #[test]
    fn restart_and_all_capacity_dimensions_fail_closed_without_releasing_reservations() {
        let (tmp, mut db) = fixture(crate::Config {
            maximum_chunk_physical_bytes: 20,
            maximum_chunk_objects: 2,
            maximum_chunk_uncertain_bytes: 10,
            ..Default::default()
        });
        db.reserve_chunk_object(&[1; 32], 10, &[1; 16], 1).unwrap();
        assert!(db.reserve_chunk_object(&[2; 32], 1, &[2; 16], 2).is_err());
        drop(db);
        let mut db = Database::open(
            tmp.path().join("db"),
            crate::Config {
                maximum_chunk_physical_bytes: 10,
                ..Default::default()
            },
        )
        .unwrap();
        db.recover_chunk_publications().unwrap();
        assert_eq!(db.chunk_physical_usage().unwrap().uncertain_bytes, 10);
        assert_eq!(
            db.reserve_chunk_object(&[1; 32], 10, &[3; 16], 3)
                .unwrap()
                .action,
            ChunkReservationAction::Verify
        );
        assert!(db.reserve_chunk_object(&[2; 32], 1, &[4; 16], 4).is_err());
    }
    #[test]
    fn schema_51_backfill_requires_resumable_complete_inventory_and_counts_unknown_keys() {
        let (tmp, mut db) = fixture(crate::Config::default());
        let token = db.reserve_chunk_object(&[1; 32], 10, &[1; 16], 1).unwrap();
        db.settle_chunk_publication(&token, true).unwrap();
        reference(&db, &[1; 32]);
        db.connection
            .execute_batch(include_str!("../tests/fixtures/remove_chunk_ledger.sql"))
            .unwrap();
        db.connection
            .execute_batch("ALTER TABLE chunk_storage_binding DROP COLUMN authority_identity")
            .unwrap();
        db.connection
            .pragma_update(None, "user_version", 51)
            .unwrap();
        drop(db);
        let mut db = Database::open(tmp.path().join("db"), Default::default()).unwrap();
        assert!(!db.chunk_physical_usage().unwrap().inventory_complete);
        assert!(db.reserve_chunk_object(&[2; 32], 10, &[2; 16], 2).is_err());
        let checkpoint = db.begin_chunk_inventory(false).unwrap();
        assert!(db
            .record_chunk_inventory_page(&checkpoint, &[], None)
            .is_err());
        let next = db
            .record_chunk_inventory_page(
                &checkpoint,
                &[ChunkInventoryEntry {
                    key: object_key(&[1; 32]),
                    encoded_bytes: 10,
                }],
                Some("opaque-token"),
            )
            .unwrap();
        drop(db);
        let mut db = Database::open(tmp.path().join("db"), Default::default()).unwrap();
        assert_eq!(db.begin_chunk_inventory(false).unwrap(), next);
        db.record_chunk_inventory_page(
            &next,
            &[ChunkInventoryEntry {
                key: "old-temporary-object".into(),
                encoded_bytes: 7,
            }],
            None,
        )
        .unwrap();
        let usage = db.chunk_physical_usage().unwrap();
        assert!(usage.inventory_complete);
        assert_eq!(
            (
                usage.present_bytes,
                usage.objects,
                usage.unknown_objects,
                usage.orphan_bytes
            ),
            (17, 2, 1, 7)
        );
        let cp = db.begin_chunk_inventory(true).unwrap();
        assert_ne!(cp.generation, next.generation);
        assert!(!db.chunk_physical_usage().unwrap().inventory_complete);
    }
    #[test]
    fn conflicting_legacy_lengths_roll_back_schema_52_migration() {
        let (tmp, mut db) = fixture(Default::default());
        let token = db.reserve_chunk_object(&[1; 32], 10, &[1; 16], 1).unwrap();
        db.settle_chunk_publication(&token, true).unwrap();
        reference(&db, &[1; 32]);
        db.connection
            .execute_batch(include_str!("../tests/fixtures/remove_chunk_ledger.sql"))
            .unwrap();
        db.connection.execute("INSERT INTO kv_file_chunks(uid,file_id,clear_offset,ciphertext,final_chunk,exact_upload_chunk,external_digest,external_ciphertext_bytes,external_encoded_bytes) SELECT uid,file_id,1,ciphertext,final_chunk,exact_upload_chunk,external_digest,external_ciphertext_bytes,11 FROM kv_file_chunks", []).unwrap();
        db.connection
            .execute_batch("ALTER TABLE chunk_storage_binding DROP COLUMN authority_identity")
            .unwrap();
        db.connection
            .pragma_update(None, "user_version", 51)
            .unwrap();
        drop(db);
        assert!(Database::open(tmp.path().join("db"), Default::default()).is_err());
        let c = Connection::open(tmp.path().join("db")).unwrap();
        let version: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 51);
        let ledger: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='chunk_objects')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!ledger);
        let refs: i64 = c
            .query_row("SELECT count(*) FROM kv_file_chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(refs, 2);
    }
    #[test]
    fn object_count_limit_and_oversized_unknown_inventory_fail_closed() {
        let (_tmp, mut db) = fixture(crate::Config {
            maximum_chunk_objects: 1,
            maximum_chunk_physical_bytes: 10,
            ..Default::default()
        });
        let token = db.reserve_chunk_object(&[1; 32], 1, &[1; 16], 1).unwrap();
        db.settle_chunk_publication(&token, true).unwrap();
        assert!(matches!(
            db.reserve_chunk_object(&[2; 32], 1, &[2; 16], 2),
            Err(Error::ChunkAdmission(_))
        ));
        let cp = db.begin_chunk_inventory(false).unwrap();
        db.record_chunk_inventory_page(
            &cp,
            &[
                ChunkInventoryEntry {
                    key: token.object_key.clone(),
                    encoded_bytes: 1,
                },
                ChunkInventoryEntry {
                    key: "old-large-object".into(),
                    encoded_bytes: 100,
                },
            ],
            None,
        )
        .unwrap();
        assert_eq!(db.chunk_physical_usage().unwrap().present_bytes, 101);
        assert!(matches!(
            db.reserve_chunk_object(&[2; 32], 1, &[2; 16], 2),
            Err(Error::ChunkAdmission(_))
        ));
    }
    #[test]
    fn stale_tokens_and_duplicate_lengths_cannot_change_accounting() {
        let (_tmp, mut db) = fixture(Default::default());
        let token = db.reserve_chunk_object(&[1; 32], 10, &[1; 16], 1).unwrap();
        let mut forged = token.clone();
        forged.namespace_id = [9; 16];
        assert!(db.settle_chunk_publication(&forged, true).is_err());
        assert!(db.reserve_chunk_object(&[1; 32], 11, &[2; 16], 2).is_err());
        db.settle_chunk_publication(&token, true).unwrap();
        assert!(db.settle_chunk_publication(&token, false).is_err());
        assert_eq!(db.chunk_physical_usage().unwrap().present_bytes, 10);
    }
}
