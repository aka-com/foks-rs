use super::super::{CaptureLimits, Control, RestrictedScanner};
use super::*;
use rusqlite::{params, types::Value};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    Opened,
    Copied,
    Audited,
    Keys,
}
type Action = Box<dyn FnOnce(&Path) -> Result<()>>;
thread_local! {static FAULT:std::cell::RefCell<Option<(Stage,Action)>>=const{std::cell::RefCell::new(None)};}
pub(super) fn stage(stage: Stage, path: &Path) -> Result<()> {
    let action = FAULT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|(expected, _)| *expected == stage)
        {
            slot.take().map(|(_, action)| action)
        } else {
            None
        }
    });
    match action {
        Some(action) => action(path),
        None => Ok(()),
    }
}
fn hook(stage: Stage, action: impl FnOnce(&Path) -> Result<()> + 'static) {
    FAULT.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some((stage, Box::new(action)));
    });
}
struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    database: PathBuf,
    keys: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let database = root.join("source.sqlite");
        let keys = root.join("source-keys");
        let provider = crate::keys::DirectoryKeyProvider::open(&keys, [0x35; 32]).unwrap();
        let mut writer = foks_server_db::Database::open(&database, Default::default()).unwrap();
        crate::host::load_or_bootstrap(
            &mut writer,
            &provider,
            &crate::host::BootstrapInput {
                canonical_name: "localhost".into(),
                endpoints: crate::host::BootstrapEndpoints {
                    probe: "localhost:4430".into(),
                    public_services: "localhost:4431".into(),
                    authenticated: "localhost:4432".into(),
                },
                ttl_seconds: 60,
                now_microseconds: 1000,
            },
        )
        .unwrap();
        drop(writer);
        Self {
            _directory: directory,
            root,
            database,
            keys,
        }
    }
    fn backup(&self, name: &str) -> crate::BackupArtifacts {
        crate::backup_standalone_installation(
            &self.database,
            &self.keys,
            [0x35; 32],
            self.root.join(name),
            Default::default(),
        )
        .unwrap()
    }
    fn user(&self, head: bool, role: i64, visibility: i64) -> [u8; 33] {
        let mut uid = [7; 33];
        uid[0] = foks_proto::ENTITY_USER;
        let db = Connection::open(&self.database).unwrap();
        db.execute(
            "INSERT INTO names(normalized_name,reservation_sequence,uid) VALUES(?1,1,?2)",
            params![b"alice".as_slice(), uid.as_slice()],
        )
        .unwrap();
        db.execute("INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES(?1,?2,?2,1,zeroblob(16),0)",params![uid.as_slice(),b"alice".as_slice()]).unwrap();
        db.execute("INSERT INTO shared_keys(uid,role_type,visibility,generation,verify_key,exact_hepk) VALUES(?1,?2,?3,1,?4,X'00')",params![uid.as_slice(),role,visibility,[8u8;33].as_slice()]).unwrap();
        if head {
            db.execute("INSERT INTO user_chain_links(uid,seqno,link_hash,exact_link,root_epoch) VALUES(?1,1,?2,X'00',1)",params![uid.as_slice(),[9u8;32].as_slice()]).unwrap();
            db.execute(
                "INSERT INTO user_chain_heads(uid,seqno,link_hash) VALUES(?1,1,?2)",
                params![uid.as_slice(), [9u8; 32].as_slice()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO tree_locations(uid,seqno,location) VALUES(?1,1,?2)",
                params![uid.as_slice(), [10u8; 32].as_slice()],
            )
            .unwrap();
        }
        uid
    }
}
fn control() -> ScanControl {
    ScanControl::new(
        foks_local_deadline::Deadline::capture(Instant::now() + Duration::from_secs(300)).unwrap(),
        Arc::new(AtomicBool::new(false)),
    )
}
fn complete(artifacts: &crate::BackupArtifacts, control: &ScanControl) -> StorageScanComplete {
    let mut scanner =
        RestrictedScanner::capture_finalized_backup(artifacts, CaptureLimits::default(), control)
            .unwrap();
    for _ in 0..10000 {
        if scanner.advance(control).unwrap().complete {
            return scanner.finish(control).unwrap();
        }
    }
    panic!("scan did not terminate");
}
fn run(artifacts: &crate::BackupArtifacts) -> NativePersonalListImport {
    let control = control();
    import_personal_list(
        complete(artifacts, &control),
        Zeroizing::new([0x35; 32]),
        Default::default(),
        &control,
    )
    .unwrap()
}
fn hash(path: &Path) -> [u8; 32] {
    Sha256::digest(std::fs::read(path).unwrap()).into()
}
fn edit(artifacts: &crate::BackupArtifacts, sql: &str) {
    let db = Connection::open(&artifacts.database).unwrap();
    db.pragma_update(None, "journal_mode", "DELETE").unwrap();
    db.execute_batch(sql).unwrap();
    db.close().unwrap();
}

#[test]
fn genuine_import_has_new_identity_exact_rows_and_retained_locks() {
    let f = Fixture::new();
    f.user(true, 1, i16::MIN.into());
    let artifacts = f.backup("valid");
    let before = hash(&artifacts.database);
    let a = run(&artifacts);
    let b = run(&artifacts);
    assert_eq!(hash(&artifacts.database), before);
    assert_eq!(a.qualification.rows, b.qualification.rows);
    assert_ne!(
        a.qualification.target_incarnation,
        b.qualification.target_incarnation
    );
    assert_eq!(a.qualification.authority_users, 1);
    assert!(crate::DatabaseWriterGuard::acquire(a.identity.path()).is_err());
    assert!(crate::storage_lease::StorageLease::with_store(a.identity.path(), None, true).is_err());
    let path = a.identity.path().to_owned();
    drop(a);
    assert!(!path.exists());
    b.verify_owned().unwrap();
}
#[test]
fn headless_invalid_role_is_retained_but_present_head_matches_native_role_conversion() {
    for (head, role, visibility, accepted) in [
        (false, 0, 0, true),
        (true, 0, 0, false),
        (true, 1, i16::MIN.into(), true),
        (true, 1, i16::MAX.into(), true),
        (true, 1, 32768, false),
        (true, 2, 1, false),
    ] {
        let f = Fixture::new();
        let uid = f.user(head, role, visibility);
        let reader = foks_server_db::ReadDatabase::open(&f.database, Default::default()).unwrap();
        let native = reader.user_authority(&uid).unwrap();
        let native_accept = native.as_ref().is_none_or(|a| {
            a.shared_keys
                .iter()
                .all(|k| crate::auth::team::stored_role(k.role_type, k.visibility).is_some())
        });
        assert_eq!(native_accept, accepted);
        drop(reader);
        let artifacts = f.backup("role");
        let ctl = control();
        let result = import_personal_list(
            complete(&artifacts, &ctl),
            Zeroizing::new([0x35; 32]),
            Default::default(),
            &ctl,
        );
        assert_eq!(result.is_ok(), accepted, "{head} {role} {visibility}");
    }
}
#[test]
fn hooked_key_adapters_match_native_read_oracles() {
    let f = Fixture::new();
    let artifacts = f.backup("oracles");
    let ctl = control();
    let source = complete(&artifacts, &ctl);
    let db = &source._database;
    let native =
        foks_server_db::ReadDatabase::open(&artifacts.database, Default::default()).unwrap();
    let ours = keys::bootstrap(db).unwrap();
    assert_eq!(ours, native.host_bootstrap().unwrap().unwrap());
    assert_eq!(
        keys::host_generations(db).unwrap(),
        native.host_key_generations().unwrap()
    );
    assert_eq!(
        keys::capabilities(db).unwrap(),
        native.capability_key_generations().unwrap()
    );
    assert_eq!(
        keys::rotation(db).unwrap(),
        native.active_host_rotation().unwrap()
    );
    let current = keys::root(db, None).unwrap();
    assert_eq!(current, native.current_root().unwrap().unwrap());
    assert_eq!(
        keys::root(db, Some(current.epoch)).unwrap(),
        native.roots_at(&[current.epoch]).unwrap().unwrap()[0]
    );
    let ours = crate::standalone::host_key_backup_files_from_loaded(
        keys::host_generations(db).unwrap(),
        keys::rotation(db).unwrap(),
        || Ok(keys::capabilities(db).unwrap()),
    )
    .unwrap();
    assert_eq!(
        ours,
        crate::standalone::host_key_backup_files(&native).unwrap()
    );
}
#[test]
fn exact_rowids_wr_and_sequence_exhaustion_survive_without_trigger_replay() {
    let f = Fixture::new();
    let uid = f.user(false, 1, 0);
    let db = Connection::open(&f.database).unwrap();
    db.execute(
        "UPDATE names SET rowid=?1 WHERE uid=?2",
        params![i64::MIN, uid.as_slice()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO user_local_view_permissions(viewer_uid,target_id) VALUES(?1,?2)",
        params![uid.as_slice(), [foks_proto::ENTITY_USER; 33].as_slice()],
    )
    .unwrap();
    db.execute("INSERT INTO admin_audit(event_id,host_id,action,target_id,occurred_at_us) SELECT ?1,host_id,1,X'',0 FROM host_metadata",[i64::MAX]).unwrap();
    db.execute("DELETE FROM admin_audit", []).unwrap();
    drop(db);
    let artifacts = f.backup("identities");
    let owner = run(&artifacts);
    let db = owner.database.connection();
    assert_eq!(
        db.query_row(
            "SELECT rowid FROM names WHERE uid=?1",
            [uid.as_slice()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        i64::MIN
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM user_name_history", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='admin_audit'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        i64::MAX
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM user_local_view_permissions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn wrong_keys_and_changed_private_source_never_return_owner() {
    let f = Fixture::new();
    let artifacts = f.backup("keys");
    let ctl = control();
    assert!(import_personal_list(
        complete(&artifacts, &ctl),
        Zeroizing::new([0; 32]),
        Default::default(),
        &ctl
    )
    .is_err());
    let ctl = control();
    let source = complete(&artifacts, &ctl);
    std::fs::write(source._package.database(), b"replaced").unwrap();
    assert!(
        import_personal_list(source, Zeroizing::new([0x35; 32]), Default::default(), &ctl).is_err()
    );
    let private = f.root.join("missing-wrap");
    std::fs::create_dir(&private).unwrap();
    assert!(
        crate::keys::DirectoryKeyProvider::open_existing(&private, Zeroizing::new([0x35; 32]))
            .is_err()
    );
    assert!(!private.join(crate::keys::WRAPPING_KEY_FILE).exists());
}
#[test]
fn injected_failure_at_each_owned_stage_cleans_destination_and_preserves_source() {
    let f = Fixture::new();
    let artifacts = f.backup("faults");
    let before = hash(&artifacts.database);
    for stage in [Stage::Opened, Stage::Copied, Stage::Audited, Stage::Keys] {
        let observed = Arc::new(std::sync::Mutex::new(None));
        let copy = observed.clone();
        hook(stage, move |path| {
            *copy.lock().unwrap() = Some(path.to_owned());
            Err(ScanError::Incomplete)
        });
        let ctl = control();
        assert!(import_personal_list(
            complete(&artifacts, &ctl),
            Zeroizing::new([0x35; 32]),
            Default::default(),
            &ctl
        )
        .is_err());
        assert!(!observed.lock().unwrap().as_ref().unwrap().exists());
        assert_eq!(hash(&artifacts.database), before);
    }
}
#[test]
fn cancellation_and_different_control_owner_cannot_extend_import() {
    let f = Fixture::new();
    let artifacts = f.backup("cancel");
    let ctl = control();
    let source = complete(&artifacts, &ctl);
    ctl.cancelled.store(true, Ordering::Release);
    assert!(matches!(
        import_personal_list(source, Zeroizing::new([0x35; 32]), Default::default(), &ctl),
        Err(ScanError::Cancelled)
    ));
    let ctl = control();
    let source = complete(&artifacts, &ctl);
    assert!(import_personal_list(
        source,
        Zeroizing::new([0x35; 32]),
        Default::default(),
        &control()
    )
    .is_err());
}
#[test]
fn copied_foreign_key_violation_and_nonpersonal_target_are_rejected() {
    let f = Fixture::new();
    f.user(false, 1, 0);
    for (name, sql, reason) in [
        (
            "fk",
            // This fixture intentionally corrupts a finalized backup. Disable FK
            // enforcement only for this edit so the importer must detect it.
            "PRAGMA foreign_keys=OFF; INSERT INTO user_local_view_permissions(viewer_uid,target_id) VALUES(zeroblob(33),CAST(X'01'||zeroblob(32) AS BLOB))",
            "import foreign key violation",
        ),
        (
            "team-target",
            "INSERT INTO user_local_view_permissions(viewer_uid,target_id) SELECT uid,zeroblob(33) FROM users",
            "personal user entity type",
        ),
    ] {
        let artifacts = f.backup(name);
        edit(&artifacts, sql);
        let before = hash(&artifacts.database);
        let ctl = control();
        let source = complete(&artifacts, &ctl);
        assert!(matches!(
            import_personal_list(source, Zeroizing::new([0x35; 32]), Default::default(), &ctl),
            Err(ScanError::Unsupported(actual)) if actual == reason
        ));
        assert_eq!(hash(&artifacts.database), before);
    }
}

#[test]
fn every_explicit_empty_table_family_is_rejected_without_payload_materialization() {
    // Policy-unit fixture only: remove trusted triggers and ignore CHECK/FK to populate
    // one inert row per forbidden table. It is not an R2-admitted database.
    let f = Fixture::new();
    let ctl = control();
    let job = Control::new(&ctl).unwrap();
    let c = Connection::open(&f.database).unwrap();
    c.pragma_update(None, "foreign_keys", false).unwrap();
    c.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    let db = scan::Database::private_target(c, job).unwrap();
    for (name, _) in triggers(&db).unwrap() {
        db.connection()
            .execute(&format!("DROP TRIGGER {}", copy::quote(&name)), [])
            .unwrap();
    }
    policy::verify(&db).unwrap();
    for &name in policy::EMPTY {
        let table = HOSTED_SMALL_V1.table(Component::Foks, name).unwrap();
        let values = table
            .columns()
            .map(|column| match column.storage_type() {
                foks_schema_contract::storage::StorageType::Integer => Value::Integer(1),
                foks_schema_contract::storage::StorageType::Blob => Value::Blob(vec![0; 33]),
                foks_schema_contract::storage::StorageType::Text => Value::Text("x".into()),
                foks_schema_contract::storage::StorageType::Real => Value::Real(1.0),
            })
            .collect::<Vec<_>>();
        let sql = format!(
            "INSERT INTO {} VALUES({})",
            copy::quote(name),
            (1..=values.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        db.connection()
            .execute(&sql, rusqlite::params_from_iter(&values))
            .unwrap();
        assert!(policy::verify(&db).is_err(), "{name}");
        db.connection()
            .execute(&format!("DELETE FROM {}", copy::quote(name)), [])
            .unwrap();
        policy::verify(&db).unwrap();
    }
    db.connection()
        .pragma_update(None, "ignore_check_constraints", false)
        .unwrap();
}

#[test]
fn embedded_nul_text_is_exact_and_invalid_utf8_is_not_lossily_imported() {
    let f = Fixture::new();
    let db = Connection::open(&f.database).unwrap();
    db.execute("UPDATE services SET endpoint=?1", ["before\0after"])
        .unwrap();
    drop(db);
    let artifacts = f.backup("nul");
    let owner = run(&artifacts);
    assert_eq!(
        owner
            .database
            .connection()
            .query_row("SELECT endpoint FROM services LIMIT 1", [], |r| r
                .get::<_, String>(0))
            .unwrap()
            .as_bytes(),
        b"before\0after"
    );
    let artifacts = f.backup("invalid-utf8");
    edit(
        &artifacts,
        "UPDATE services SET endpoint=CAST(X'80' AS TEXT)",
    );
    let ctl = control();
    assert!(matches!(
        import_personal_list(
            complete(&artifacts, &ctl),
            Zeroizing::new([0x35; 32]),
            Default::default(),
            &ctl
        ),
        Err(ScanError::Unsupported("invalid imported UTF8"))
    ));
}
#[test]
fn cancellation_at_each_owned_stage_and_identity_replacement_cannot_publish_owner() {
    let f = Fixture::new();
    let artifacts = f.backup("after-copy");
    let before = hash(&artifacts.database);
    for point in [Stage::Opened, Stage::Copied, Stage::Audited, Stage::Keys] {
        let ctl = control();
        let receipt = complete(&artifacts, &ctl);
        let cancelled = Arc::clone(&ctl.cancelled);
        let observed = Arc::new(std::sync::Mutex::new(None));
        let copy = Arc::clone(&observed);
        hook(point, move |path| {
            *copy.lock().unwrap() = Some(path.to_owned());
            cancelled.store(true, Ordering::Release);
            Ok(())
        });
        assert!(
            matches!(
                import_personal_list(
                    receipt,
                    Zeroizing::new([0x35; 32]),
                    Default::default(),
                    &ctl
                ),
                Err(ScanError::Cancelled)
            ),
            "{point:?}"
        );
        assert!(!observed.lock().unwrap().as_ref().unwrap().exists());
        assert_eq!(hash(&artifacts.database), before);
    }
    let ctl = control();
    hook(Stage::Keys, |path| {
        std::fs::rename(path, path.with_extension("old"))?;
        std::fs::write(path, b"replacement")?;
        Ok(())
    });
    assert!(import_personal_list(
        complete(&artifacts, &ctl),
        Zeroizing::new([0x35; 32]),
        Default::default(),
        &ctl
    )
    .is_err());
}
#[test]
fn pure_host_ledger_helper_preserves_lazy_capability_read_after_host_failure() {
    let called = std::cell::Cell::new(false);
    assert!(
        crate::standalone::host_key_backup_files_from_loaded(Vec::new(), None, || {
            called.set(true);
            Ok(Vec::new())
        })
        .is_err()
    );
    assert!(!called.get());
}

#[test]
fn genuine_package_public_key_mismatch_and_bounded_key_inputs_refuse_import() {
    let f = Fixture::new();
    for (name, sql, expected) in [
        ("public-mismatch", "UPDATE host_key_generations SET public_entity_id=zeroblob(33) WHERE state=2", Some("import host key mismatch")),
        ("hostchain-bytes", "UPDATE hostchain_links SET exact_link=zeroblob(1048576); INSERT INTO hostchain_links VALUES(2,zeroblob(32),X'01')", Some("hostchain aggregate cap")),
        ("hostchain-count", "DELETE FROM hostchain_links; WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4097) INSERT INTO hostchain_links SELECT x,CAST(printf('%032d',x) AS BLOB),X'00' FROM n", Some("hostchain count/type")),
        ("probe-cap", "UPDATE host_metadata SET bootstrap_blob=zeroblob(1048577)", None),
    ] {
        // The finalized producer is the actual source. Corruption/profile excess is
        // applied to that completed package, never repaired by the scanner/importer.
        let artifacts = f.backup(name);
        edit(&artifacts, sql);
        let before = hash(&artifacts.database);
        let ctl = control();
        let mut scanner = RestrictedScanner::capture_finalized_backup(&artifacts, CaptureLimits::default(), &ctl).unwrap();
        let scanned = loop {
            match scanner.advance(&ctl) {
                Ok(progress) if progress.complete => break scanner.finish(&ctl),
                Ok(_) => {},
                Err(error) => break Err(error),
            }
        };
        // The per-value storage profile rejects an over-1MiB probe already at R2.
        // The other cases reach the key validation of the real R3 path.
        if name == "probe-cap" {
            assert!(scanned.is_err());
        } else {
            let result = import_personal_list(scanned.unwrap(), Zeroizing::new([0x35;32]), Default::default(), &ctl);
            match expected {
                Some(reason) => assert!(matches!(result, Err(ScanError::Unsupported(actual)) if actual == reason), "{name}"),
                None => assert!(result.is_err(), "{name}"),
            }
        }
        assert_eq!(hash(&artifacts.database), before);
    }
}

#[test]
fn hooked_generation_preflight_accepts_4096_and_refuses_4097_before_collection() {
    let f = Fixture::new();
    let c = Connection::open(&f.database).unwrap();
    // Adapter-only fixture, with every row valid under native constraints. It
    // intentionally lacks encrypted files and is not an admitted key package.
    c.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4095) INSERT INTO host_key_generations SELECT CAST(printf('%016d',x) AS BLOB),1,printf('revoked-%d',x),CAST(printf('%033d',x) AS BLOB),4,x,1 FROM n").unwrap();
    let ctl = control();
    let db = scan::Database::private_target(c, Control::new(&ctl).unwrap()).unwrap();
    assert_eq!(keys::host_generations(&db).unwrap().len(), 4096);
    db.connection()
        .execute(
            "INSERT INTO host_key_generations VALUES(?1,1,'last-revoked',?2,4,4096,1)",
            params![[0xf1u8; 16].as_slice(), [0xf2u8; 33].as_slice()],
        )
        .unwrap();
    assert!(matches!(
        keys::host_generations(&db),
        Err(ScanError::Unsupported("key preflight count"))
    ));
    db.connection().execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4095) INSERT INTO capability_key_generations SELECT CAST(printf('%016d',x) AS BLOB),printf('revoked-%d',x),3,x,4096 FROM n").unwrap();
    assert_eq!(keys::capabilities(&db).unwrap().len(), 4096);
    db.connection()
        .execute(
            "INSERT INTO capability_key_generations VALUES(?1,'last-revoked',3,4096,4096)",
            [[0xf1u8; 16].as_slice()],
        )
        .unwrap();
    assert!(matches!(
        keys::capabilities(&db),
        Err(ScanError::Unsupported("key preflight count"))
    ));
}

#[test]
fn bootstrap_adapter_checks_probe_bound_before_owned_collection() {
    let f = Fixture::new();
    let ctl = control();
    let db = scan::Database::private_target(
        Connection::open(&f.database).unwrap(),
        Control::new(&ctl).unwrap(),
    )
    .unwrap();
    db.connection()
        .execute(
            "UPDATE host_metadata SET bootstrap_blob=zeroblob(1048576)",
            [],
        )
        .unwrap();
    assert_eq!(keys::bootstrap(&db).unwrap().probe_response.len(), 1048576);
    db.connection()
        .execute(
            "UPDATE host_metadata SET bootstrap_blob=zeroblob(1048577)",
            [],
        )
        .unwrap();
    assert!(matches!(
        keys::bootstrap(&db),
        Err(ScanError::Unsupported("bootstrap blob cap"))
    ));
}

#[test]
fn derived_namespace_counts_are_copied_without_trigger_replay() {
    let f = Fixture::new();
    let uid = f.user(false, 1, 0);
    let mut writer = foks_server_db::Database::open(&f.database, Default::default()).unwrap();
    writer
        .put_kv_directory(&foks_server_db::KvDirectoryMutation {
            uid: &uid,
            id: &[0x91; 16],
            version: 1,
            key_role: 1,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: b"opaque native directory bytes",
        })
        .unwrap();
    drop(writer);
    let artifacts = f.backup("derived");
    let source = Connection::open(&artifacts.database).unwrap();
    let counts = |c: &Connection| {
        c.query_row("SELECT accounted_bytes,accounted_objects,directory_count FROM kv_namespaces WHERE namespace_id=?1", [uid.as_slice()], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
    };
    let expected = counts(&source);
    assert_eq!(expected.2, 1);
    let owner = run(&artifacts);
    assert_eq!(counts(owner.database.connection()), expected);
}

#[test]
fn hooked_key_adapters_match_nontrivial_rotation_and_revoked_native_rows() {
    let f = Fixture::new();
    let c = Connection::open(&f.database).unwrap();
    let old: Vec<u8> = c
        .query_row(
            "SELECT generation_id FROM host_key_generations WHERE state=2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for (id, state) in [(0x91u8, 1i64), (0x92, 4)] {
        let id = [id; 16];
        c.execute(
            "INSERT INTO host_key_generations VALUES(?1,1,?2,?3,?4,100,?5)",
            params![
                id.as_slice(),
                crate::host::generation_file_name(id),
                [id[0]; 33].as_slice(),
                state,
                if state == 1 { None } else { Some(2i64) }
            ],
        )
        .unwrap();
    }
    for (id, state) in [(0x93u8, 2i64), (0x94, 3)] {
        let id = [id; 16];
        c.execute(
            "INSERT INTO capability_key_generations VALUES(?1,?2,?3,100,200)",
            params![
                id.as_slice(),
                crate::keys::capability_generation_file_name(id),
                state
            ],
        )
        .unwrap();
    }
    c.execute(
        "INSERT INTO host_rotation_operations VALUES(?1,1,?2,?3,1,NULL,NULL,NULL,100,100)",
        params![[0x95u8; 16].as_slice(), old, [0x91u8; 16].as_slice()],
    )
    .unwrap();
    let ctl = control();
    let db = scan::Database::private_target(c, Control::new(&ctl).unwrap()).unwrap();
    for published in [false, true] {
        if published {
            db.connection().execute_batch("UPDATE host_key_generations SET state=3 WHERE state=2; UPDATE host_key_generations SET state=2,activated_hostchain_seqno=2 WHERE state=1; UPDATE host_rotation_operations SET phase=2,add_link_seqno=2,add_published_at=150,updated_at=150").unwrap();
        }
        let native = foks_server_db::ReadDatabase::open(&f.database, Default::default()).unwrap();
        assert_eq!(
            keys::host_generations(&db).unwrap(),
            native.host_key_generations().unwrap()
        );
        assert_eq!(
            keys::capabilities(&db).unwrap(),
            native.capability_key_generations().unwrap()
        );
        let rotation = keys::rotation(&db).unwrap();
        assert!(rotation.is_some());
        assert_eq!(rotation, native.active_host_rotation().unwrap());
        let ours = crate::standalone::host_key_backup_files_from_loaded(
            keys::host_generations(&db).unwrap(),
            rotation,
            || Ok(keys::capabilities(&db).unwrap()),
        )
        .unwrap();
        assert_eq!(
            ours,
            crate::standalone::host_key_backup_files(&native).unwrap()
        );
    }
}

#[test]
fn key_binding_uses_verified_identity_and_canonical_encrypted_inventory() {
    let f = Fixture::new();
    let first = f.backup("key-binding-first");
    let second = f.backup("key-binding-second");
    edit(
        &second,
        "UPDATE services SET endpoint='different-service-row'",
    );
    let a = run(&first);
    let b = run(&second);
    assert_eq!(a.binding.canonical_name, "localhost");
    let reader = foks_server_db::ReadDatabase::open(&f.database, Default::default()).unwrap();
    assert_eq!(
        a.binding.host.as_slice(),
        reader.host_bootstrap().unwrap().unwrap().host_id
    );
    assert_ne!(a.qualification.source_digest, b.qualification.source_digest);
    assert_eq!(a.binding.key_bundle_digest, b.binding.key_bundle_digest);
    assert_ne!(
        a.qualification.target_incarnation,
        b.qualification.target_incarnation
    );

    // Independent fixture oracle reads the real encrypted files and sorts their
    // names with the manifest. It intentionally does not use capture's inventory.
    let mut files = vec![(
        "key-manifest.txt".to_owned(),
        0u8,
        first.key_manifest.clone(),
    )];
    for entry in std::fs::read_dir(&first.key_directory).unwrap() {
        let entry = entry.unwrap();
        files.push((entry.file_name().into_string().unwrap(), 1, entry.path()));
    }
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let manifest = files.iter().position(|v| v.1 == 0).unwrap();
    assert!(manifest > 0 && manifest + 1 < files.len());
    let mut expected = Sha256::new();
    expected.update(b"foks-native-personal-list-encrypted-keys-v1\0");
    for (name, kind, path) in files {
        let content = std::fs::read(path).unwrap();
        expected.update([kind]);
        expected.update((name.len() as u64).to_le_bytes());
        expected.update(name.as_bytes());
        expected.update((content.len() as u64).to_le_bytes());
        expected.update(Sha256::digest(content));
    }
    assert_eq!(
        a.binding.key_bundle_digest,
        <[u8; 32]>::from(expected.finalize())
    );
}

#[test]
fn consuming_database_handoff_intersects_original_job_and_cannot_clear_failure() {
    let original = control();
    let shorter = ScanControl::new(
        original
            .deadline
            .shorten(Instant::now() + Duration::from_secs(60))
            .unwrap(),
        Arc::clone(&original.cancelled),
    );
    let job = Control::new(&original).unwrap();
    let database =
        scan::Database::private_target(Connection::open_in_memory().unwrap(), Arc::clone(&job))
            .unwrap();
    let (connection, retained) = database.into_owned_connection(&shorter).unwrap();
    assert!(Arc::ptr_eq(&job, &retained));
    assert_eq!(
        retained.control.lock().unwrap().deadline.envelope(),
        shorter.deadline.envelope()
    );
    retained.intersect(&original).unwrap();
    assert_eq!(
        retained.control.lock().unwrap().deadline.envelope(),
        shorter.deadline.envelope()
    );
    original.cancelled.store(true, Ordering::Release);
    assert!(connection.query_row(
        "WITH RECURSIVE sequence(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM sequence WHERE x<10000) SELECT sum(x) FROM sequence",
        [],
        |row| row.get::<_, i64>(0),
    ).is_err());
    assert!(retained.interrupted.load(Ordering::Acquire));
    drop(connection);

    for rejection in 0..3 {
        let original = control();
        let job = Control::new(&original).unwrap();
        let database =
            scan::Database::private_target(Connection::open_in_memory().unwrap(), Arc::clone(&job))
                .unwrap();
        let supplied = match rejection {
            0 => control(),
            1 => {
                original.cancelled.store(true, Ordering::Release);
                original.clone()
            }
            _ => {
                job.interrupted.store(true, Ordering::Release);
                original.clone()
            }
        };
        let result = database.into_owned_connection(&supplied);
        assert!(match rejection {
            0 => matches!(
                result,
                Err(ScanError::Unsupported("different cancellation owner"))
            ),
            1 => matches!(result, Err(ScanError::Cancelled)),
            _ => matches!(
                result,
                Err(ScanError::Unsupported("SQLite operation/deadline limit"))
            ),
        });
    }
}

fn certificate_fixture() -> Fixture {
    let f = Fixture::new();
    let uid = f.user(false, 1, 0);
    let db = Connection::open(&f.database).unwrap();
    // The offline DER audit intentionally does not require this inactive owner
    // or its credential to match a current device/subkey public key.
    db.execute(
        "INSERT INTO devices(device_id,uid,active,role_type,visibility,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES(?1,?2,0,1,0,zeroblob(32),zeroblob(17),X'00',X'00')",
        params![[0x44_u8; 33].as_slice(), uid.as_slice()],
    ).unwrap();
    drop(db);
    f
}
fn insert_certificate(fixture: &Fixture, serial: &[u8], der: &[u8]) {
    Connection::open(&fixture.database).unwrap().execute(
        "INSERT INTO issued_certificates(serial,uid,device_id,credential_id,not_before,not_after,exact_certificate) SELECT ?1,uid,device_id,?2,1234567,987654321,?3 FROM devices LIMIT 1",
        params![serial, [0x55_u8; 33].as_slice(), der],
    ).unwrap();
}
fn issued_certificate(serial: &[u8]) -> Vec<u8> {
    crate::pki::issue_bounded_device_certificate(
        &crate::keys::MemoryKeyProvider::default(),
        "localhost",
        foks_crypto::ed25519_public_key(&[0x42; 32]),
        serial,
        1_700_000_000_000_000,
        1_800_000_000_000_000,
    )
    .unwrap()
}
fn certificate_workspace() -> (
    Arc<crate::net::memory::Request>,
    list::certificate::CertificateWorkspace,
) {
    let owner = crate::net::memory::Pools::new(Default::default()).request(None);
    let workspace = list::certificate::CertificateWorkspace::new(Arc::clone(&owner), 4096).unwrap();
    (owner, workspace)
}
fn audit_certificates(owner: &NativePersonalListImport) -> Result<()> {
    let (_memory, mut workspace) = certificate_workspace();
    let before = semantic::coordinate(&owner.database)?;
    let result = certificates::audit(&owner.database, &mut workspace);
    if result.is_ok() {
        assert_eq!(semantic::coordinate(&owner.database)?, before);
    }
    result
}

#[test]
fn certificate_inventory_accepts_all_padding_widths_zero_and_sign_padded_magnitude() {
    let f = certificate_fixture();
    for width in 1..=20 {
        let mut serial = vec![0; width];
        serial[width - 1] = width as u8;
        insert_certificate(&f, &serial, &issued_certificate(&[width as u8]));
    }
    insert_certificate(&f, &[0; 20], &issued_certificate(&[0]));
    insert_certificate(&f, &[0x80; 20], &issued_certificate(&[0x80; 20]));
    let artifacts = f.backup("certificate-widths");
    let original = hash(&artifacts.database);
    let owner = run(&artifacts);
    audit_certificates(&owner).unwrap();
    assert_eq!(hash(&artifacts.database), original);
}

#[test]
fn certificate_inventory_rejects_duplicates_only_after_complete_first_pass() {
    for malformed_last in [false, true] {
        let f = certificate_fixture();
        let der = issued_certificate(&[1]);
        insert_certificate(&f, &[1], &der);
        insert_certificate(&f, &[0, 1], &der);
        if malformed_last {
            insert_certificate(&f, &[2], &[1, 2, 3]);
        }
        let owner = run(&f.backup("certificate-duplicate"));
        let expected = if malformed_last {
            "issued certificate restricted DER"
        } else {
            "duplicate issued certificate DER"
        };
        assert!(
            matches!(audit_certificates(&owner), Err(ScanError::Unsupported(reason)) if reason == expected)
        );
    }
}

#[test]
fn certificate_inventory_rejects_unbounded_unsupported_or_misbound_der() {
    let valid = issued_certificate(&[1]);
    let mut trailing = valid.clone();
    trailing.push(0);
    let mut negative = valid.clone();
    let serial_offset = {
        let (_, parsed) = x509_parser::parse_x509_certificate(&valid).unwrap();
        parsed.raw_serial().as_ptr() as usize - valid.as_ptr() as usize
    };
    negative[serial_offset] |= 0x80;
    let ec = rcgen::CertificateParams::default()
        .self_signed(&rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap())
        .unwrap()
        .der()
        .to_vec();
    for (name, serial, der, reason) in [
        (
            "mismatch",
            vec![2],
            valid,
            "issued certificate serial mismatch",
        ),
        (
            "trailing",
            vec![1],
            trailing,
            "issued certificate restricted DER",
        ),
        (
            "negative",
            vec![1],
            negative,
            "issued certificate restricted DER",
        ),
        (
            "unsupported",
            vec![1],
            ec,
            "issued certificate restricted DER",
        ),
        ("empty", vec![1], vec![], "issued certificate DER bytes"),
        (
            "oversized",
            vec![1],
            vec![0; 4097],
            "issued certificate DER bytes",
        ),
        (
            "magnitude",
            vec![1],
            issued_certificate(&[1; 21]),
            "issued certificate restricted DER",
        ),
    ] {
        let f = certificate_fixture();
        insert_certificate(&f, &serial, &der);
        let owner = run(&f.backup(name));
        assert!(
            matches!(audit_certificates(&owner), Err(ScanError::Unsupported(actual)) if actual == reason),
            "{name}"
        );
    }
}

#[test]
fn certificate_inventory_preserves_raw_negative_expiry_and_different_sql_times() {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let mut parameters = rcgen::CertificateParams::default();
    parameters.not_before = time::OffsetDateTime::from_unix_timestamp(-2).unwrap();
    parameters.not_after = time::OffsetDateTime::from_unix_timestamp(-1).unwrap();
    parameters.serial_number = Some(rcgen::SerialNumber::from_slice(&[1]));
    let certificate = parameters.self_signed(&key).unwrap();
    let f = certificate_fixture();
    insert_certificate(&f, &[1], certificate.der());
    let owner = run(&f.backup("negative-der-expiry"));
    audit_certificates(&owner).unwrap();
}

#[test]
fn certificate_inventory_cannot_refresh_cancellation_or_failed_memory_owner() {
    let f = certificate_fixture();
    insert_certificate(&f, &[1], &issued_certificate(&[1]));
    let artifacts = f.backup("certificate-cancellation");
    let owner = run(&artifacts);
    let job = owner.database.control();
    let cancelled = Arc::clone(&job.control.lock().unwrap().cancelled);
    certificates::after_first_pass(move || cancelled.store(true, Ordering::Release));
    assert!(matches!(
        audit_certificates(&owner),
        Err(ScanError::Cancelled)
    ));
    let owner = run(&artifacts);
    let (memory, mut workspace) = certificate_workspace();
    assert!(!foks_snowpack::AllocationBudget::reserve(
        memory.as_ref(),
        usize::MAX
    ));
    assert!(matches!(
        certificates::audit(&owner.database, &mut workspace),
        Err(ScanError::Allocation)
    ));
}
