mod common;

use foks_server_db::{Config, Database, Error, KvDirectoryMutation, KvNodeMutation, ReadDatabase};
use rusqlite::{params, Connection};

const UID: [u8; 33] = [1; 33];

#[test]
fn native_writers_invalidate_snapshots_and_known_identity_replay_does_not_reexecute() {
    let mut f = common::TestDatabase::new();
    let empty = f.database.read_dependency().unwrap();
    f.reserve(1_000_000);
    assert_eq!(f.database.read_dependency().unwrap(), empty); // reservation only
    f.commit(None).unwrap();
    let identity = f.database.read_dependency().unwrap();
    assert_eq!(identity.incarnation, empty.incarnation);
    assert!(identity.revision > empty.revision);
    f.commit(None).unwrap();
    assert_eq!(f.database.read_dependency().unwrap(), identity);
    f.database
        .record_certificate(&[1], &UID, &[4; 33], &[4; 33], 1, 100, b"exact-certificate")
        .unwrap();
    let cert = f.database.read_dependency().unwrap();
    assert!(cert.revision > identity.revision);

    let reader = ReadDatabase::open(&f.path, Config::default()).unwrap();
    let snapshot = reader.snapshot().unwrap();
    snapshot.require_read_dependency(&cert).unwrap();
    let directory = KvDirectoryMutation {
        uid: &UID,
        id: &[1; 16],
        version: 1,
        key_role: 3,
        key_visibility: 0,
        key_generation: 1,
        status: 0,
        exact: b"directory",
    };
    f.database.put_kv_directory(&directory).unwrap();
    // Existing snapshot has its own valid earlier linearization point. A new
    // page transaction must compare the original bundle coordinate again.
    snapshot.require_read_dependency(&cert).unwrap();
    drop(snapshot);
    assert!(matches!(
        reader.snapshot().unwrap().require_read_dependency(&cert),
        Err(Error::ReadDependencyChanged)
    ));
    let directory_coord = f.database.read_dependency().unwrap();
    f.database.put_kv_directory(&directory).unwrap();
    // Native directory replay still executes the namespace-admission UPSERT.
    let replayed = f.database.read_dependency().unwrap();
    assert_eq!(replayed.incarnation, directory_coord.incarnation);
    assert!(replayed.revision > directory_coord.revision);
    let mut id = [7; 17];
    id[0] = 3;
    let exact = common::small_box(&[7; 32]);
    let before_node = f.database.read_dependency().unwrap();
    f.database
        .put_kv_node(&KvNodeMutation {
            uid: &UID,
            id: &id,
            node_type: 3,
            exact: &exact,
        })
        .unwrap();
    assert!(f.database.read_dependency().unwrap().revision > before_node.revision);
}

#[test]
fn saved_schema_proof_refuses_ddl_and_new_opener_checks_existing_invariants() {
    let f = common::TestDatabase::new();
    let reader = ReadDatabase::open(&f.path, Default::default()).unwrap();
    let old = f.database.read_dependency().unwrap();
    let sql = Connection::open(&f.path).unwrap();
    sql.execute_batch("DROP TRIGGER kv_nodes_usage_delete;")
        .unwrap();
    assert!(matches!(
        reader.read_dependency(),
        Err(Error::ReadDependencySchema(_))
    ));
    assert!(matches!(
        f.database.read_dependency(),
        Err(Error::ReadDependencySchema(_))
    ));
    assert!(matches!(
        ReadDatabase::open(&f.path, Default::default()),
        Err(Error::ReadDependencySchema(_))
    ));
    assert!(matches!(
        Database::open_existing(&f.path, Default::default()),
        Err(Error::ReadDependencySchema(_))
    ));
    let actual: i64 = sql
        .query_row("SELECT revision FROM metadata_read_dependency", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(actual as u64, old.revision); // cookie, not artificial revision bump
}

#[test]
fn private_restore_rotation_preserves_public_records_and_is_cas_bound() {
    let mut f = common::TestDatabase::new();
    f.reserve(1_000_000);
    f.commit(None).unwrap();
    let prior = f.database.read_dependency().unwrap();
    let identity = f.database.identity(&UID).unwrap().unwrap();
    let root = f.database.current_root().unwrap().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let copy = temp.path().join("private.sqlite");
    f.database.online_backup(&copy).unwrap();
    let mut restored = Database::open(&copy, Default::default()).unwrap();
    assert_eq!(restored.read_dependency().unwrap(), prior);
    let fresh = restored.rotate_private_read_dependency(&prior).unwrap();
    assert_ne!(fresh.incarnation, prior.incarnation);
    assert_eq!(fresh.revision, 1);
    assert!(matches!(
        restored.rotate_private_read_dependency(&prior),
        Err(Error::ReadDependencyChanged)
    ));
    assert_eq!(restored.read_dependency().unwrap(), fresh);
    assert_eq!(f.database.read_dependency().unwrap(), prior);
    assert_eq!(
        restored.identity(&UID).unwrap().unwrap().exact_link,
        identity.exact_link
    );
    assert_eq!(
        restored.current_root().unwrap().unwrap().exact_signed_root,
        root.exact_signed_root
    );
    drop(restored);
    assert_eq!(
        Database::open_existing(&copy, Default::default())
            .unwrap()
            .read_dependency()
            .unwrap(),
        fresh
    );
}

#[test]
fn genuine_58_opener_upgrade_generates_coordinate_and_readers_refuse_predecessor() {
    let mut f = common::TestDatabase::new();
    f.reserve(1_000_000);
    f.commit(None).unwrap();
    let expected = f.database.identity(&UID).unwrap().unwrap().exact_link;
    let c = Connection::open(&f.path).unwrap();
    c.execute_batch(concat!(
        include_str!("fixtures/remove_waitlist_count.sql"),
        include_str!("fixtures/remove_read_dependency.sql")
    ))
    .unwrap();
    c.pragma_update(None, "user_version", 58).unwrap();
    assert!(matches!(
        ReadDatabase::open(&f.path, Default::default()),
        Err(Error::SchemaVersion { found: 58 })
    ));
    let upgraded = Database::open(&f.path, Default::default()).unwrap();
    let coordinate = upgraded.read_dependency().unwrap();
    assert_eq!(coordinate.revision, 1);
    assert_eq!(
        upgraded.identity(&UID).unwrap().unwrap().exact_link,
        expected
    );
    assert_eq!(
        upgraded.read_dependency_schema().contract_hash(),
        f.database.read_dependency_schema().contract_hash()
    );
    assert!(matches!(
        f.database.read_dependency(),
        Err(Error::ReadDependencySchema(_))
    ));
}

#[test]
fn actual_authorization_dml_cannot_aba_without_revision_change() {
    let mut f = common::TestDatabase::new();
    f.reserve(1_000_000);
    f.commit(None).unwrap();
    let old = f.database.read_dependency().unwrap();
    let c = Connection::open(&f.path).unwrap();
    c.execute("UPDATE devices SET active=0 WHERE uid=?1", [UID.as_slice()])
        .unwrap();
    c.execute("UPDATE devices SET active=1 WHERE uid=?1", [UID.as_slice()])
        .unwrap();
    let now = f.database.read_dependency().unwrap();
    assert!(now.revision > old.revision);
    let tx = c.unchecked_transaction().unwrap();
    tx.execute(
        "UPDATE devices SET role_type=1,visibility=10,subkey_id=?1 WHERE uid=?2",
        params![[9u8; 33].as_slice(), UID.as_slice()],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO shared_keys VALUES(?1,1,10,2,?2,X'01')",
        params![UID.as_slice(), [14u8; 33].as_slice()],
    )
    .unwrap();
    tx.rollback().unwrap();
    assert_eq!(f.database.read_dependency().unwrap(), now);
    f.database.run_maintenance(1_000_000, 0).unwrap();
    assert_eq!(f.database.read_dependency().unwrap(), now); // bookkeeping-only sweep
}

#[test]
fn typed_multistatement_identity_failure_at_revision_limit_rolls_back_prior_statements() {
    let mut f = common::TestDatabase::new();
    f.reserve(1_000_000);
    let sql = Connection::open(&f.path).unwrap();
    sql.execute(
        "UPDATE metadata_read_dependency SET revision=?1",
        [i64::MAX - 1],
    )
    .unwrap();
    let prior = f.database.read_dependency().unwrap();
    assert!(f.commit(None).is_err());
    assert_eq!(f.database.read_dependency().unwrap(), prior);
    assert!(f.database.identity(&UID).unwrap().is_none());
    assert!(f.database.current_root().unwrap().is_none());
    assert_eq!(
        sql.query_row("SELECT count(*) FROM kv_namespaces", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
