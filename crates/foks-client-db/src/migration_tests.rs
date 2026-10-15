//! Predecessor hard-state schemas rebuilt from `INITIAL` with reverse DDL,
//! and the migration of each one to the current version.
use crate::*;
use std::path::Path;

const PROBE: &[u8] =
    include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp");

/// The merkle_heads table as version 38 defined it, before evidence kind 4.
const MERKLE_HEADS_V38: &str = r#"CREATE TABLE merkle_heads (
    host_id BLOB PRIMARY KEY REFERENCES hosts(host_id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    root_hash BLOB NOT NULL CHECK (length(root_hash) = 32),
    evidence_kind INTEGER NOT NULL CHECK (evidence_kind IN (1, 2, 3)),
    anchor_epoch INTEGER CHECK (anchor_epoch IS NULL OR anchor_epoch >= 0),
    evidence_bytes BLOB NOT NULL CHECK (length(evidence_bytes) > 0),
    FOREIGN KEY (host_id, epoch) REFERENCES merkle_roots(host_id, epoch)
) STRICT, WITHOUT ROWID;"#;

const CLOCK_BOOT_ID: [u8; 16] = [5; 16];

/// A schema version this build migrates. `admission_floor` keeps the name
/// kv_adapter_clocks.validated_time_floor had before version 40, and
/// `boot_id` keeps the column version 41 added; the migration accepts each
/// combination for each version.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Predecessor {
    pub(crate) version: u32,
    pub(crate) admission_floor: bool,
    pub(crate) boot_id: bool,
}

impl Predecessor {
    fn all() -> impl Iterator<Item = Self> {
        [38, 39, 40].into_iter().flat_map(|version| {
            [(false, false), (false, true), (true, false), (true, true)]
                .into_iter()
                .map(move |(admission_floor, boot_id)| Self {
                    version,
                    admission_floor,
                    boot_id,
                })
        })
    }
}

/// Rewrites a current database into `predecessor`'s schema, keeping its rows.
/// For version 38 the caller must first store every Merkle head as legacy
/// evidence (kinds 1 to 3); the version 38 merkle_heads CHECK rejects
/// `LocalCheckpoint` rows.
pub(crate) fn rebuild_predecessor(transaction: &Transaction<'_>, predecessor: Predecessor) {
    if !predecessor.boot_id {
        transaction
            .execute_batch("ALTER TABLE kv_adapter_clocks DROP COLUMN boot_id;")
            .unwrap();
    }
    if predecessor.admission_floor {
        transaction
            .execute_batch(
                "ALTER TABLE kv_adapter_clocks
                 RENAME COLUMN validated_time_floor TO admission_floor;",
            )
            .unwrap();
    }
    if predecessor.version == 38 {
        transaction
            .execute_batch(
                "CREATE TEMP TABLE saved_heads AS SELECT * FROM merkle_heads;
                 DROP TABLE merkle_heads;",
            )
            .unwrap();
        transaction.execute_batch(MERKLE_HEADS_V38).unwrap();
        install_table_revision_triggers(transaction, "merkle_heads").unwrap();
        transaction
            .execute_batch(
                "INSERT INTO merkle_heads SELECT * FROM saved_heads;
                 DROP TABLE saved_heads;",
            )
            .unwrap();
    }
    transaction
        .pragma_update(None, "user_version", predecessor.version)
        .unwrap();
}

/// A predecessor database holding a verified host, its Merkle head and one
/// adapter clock row, with the Merkle head a current database stores for the
/// same host. Version 38 had no `LocalCheckpoint` evidence, so its head holds
/// the same signed root as legacy `SignedBootstrap` evidence.
fn predecessor_store(path: &Path, predecessor: Predecessor) -> (HardStateStore, (i64, Vec<u8>)) {
    let mut store = HardStateStore::open(path).unwrap();
    let public = foks_verify::verify_public_host("foks.app", PROBE).unwrap();
    store.accept_verified_host(&public.snapshot).unwrap();
    let current_head = merkle_head(&store.connection);
    if predecessor.version == 38 {
        let signed = merkle_checkpoint::signed_head(public.snapshot.merkle_root().evidence());
        let (kind, anchor, bytes) =
            encode_evidence(&MerkleRootEvidence::SignedBootstrap(signed.to_vec())).unwrap();
        store
            .connection
            .execute(
                "UPDATE merkle_heads SET evidence_kind=?1, anchor_epoch=?2, evidence_bytes=?3",
                params![kind, anchor, bytes],
            )
            .unwrap();
    }
    store
        .connection
        .execute(
            "INSERT INTO kv_adapter_clocks
                 (host_id, user_id, process_id, anchor_wall, anchor_monotonic,
                  validated_time_floor, reject_issued_before, boot_id)
             SELECT host_id, ?1, ?2, 200000, 1000, 199000, 198000, ?3 FROM hosts",
            params![
                [1u8; 33].as_slice(),
                [2u8; 16].as_slice(),
                CLOCK_BOOT_ID.as_slice()
            ],
        )
        .unwrap();
    let transaction = store.write_transaction().unwrap();
    rebuild_predecessor(&transaction, predecessor);
    transaction.commit().unwrap();
    (store, current_head)
}

fn merkle_head(connection: &Connection) -> (i64, Vec<u8>) {
    connection
        .query_row(
            "SELECT evidence_kind, evidence_bytes FROM merkle_heads",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

#[test]
fn frozen_merkle_heads_definition_matches_initial() {
    let start = SCHEMA.find("CREATE TABLE merkle_heads (").unwrap();
    let end = start + SCHEMA[start..].find(';').unwrap() + 1;
    assert_eq!(&SCHEMA[start..end], merkle_checkpoint::MERKLE_HEADS_V39);
}

#[test]
fn every_supported_predecessor_migrates_to_the_initial_schema() {
    let mut reference = Connection::open_in_memory().unwrap();
    initialize_or_verify(&mut reference).unwrap();
    let reference = inspection::schema(&reference).unwrap();
    for predecessor in Predecessor::all() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard.sqlite");
        let (old, current_head) = predecessor_store(&path, predecessor);
        let before = old.metadata().unwrap();
        let head = merkle_head(&old.connection);
        assert_eq!(head.0 == 1, predecessor.version == 38, "{predecessor:?}");
        // A version 39 or 40 database with validated_time_floor and boot_id
        // differs from the current schema in user_version alone.
        let schema_differs =
            predecessor.version == 38 || predecessor.admission_floor || !predecessor.boot_id;
        assert_eq!(
            inspection::schema(&old.connection).unwrap() != reference,
            schema_differs,
            "{predecessor:?}"
        );
        drop(old);

        let mut migrated = HardStateStore::open(&path).unwrap();
        let version: u32 = migrated
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION, "{predecessor:?}");
        assert_eq!(
            inspection::schema(&migrated.connection).unwrap(),
            reference,
            "{predecessor:?}"
        );
        let after = migrated.metadata().unwrap();
        assert_eq!(after.database_id, before.database_id, "{predecessor:?}");
        assert_eq!(
            merkle_head(&migrated.connection),
            current_head,
            "{predecessor:?}"
        );
        if predecessor.version == 38 {
            // The version 38 step rewrites Merkle evidence and records a write.
            assert!(after.revision > before.revision, "{predecessor:?}");
            assert_ne!(after.write_token, before.write_token, "{predecessor:?}");
        } else {
            // Column changes alone leave the rollback checkpoint valid.
            assert_eq!(after, before, "{predecessor:?}");
        }
        let clock: (i64, Option<Vec<u8>>) = migrated
            .connection
            .query_row(
                "SELECT validated_time_floor, boot_id FROM kv_adapter_clocks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let boot_id = predecessor.boot_id.then(|| CLOCK_BOOT_ID.to_vec());
        assert_eq!(clock, (199_000, boot_id), "{predecessor:?}");
        assert!(
            migrated.host_for_lookup("foks.app").unwrap().is_some(),
            "{predecessor:?}"
        );
        migrated.checkpoint_for_snapshot().unwrap();
        drop(migrated);
        HardStateStore::inspect_existing(&path).unwrap();
        // A migrated database opens as current and is not migrated again.
        let reopened = HardStateStore::open(&path).unwrap();
        assert_eq!(reopened.metadata().unwrap(), after, "{predecessor:?}");
    }
}
