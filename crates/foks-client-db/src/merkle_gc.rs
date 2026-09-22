//! Reference-aware collection for flat trusted-local checkpoints.
use crate::*;
use std::collections::{BTreeMap, BTreeSet};

/// A root named by an external rollback checkpoint that collection must retain.
#[derive(Clone, Debug)]
pub struct MerkleRootPin {
    pub host_id: Vec<u8>,
    pub epoch: u64,
    pub root_hash: [u8; 32],
}

impl HardStateStore {
    /// Cheap admission gate; automatic collection starts above 128 roots.
    pub fn needs_merkle_compaction(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT count(*) > 128 FROM (SELECT 1 FROM merkle_roots LIMIT 129)",
            [],
            |r| r.get(0),
        )?)
    }

    /// Collects unreferenced roots in one write transaction. The caller MUST
    /// exclude concurrent profile operations and account for private resumable
    /// material outside SQLite before calling. External checkpoint pins must
    /// describe the last durable publication, not a pending in-memory update.
    /// Unfinished SQLite workflows defer collection, returning zero.
    pub fn compact_merkle_roots(&mut self, pins: &[MerkleRootPin]) -> Result<usize> {
        let tx = self.write_transaction()?;
        // Protected request bodies can cite roots not visible in public journal
        // columns. Keep all roots until every owning workflow is terminal.
        for sql in [
            "SELECT EXISTS(SELECT 1 FROM import_readiness WHERE required=1)",
            "SELECT EXISTS(SELECT 1 FROM signup_operations WHERE state!=3)",
            "SELECT EXISTS(SELECT 1 FROM adhoc_team_operations WHERE state!=3)",
            "SELECT EXISTS(SELECT 1 FROM team_mutation_operations WHERE state NOT IN (5,6,7))",
            "SELECT EXISTS(SELECT 1 FROM mutation_operations WHERE state NOT IN (5,6))",
            "SELECT EXISTS(SELECT 1 FROM federation_saga_operations WHERE state!=3)",
            "SELECT EXISTS(SELECT 1 FROM chat_operations WHERE state IN (0,1))",
            "SELECT EXISTS(SELECT 1 FROM sso_flows WHERE state IN (0,1,2,3,7))",
            "SELECT EXISTS(SELECT 1 FROM kv_adapter_submissions WHERE state=0)",
        ] {
            if tx.query_row(sql, [], |r| r.get::<_, bool>(0))? {
                return Ok(0);
            }
        }
        let mut live = BTreeSet::<(Vec<u8>, u64)>::new();
        let roots = tx
            .prepare("SELECT host_id, epoch, root_hash FROM merkle_roots")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })?
            .map(|row| {
                let (host, epoch, hash) = row?;
                Ok((
                    (host, stored_unsigned("Merkle epoch", epoch)?),
                    fixed_hash(hash, "invalid root hash")?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        for pin in pins {
            let key = (pin.host_id.clone(), pin.epoch);
            if roots.get(&key) != Some(&pin.root_hash) {
                return Err(Error::InvalidSnapshot(
                    "external Merkle checkpoint is absent",
                ));
            }
            live.insert(key);
        }
        // Legacy recursive evidence has additional dependencies; only flat
        // databases can be collected, even if someone bypassed normal opening.
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM merkle_heads WHERE evidence_kind!=4)",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::InvalidSnapshot(
                "Merkle collection requires flat checkpoints",
            ));
        }
        for row in tx
            .prepare("SELECT evidence_kind, anchor_epoch, evidence_bytes FROM merkle_heads")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })?
        {
            let (kind, anchor, bytes) = row?;
            if !matches!(
                decode_evidence(kind, anchor, bytes)?,
                MerkleRootEvidence::LocalCheckpoint { .. }
            ) {
                return Err(Error::InvalidSnapshot("invalid local Merkle checkpoint"));
            }
        }
        for table in ["merkle_heads", "users", "teams", "user_generic_chains"] {
            let column = if table == "merkle_heads" {
                "epoch"
            } else {
                "merkle_epoch"
            };
            let hash_column = if table == "merkle_heads" {
                "root_hash"
            } else {
                "merkle_root_hash"
            };
            for row in tx
                .prepare(&format!(
                    "SELECT host_id, {column}, {hash_column} FROM {table}"
                ))?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                })?
            {
                let (host, epoch, hash) = row?;
                let key = (host, stored_unsigned("referenced Merkle epoch", epoch)?);
                if roots.get(&key) != Some(&fixed_hash(hash, "invalid referenced root hash")?) {
                    return Err(Error::InvalidSnapshot(
                        "referenced Merkle root hash differs",
                    ));
                }
                live.insert(key);
            }
        }
        for (table, extract) in [
            (
                "users",
                foks_verify::user_evidence_root_epochs
                    as fn(&[u8]) -> foks_verify::Result<Vec<u64>>,
            ),
            ("teams", foks_verify::team_evidence_root_epochs),
        ] {
            for row in tx
                .prepare(&format!("SELECT host_id, evidence_bytes FROM {table}"))?
                .query_map([], |r| {
                    Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
                })?
            {
                let (host, bytes) = row?;
                let epochs = extract(&bytes)
                    .map_err(|_| Error::InvalidSnapshot("invalid Merkle collection evidence"))?;
                live.extend(epochs.into_iter().map(|epoch| (host.clone(), epoch)));
            }
        }
        for row in tx
            .prepare("SELECT host_id, chain_bytes FROM user_generic_chains")?
            .query_map([], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
        {
            let (host, bytes) = row?;
            let Value::Array(fields) = decode(&bytes)? else {
                return Err(Error::InvalidSnapshot("invalid generic chain"));
            };
            let Some((Value::Unsigned(2 | 4), links)) = fields.split_first() else {
                return Err(Error::InvalidSnapshot("invalid generic chain type"));
            };
            for link in links {
                let link = foks_proto::UserLink::decode(&encode(link)?)
                    .map_err(|_| Error::InvalidSnapshot("invalid generic link"))?;
                let link = link
                    .decode_generic()
                    .map_err(|_| Error::InvalidSnapshot("invalid generic payload"))?;
                live.insert((host.clone(), link.root.epoch));
            }
        }
        // Import completion markers lack host IDs. Preserve their epochs on every host
        // where present (active import verification already deferred above).
        for epoch in tx.prepare("SELECT verified_merkle_epoch FROM import_accounts WHERE verified_merkle_epoch IS NOT NULL")?
            .query_map([], |r| r.get::<_, i64>(0))? {
            let epoch = stored_unsigned("import Merkle epoch", epoch?)?;
            live.extend(roots.keys().filter(|(_, e)| *e == epoch).cloned());
        }
        if live.iter().any(|key| !roots.contains_key(key)) {
            return Err(Error::InvalidSnapshot("referenced Merkle root is absent"));
        }
        let mut removed = 0;
        for (host, epoch) in roots.keys().filter(|key| !live.contains(*key)) {
            removed += tx.execute(
                "DELETE FROM merkle_roots WHERE host_id=?1 AND epoch=?2",
                params![host, sqlite_integer("Merkle epoch", *epoch)?],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }
}
