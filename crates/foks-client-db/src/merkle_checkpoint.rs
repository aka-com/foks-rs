//! Trusted-local Merkle checkpoint storage and legacy conversion.
use crate::*;

pub(super) fn signed_head(evidence: &MerkleRootEvidence) -> &[u8] {
    match evidence {
        MerkleRootEvidence::SignedBootstrap(bytes) => bytes,
        MerkleRootEvidence::LocalCheckpoint { signed_root }
        | MerkleRootEvidence::SignedRefresh { signed_root, .. }
        | MerkleRootEvidence::SkipPath { signed_root, .. } => signed_root,
    }
}

fn invalid() -> Error {
    Error::InvalidSnapshot("invalid legacy Merkle checkpoint")
}

// Legacy evidence is a linear spine, not an arbitrary Snowpack tree. Walk it
// without recursive decoding/allocation/drop. Scalar fields still use the
// canonical decoder. Only SkipPath frames have a field after their prior.
fn legacy_head(kind: i64, anchor: Option<i64>, epoch: u64, bytes: &[u8]) -> Result<Vec<u8>> {
    let mut input = bytes;
    let mut upper = epoch;
    let mut suffixes = 0usize;
    let mut signed = None;
    let mut outer = true;
    loop {
        let (&marker, rest) = input.split_first().ok_or_else(invalid)?;
        input = rest;
        let tag = scalar(&mut input)?;
        match (marker, tag) {
            (0x92, Value::Unsigned(0)) => {
                let bootstrap = binary(&mut input)?;
                if outer {
                    if kind != 1 || anchor.is_some() {
                        return Err(invalid());
                    }
                    signed = Some(bootstrap);
                }
                break;
            }
            (0x94, Value::Unsigned(2)) => {
                let head = binary(&mut input)?;
                let prior = unsigned(&mut input)?;
                if prior >= upper {
                    return Err(invalid());
                }
                if outer {
                    if kind != 3 || anchor != Some(sqlite_integer("Merkle prior", prior)?) {
                        return Err(invalid());
                    }
                    signed = Some(head);
                }
                upper = prior;
            }
            (0x95, Value::Unsigned(1)) => {
                let prior = unsigned(&mut input)?;
                if prior >= upper {
                    return Err(invalid());
                }
                if outer && (kind != 2 || anchor != Some(sqlite_integer("Merkle anchor", prior)?)) {
                    return Err(invalid());
                }
                binary(&mut input)?; // historical response
                suffixes += 1;
                upper = prior;
            }
            _ => return Err(invalid()),
        }
        outer = false;
    }
    for index in 0..suffixes {
        let head = binary(&mut input)?;
        if kind == 2 && index + 1 == suffixes {
            signed = Some(head);
        }
    }
    if !input.is_empty() {
        return Err(invalid());
    }
    signed.ok_or_else(invalid)
}

fn scalar(input: &mut &[u8]) -> Result<Value> {
    let (value, length) = foks_snowpack::decode_prefix(input)?;
    *input = &input[length..];
    Ok(value)
}
fn binary(input: &mut &[u8]) -> Result<Vec<u8>> {
    match scalar(input)? {
        Value::Binary(bytes) if !bytes.is_empty() => Ok(bytes),
        _ => Err(invalid()),
    }
}
fn unsigned(input: &mut &[u8]) -> Result<u64> {
    match scalar(input)? {
        Value::Unsigned(value) => Ok(value),
        _ => Err(invalid()),
    }
}

pub(super) fn migrate(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another opener may have upgraded while this connection waited for the writer.
    let version: u32 = transaction.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    if version != 38 {
        return Err(invalid());
    }
    let rows = transaction.prepare(
        "SELECT host_id, epoch, root_hash, evidence_kind, anchor_epoch, evidence_bytes FROM merkle_heads"
    )?.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?,
        r.get::<_, Vec<u8>>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<i64>>(4)?,
        r.get::<_, Vec<u8>>(5)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
    let mut converted = Vec::new();
    for (host_id, epoch, hash, kind, anchor, bytes) in rows {
        let epoch_u64 = stored_unsigned("Merkle epoch", epoch)?;
        let evidence = MerkleRootEvidence::LocalCheckpoint {
            signed_root: legacy_head(kind, anchor, epoch_u64, &bytes)?,
        };
        let host = load_host_row(&transaction, &host_id)?.ok_or_else(invalid)?;
        foks_verify::restore_public_host_identity(
            &host_id,
            &host.genesis_key,
            host.chain_seqno,
            host.chain_tail_hash,
            &host.chain_bytes,
            &host.public_zone_bytes,
        )
        .map_err(|_| invalid())?;
        let roots = load_authenticated_roots(&transaction, &host_id)?;
        let head = roots
            .iter()
            .find(|r| r.epoch == epoch_u64)
            .ok_or_else(invalid)?;
        foks_verify::restore_local_merkle_checkpoint(
            VerifiedMerkleRootParts {
                epoch: epoch_u64,
                root_hash: fixed_hash(hash.clone(), "invalid Merkle head hash")?,
                root_bytes: head.root_bytes.as_deref().ok_or_else(invalid)?,
                evidence: &evidence,
                authenticated_roots: &roots,
            },
            &host.chain_bytes,
        )
        .map_err(|_| invalid())?;
        converted.push((host_id, epoch, hash, encode_evidence(&evidence)?.2));
    }
    // No table references merkle_heads. Recreate it with the new CHECK constraint
    // using the exact canonical schema (snapshot inspection compares SQL text).
    transaction.execute_batch("DROP TABLE merkle_heads")?;
    let definition = SCHEMA
        .split("CREATE TABLE merkle_heads (")
        .nth(1)
        .ok_or_else(invalid)?;
    let definition = definition.split_once(';').ok_or_else(invalid)?.0;
    transaction.execute_batch(&format!("CREATE TABLE merkle_heads ({definition};"))?;
    install_table_revision_triggers(&transaction, "merkle_heads")?;
    for (host_id, epoch, hash, bytes) in converted {
        transaction.execute(
            "INSERT INTO merkle_heads VALUES (?1, ?2, ?3, 4, NULL, ?4)",
            params![host_id, epoch, hash, bytes],
        )?;
    }
    let has_admission_floor: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info('kv_adapter_clocks')
            WHERE name='admission_floor'
        )",
        [],
        |row| row.get(0),
    )?;
    if has_admission_floor {
        transaction.execute_batch(
            "ALTER TABLE kv_adapter_clocks
             RENAME COLUMN admission_floor TO validated_time_floor;",
        )?;
    }
    transaction.execute("UPDATE hard_state_metadata SET hard_state_revision=hard_state_revision+1, write_token=randomblob(16) WHERE singleton=1", [])?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );

    fn spine(depth: u64, signed: &[u8], skip: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        for prior in (0..depth).rev() {
            if skip {
                bytes.extend([0x95, 1]);
                bytes.extend(encode(&Value::Unsigned(prior)).unwrap());
                bytes.extend(encode(&Value::Binary(vec![9])).unwrap());
            } else {
                bytes.extend([0x94, 2]);
                bytes.extend(encode(&Value::Binary(signed.to_vec())).unwrap());
                bytes.extend(encode(&Value::Unsigned(prior)).unwrap());
            }
        }
        bytes.extend([0x92, 0]);
        bytes.extend(encode(&Value::Binary(signed.to_vec())).unwrap());
        if skip {
            for _ in 0..depth {
                bytes.extend(encode(&Value::Binary(signed.to_vec())).unwrap());
            }
        }
        bytes
    }

    #[test]
    fn legacy_spine_is_iterative_and_strict_beyond_old_depth_limits() {
        for skip in [false, true] {
            let bytes = spine(6000, &[42], skip);
            let kind = if skip { 2 } else { 3 };
            assert!(decode(&bytes).is_err());
            assert_eq!(legacy_head(kind, Some(5999), 6000, &bytes).unwrap(), [42]);
            assert!(legacy_head(kind, Some(5999), 5999, &bytes).is_err());
            assert!(legacy_head(kind, Some(5998), 6000, &bytes).is_err());
            assert!(legacy_head(kind, Some(5999), 6000, &bytes[..bytes.len() - 1]).is_err());
            let mut trailing = bytes;
            trailing.push(0);
            assert!(legacy_head(kind, Some(5999), 6000, &trailing).is_err());
        }
    }

    #[test]
    fn mixed_legacy_spines_extract_the_outer_signature() {
        let bootstrap = MerkleRootEvidence::SignedBootstrap(vec![1]);
        let skip = MerkleRootEvidence::SkipPath {
            signed_root: vec![2],
            anchor_epoch: 1,
            historical_response: vec![9],
            prior: Box::new(bootstrap),
        };
        let refresh = MerkleRootEvidence::SignedRefresh {
            signed_root: vec![3],
            prior_epoch: 2,
            prior: Box::new(skip),
        };
        let (kind, anchor, bytes) = encode_evidence(&refresh).unwrap();
        assert_eq!(legacy_head(kind, anchor, 3, &bytes).unwrap(), [3]);
        let outer = MerkleRootEvidence::SkipPath {
            signed_root: vec![4],
            anchor_epoch: 3,
            historical_response: vec![9],
            prior: Box::new(refresh),
        };
        let (kind, anchor, bytes) = encode_evidence(&outer).unwrap();
        assert_eq!(legacy_head(kind, anchor, 4, &bytes).unwrap(), [4]);
    }

    fn legacy_store(path: &std::path::Path, corrupt: bool) -> HardStateStore {
        let mut store = HardStateStore::open(path).unwrap();
        let public = foks_verify::verify_public_host("foks.app", PROBE).unwrap();
        let snapshot = &public.snapshot;
        store.accept_verified_host(snapshot).unwrap();
        let root = snapshot.merkle_root();
        let mut signed = signed_head(root.evidence()).to_vec();
        if corrupt {
            signed[0] ^= 1;
        }
        let bytes = spine(root.epoch(), &signed, true);
        store
            .connection
            .execute(
                "UPDATE merkle_heads SET evidence_kind=2, anchor_epoch=?1, evidence_bytes=?2",
                params![i64::try_from(root.epoch() - 1).unwrap(), bytes],
            )
            .unwrap();
        // Recreate the actual v38 table/constraint and its canonical triggers.
        let tx = store.write_transaction().unwrap();
        tx.execute_batch(
            "CREATE TEMP TABLE saved_heads AS SELECT * FROM merkle_heads; DROP TABLE merkle_heads;",
        )
        .unwrap();
        let old = SCHEMA.replace("IN (1, 2, 3, 4)", "IN (1, 2, 3)");
        let def = old
            .split("CREATE TABLE merkle_heads (")
            .nth(1)
            .unwrap()
            .split_once(';')
            .unwrap()
            .0;
        tx.execute_batch(&format!("CREATE TABLE merkle_heads ({def};"))
            .unwrap();
        install_table_revision_triggers(&tx, "merkle_heads").unwrap();
        tx.execute_batch("INSERT INTO merkle_heads SELECT * FROM saved_heads; DROP TABLE saved_heads; PRAGMA user_version=38;").unwrap();
        tx.commit().unwrap();
        store
    }

    #[test]
    fn migrates_deep_legacy_head_and_preserves_identity_roots_and_canonical_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard.sqlite");
        let old = legacy_store(&path, false);
        let before = old.metadata().unwrap();
        let count: i64 = old
            .connection
            .query_row("SELECT count(*) FROM merkle_roots", [], |r| r.get(0))
            .unwrap();
        drop(old);
        let mut migrated = HardStateStore::open(&path).unwrap();
        let after = migrated.metadata().unwrap();
        assert_eq!(before.database_id, after.database_id);
        assert!(after.revision > before.revision);
        assert_ne!(before.write_token, after.write_token);
        let host = migrated.host_for_lookup("foks.app").unwrap().unwrap();
        let root = &host.merkle_root;
        assert_eq!(root.authenticated_roots.len() as i64, count);
        assert!(matches!(
            root.evidence,
            MerkleRootEvidence::LocalCheckpoint { .. }
        ));
        foks_verify::restore_local_merkle_checkpoint(
            VerifiedMerkleRootParts {
                epoch: root.epoch,
                root_hash: root.root_hash,
                root_bytes: &root.root_bytes,
                evidence: &root.evidence,
                authenticated_roots: &root.authenticated_roots,
            },
            &host.chain_bytes,
        )
        .unwrap();
        migrated.checkpoint_for_snapshot().unwrap();
        drop(migrated);
        HardStateStore::inspect_existing(&path).unwrap();
    }

    #[test]
    fn migration_write_failure_rolls_back_the_table_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard.sqlite");
        let old = legacy_store(&path, false);
        old.connection
            .execute(
                "UPDATE hard_state_metadata SET hard_state_revision=9223372036854775807",
                [],
            )
            .unwrap();
        let before = old.metadata().unwrap();
        assert!(HardStateStore::open(&path).is_err());
        assert_eq!(old.metadata().unwrap(), before);
        assert_eq!(
            old.connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            38
        );
        assert_eq!(
            old.connection
                .query_row("SELECT evidence_kind FROM merkle_heads", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn failed_migration_leaves_schema_evidence_and_metadata_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard.sqlite");
        let old = legacy_store(&path, true);
        let before = old.metadata().unwrap();
        assert!(HardStateStore::open(&path).is_err());
        assert_eq!(old.metadata().unwrap(), before);
        assert_eq!(
            old.connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            38
        );
        assert_eq!(
            old.connection
                .query_row("SELECT evidence_kind FROM merkle_heads", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}
