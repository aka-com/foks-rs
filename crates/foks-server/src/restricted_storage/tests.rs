use super::*;
use foks_schema_contract::storage::{Component, HOSTED_SMALL_V1};
use rusqlite::{params, Connection, StatementStatus};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

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
        let mut writer =
            foks_server_db::Database::open(&database, foks_server_db::Config::default()).unwrap();
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
            foks_server_db::Config::default(),
        )
        .unwrap()
    }
}
fn control() -> ScanControl {
    ScanControl::new(
        Deadline::capture(Instant::now() + Duration::from_secs(300)).unwrap(),
        Arc::new(AtomicBool::new(false)),
    )
}
fn scanner(artifacts: &crate::BackupArtifacts, control: &ScanControl) -> RestrictedScanner {
    RestrictedScanner::capture_finalized_backup(artifacts, CaptureLimits::default(), control)
        .unwrap()
}
fn drain(scan: &mut RestrictedScanner, control: &ScanControl) -> Result<ScanProgress> {
    for _ in 0..10000 {
        let result = scan.advance(control)?;
        if result.complete {
            return Ok(result);
        }
    }
    panic!("bounded test drain exhausted");
}
fn complete(artifacts: &crate::BackupArtifacts) -> StorageScanComplete {
    let control = control();
    let mut scanner = scanner(artifacts, &control);
    for _ in 0..1000 {
        if scanner.advance(&control).unwrap().complete {
            return scanner.finish(&control).unwrap();
        }
    }
    panic!("test scan did not terminate");
}
fn hash(path: &Path) -> [u8; 32] {
    Sha256::digest(std::fs::read(path).unwrap()).into()
}
// Test-only edits occur before capture; use a normal connection and close it.
// The scanner itself never opens the original or writes/checkpoints a capture.
fn edit(artifacts: &crate::BackupArtifacts, sql: &str) {
    let db = Connection::open(&artifacts.database).unwrap();
    db.pragma_update(None, "journal_mode", "DELETE").unwrap();
    db.execute_batch(sql).unwrap();
    db.close().unwrap();
}

#[test]
fn genuine_finalized_backup_scans_all_tables_without_changing_any_source_bytes() {
    let fixture = Fixture::new();
    let artifacts = fixture.backup("genuine");
    // Bounded fixture-only metadata diagnostic; never print key contents.
    for name in [
        "foks-server.sqlite",
        "foks-server.sqlite-wal",
        "foks-server.sqlite-shm",
        "foks-server.sqlite-journal",
        "key-manifest.txt",
        "keys/key-encryption.key",
        "keys/host.key",
        "keys/metadata.key",
        "keys/merkle.key",
        "keys/client-ca.key",
        "keys/delegated-tls.key",
        "keys/recovery.key",
        "keys/capability.key",
    ] {
        let length = std::fs::symlink_metadata(artifacts.database.parent().unwrap().join(name))
            .map(|metadata| metadata.len());
        eprintln!("genuine backup fixture {name}: {length:?}");
    }
    let control = control();
    let package = capture::Package::capture(
        &artifacts,
        CaptureLimits::default(),
        &Control::new(&control).unwrap(),
    )
    .unwrap();
    let before = hash(&artifacts.database);
    let receipt = complete(&artifacts);
    assert_eq!(receipt.source_digest(), &package.digest());
    assert_eq!(receipt.profile(), "hosted-metadata-small-v1");
    assert_eq!(receipt.summary().tables, 100);
    assert!(receipt.summary().rows > 0);
    assert_eq!(receipt.summary().external_chunk_references, 0);
    assert!(receipt
        .database_header_modes()
        .iter()
        .all(|v| matches!(v, 1 | 2)));
    assert_eq!(hash(&artifacts.database), before);
    let after = capture::Package::capture(
        &artifacts,
        CaptureLimits::default(),
        &Control::new(&control).unwrap(),
    )
    .unwrap();
    assert_eq!(after.digest(), package.digest());
}

#[test]
fn live_sidecars_and_incomplete_packages_are_never_silently_dropped() {
    let fixture = Fixture::new();
    for (i, suffix) in ["-wal", "-journal", "-shm"].iter().enumerate() {
        let artifacts = fixture.backup(&format!("sidecar{i}"));
        let path = PathBuf::from(format!("{}{suffix}", artifacts.database.display()));
        std::fs::write(&path, b"live").unwrap();
        assert!(matches!(
            RestrictedScanner::capture_finalized_backup(
                &artifacts,
                CaptureLimits::default(),
                &control()
            ),
            Err(ScanError::Unsupported("backup file or live sidecar size"))
        ));
        assert_eq!(std::fs::read(path).unwrap(), b"live");
    }
    let artifacts = fixture.backup("missing-manifest");
    std::fs::remove_file(&artifacts.key_manifest).unwrap();
    assert!(RestrictedScanner::capture_finalized_backup(
        &artifacts,
        CaptureLimits::default(),
        &control()
    )
    .is_err());
    // Genuine WAL-backed latest data: the main bytes alone do not contain this
    // insert. The package must fail before SQLite can ignore the WAL.
    let artifacts = fixture.backup("real-wal");
    let live = Connection::open(&artifacts.database).unwrap();
    live.pragma_update(None, "journal_mode", "WAL").unwrap();
    live.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    live.execute(
        "INSERT INTO hostchain_links VALUES(900,?1,?2)",
        params![[90u8; 32], b"uncheckpointed"],
    )
    .unwrap();
    assert!(
        std::fs::metadata(format!("{}-wal", artifacts.database.display()))
            .unwrap()
            .len()
            > 0
    );
    assert!(RestrictedScanner::capture_finalized_backup(
        &artifacts,
        CaptureLimits::default(),
        &control()
    )
    .is_err());
}

#[test]
fn closed_companions_symlinks_and_bounded_filenames_fail_without_a_receipt() {
    let fixture = Fixture::new();
    let artifacts = fixture.backup("extra");
    std::fs::write(artifacts.key_directory.join("extra.key"), [0; 96]).unwrap();
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    assert!(drain(&mut scan, &ctl).is_err());
    assert!(matches!(scan.finish(&ctl), Err(ScanError::Incomplete)));
    let artifacts = fixture.backup("symlink");
    let victim = artifacts.key_directory.join("metadata.key");
    std::fs::remove_file(&victim).unwrap();
    std::os::unix::fs::symlink(fixture.keys.join("metadata.key"), &victim).unwrap();
    assert!(RestrictedScanner::capture_finalized_backup(
        &artifacts,
        CaptureLimits::default(),
        &control()
    )
    .is_err());
    let artifacts = fixture.backup("long");
    std::fs::write(
        artifacts
            .key_directory
            .join(format!("{}.key", "x".repeat(93))),
        [0; 96],
    )
    .unwrap();
    assert!(RestrictedScanner::capture_finalized_backup(
        &artifacts,
        CaptureLimits::default(),
        &control()
    )
    .is_err());
    let artifacts = fixture.backup("small-key");
    std::fs::write(artifacts.key_directory.join("metadata.key"), [0; 95]).unwrap();
    assert!(RestrictedScanner::capture_finalized_backup(
        &artifacts,
        CaptureLimits::default(),
        &control()
    )
    .is_err());
    let artifacts = fixture.backup("missing-key");
    std::fs::remove_file(artifacts.key_directory.join("metadata.key")).unwrap();
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    assert!(drain(&mut scan, &ctl).is_err());
}

#[test]
fn schema_and_encoding_are_checked_before_data_and_keep_internal_objects_explicit() {
    let fixture = Fixture::new();
    for (i,sql) in [
        "PRAGMA user_version=58",
        "CREATE TABLE extra(value)",
        "CREATE TRIGGER sqliteX_extra AFTER INSERT ON hostchain_links BEGIN SELECT 1; END",
        "ANALYZE",
        "PRAGMA writable_schema=ON; UPDATE sqlite_schema SET name=NULL WHERE name='services'; PRAGMA writable_schema=OFF",
    ].iter().enumerate() {
        let artifacts=fixture.backup(&format!("schema{i}")); edit(&artifacts,sql);
        assert!(RestrictedScanner::capture_finalized_backup(&artifacts,CaptureLimits::default(),&control()).is_err(),"{i}");
    }
    let artifacts = fixture.backup("utf16");
    std::fs::remove_file(&artifacts.database).unwrap();
    let db = Connection::open(&artifacts.database).unwrap();
    db.execute_batch(&format!(
        "PRAGMA encoding='UTF-16le'; CREATE TABLE x(a); PRAGMA user_version={}",
        foks_server_db::SCHEMA_VERSION
    ))
    .unwrap();
    drop(db);
    assert!(matches!(
        RestrictedScanner::capture_finalized_backup(
            &artifacts,
            CaptureLimits::default(),
            &control()
        ),
        Err(ScanError::Unsupported("schema version or encoding"))
    ));
}

#[test]
fn historical_oversized_rows_and_locator_payloads_fail_instead_of_becoming_eof() {
    let fixture = Fixture::new();
    let artifacts = fixture.backup("large-history");
    edit(
        &artifacts,
        "INSERT INTO hostchain_links VALUES(700,zeroblob(32),zeroblob(1048577))",
    );
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    assert!(matches!(drain(&mut scan, &ctl), Err(ScanError::Row { .. })));
    assert!(matches!(scan.advance(&ctl), Err(ScanError::Incomplete)));
    let artifacts = fixture.backup("large-locator");
    edit(&artifacts,"PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON; INSERT INTO user_local_view_permissions VALUES(zeroblob(1048576),zeroblob(1048576)); PRAGMA ignore_check_constraints=OFF");
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    assert!(matches!(
        drain(&mut scan, &ctl),
        Err(ScanError::Row {
            violation: Violation::RecordBytes { .. },
            ..
        })
    ));
    assert!(scan.finish(&ctl).is_err());
    let artifacts = fixture.backup("engine-too-large");
    edit(
        &artifacts,
        "INSERT INTO hostchain_links VALUES(800,zeroblob(32),zeroblob(3000000))",
    );
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    assert!(drain(&mut scan, &ctl).is_err());
}

#[test]
fn resumable_keysets_cover_rowid_extremes_and_without_rowid_ties() {
    let fixture = Fixture::new();
    let artifacts = fixture.backup("keysets");
    let db = Connection::open(&artifacts.database).unwrap();
    db.pragma_update(None, "journal_mode", "DELETE").unwrap();
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    for i in 0..300u64 {
        db.execute(
            "INSERT INTO user_local_view_permissions VALUES(?1,?2)",
            params![i.to_be_bytes().as_slice(), [2u8; 33]],
        )
        .unwrap();
    }
    for (id, byte) in [(i64::MIN, 80u8), (i64::MAX, 81u8)] {
        db.execute(
            "INSERT INTO hostchain_links VALUES(?1,?2,?3)",
            params![id, [byte; 32], b"small"],
        )
        .unwrap();
    }
    db.pragma_update(None, "ignore_check_constraints", false)
        .unwrap();
    drop(db);
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    assert!(matches!(
        scanner(&artifacts, &ctl).finish(&ctl),
        Err(ScanError::Incomplete)
    ));
    let mut calls = 0;
    let mut previous = 0;
    loop {
        let progress = scan.advance(&ctl).unwrap();
        calls += 1;
        assert!(progress.rows_checked >= previous && progress.rows_checked - previous <= 128);
        previous = progress.rows_checked;
        if progress.complete {
            break;
        }
        assert!(calls < 10);
    }
    assert!(calls >= 3);
    let receipt = scan.finish(&ctl).unwrap();
    assert!(receipt.summary().rows >= 302);
}

#[test]
fn sequence_deleted_highwater_and_exhaustion_are_preserved_not_recomputed() {
    let fixture = Fixture::new();
    for (i, value) in [0i64, 1234, i64::MAX].into_iter().enumerate() {
        let artifacts = fixture.backup(&format!("sequence{i}"));
        edit(&artifacts,&format!("DELETE FROM admin_audit; DELETE FROM sqlite_sequence; INSERT INTO sqlite_sequence VALUES('admin_audit',{value})"));
        assert_eq!(
            complete(&artifacts).summary().admin_audit_sequence,
            Some(value)
        );
    }
    for (i,sql) in [
        "DELETE FROM sqlite_sequence; INSERT INTO sqlite_sequence VALUES('other',1)",
        "DELETE FROM sqlite_sequence; INSERT INTO sqlite_sequence VALUES('admin_audit',-1)",
        "DELETE FROM sqlite_sequence; INSERT INTO sqlite_sequence VALUES('admin_audit',NULL)",
        "DELETE FROM sqlite_sequence; INSERT INTO sqlite_sequence VALUES('admin_audit',1),('admin_audit',2)",
        "INSERT INTO admin_audit(event_id,host_id,action,target_id,occurred_at_us) VALUES(900,zeroblob(33),1,x'',1); DELETE FROM sqlite_sequence",
    ].iter().enumerate() {
        let artifacts=fixture.backup(&format!("bad-sequence{i}")); edit(&artifacts,sql);
        let ctl=control(); let mut scan=scanner(&artifacts,&ctl);
        assert!(drain(&mut scan,&ctl).is_err(),"{i}");
        assert!(scan.finish(&ctl).is_err());
    }
}

#[test]
fn cancellation_deadline_and_private_mutation_cannot_complete_or_extend() {
    let fixture = Fixture::new();
    let artifacts = fixture.backup("controls");
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    ctl.cancelled.store(true, Ordering::Release);
    assert!(matches!(scan.advance(&ctl), Err(ScanError::Cancelled)));
    ctl.cancelled.store(false, Ordering::Release);
    assert!(matches!(scan.advance(&ctl), Err(ScanError::Incomplete)));
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    let other = control();
    assert!(scan.advance(&other).is_err());
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    let expired = ScanControl::new(
        Deadline::capture(Instant::now() + Duration::from_millis(50)).unwrap(),
        ctl.cancelled.clone(),
    );
    std::thread::sleep(Duration::from_millis(60));
    assert!(matches!(
        scan.advance(&expired),
        Err(ScanError::Deadline(_))
    ));
    let ctl = control();
    let mut scan = scanner(&artifacts, &ctl);
    while !scan.advance(&ctl).unwrap().complete {}
    std::fs::OpenOptions::new()
        .append(true)
        .open(scan.package.database())
        .unwrap()
        .write_all(&[0])
        .unwrap();
    assert!(scan.finish(&ctl).is_err());
}

#[test]
fn unchanged_dependency_coordinate_does_not_make_different_package_identity_equal() {
    let fixture = Fixture::new();
    let a = fixture.backup("identity-a");
    let b = fixture.backup("identity-b");
    let coord = |artifacts: &crate::BackupArtifacts| {
        Connection::open(&artifacts.database)
            .unwrap()
            .query_row(
                "SELECT incarnation,revision FROM metadata_read_dependency WHERE singleton=1",
                [],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)),
            )
            .unwrap()
    };
    let before = coord(&b);
    edit(&b,"INSERT INTO admin_audit(host_id,action,target_id,occurred_at_us) VALUES(zeroblob(33),1,x'',1)");
    assert_eq!(coord(&b), before);
    assert_ne!(complete(&a).source_digest(), complete(&b).source_digest());
}

#[test]
fn actual_first_and_continuation_sql_use_keysets_with_history_independent_probe_work() {
    let fixture = Fixture::new();
    let artifacts = fixture.backup("cost");
    let db = Connection::open(&artifacts.database).unwrap();
    db.pragma_update(None, "journal_mode", "DELETE").unwrap();
    db.pragma_update(None, "foreign_keys", false).unwrap();
    let table = HOSTED_SMALL_V1
        .table(Component::Foks, "user_local_view_permissions")
        .unwrap();
    let mut costs = Vec::new();
    for count in [1usize, 100, 10000] {
        db.execute("DELETE FROM user_local_view_permissions", [])
            .unwrap();
        let tx = db.unchecked_transaction().unwrap();
        for i in 0..count {
            let mut uid = [0; 33];
            uid[..8].copy_from_slice(&(i as u64).to_be_bytes());
            tx.execute(
                "INSERT INTO user_local_view_permissions VALUES(?1,?2)",
                params![uid, [7u8; 33]],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let mut first = db.prepare(&scan::row_sql(table, false).unwrap()).unwrap();
        let _: String = first.query_row([], |r| r.get(0)).unwrap();
        let mut next = db.prepare(&scan::row_sql(table, true).unwrap()).unwrap();
        let absent = next.query_row(params![[255u8; 33], [255u8; 33]], |r| r.get::<_, String>(0));
        assert!(matches!(absent, Err(rusqlite::Error::QueryReturnedNoRows)));
        costs.push((
            first.get_status(StatementStatus::VmStep),
            next.get_status(StatementStatus::VmStep),
        ));
    }
    assert!(costs.iter().all(|v| v == &costs[0]), "{costs:?}");
}

#[test]
fn capture_limits_and_private_uri_options_are_explicit() {
    let fixture = Fixture::new();
    let artifacts = fixture.backup("bounded-package");
    let ctl = control();
    assert!(RestrictedScanner::capture_finalized_backup(
        &artifacts,
        CaptureLimits {
            maximum_database_bytes: 1
        },
        &ctl
    )
    .is_err());
    assert!(RestrictedScanner::capture_finalized_backup(
        &artifacts,
        CaptureLimits {
            maximum_database_bytes: MAX_DATABASE + 1
        },
        &ctl
    )
    .is_err());
    let paused = control();
    paused.cancelled.store(true, Ordering::Release);
    assert!(matches!(
        RestrictedScanner::capture_finalized_backup(&artifacts, CaptureLimits::default(), &paused),
        Err(ScanError::Cancelled)
    ));
    // URI punctuation is only in the owned pathname, never interpreted as an
    // option. The source package retains its exact standard filenames.
    let path = fixture.root.join("private?#%mode=rw.sqlite");
    std::fs::copy(&artifacts.database, &path).unwrap();
    let before = hash(&path);
    let db = scan::Database::open(path.clone(), Control::new(&ctl).unwrap()).unwrap();
    db.verify_schema().unwrap();
    drop(db);
    assert_eq!(hash(&path), before);
    assert!(!PathBuf::from(format!("{}-wal", path.display())).exists());
    assert!(!PathBuf::from(format!("{}-shm", path.display())).exists());
    let artifacts = fixture.backup("key-count");
    for i in 0..MAX_KEYS {
        std::fs::write(
            artifacts.key_directory.join(format!("extra-{i}.key")),
            [0; 96],
        )
        .unwrap();
    }
    assert!(matches!(
        RestrictedScanner::capture_finalized_backup(&artifacts, CaptureLimits::default(), &ctl),
        Err(ScanError::Unsupported("backup file count"))
    ));
}

#[test]
fn bounded_errors_do_not_echo_sqlite_schema_text() {
    let sql = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
        Some("private source schema payload".into()),
    );
    let error = ScanError::from(sql);
    assert!(matches!(
        error,
        ScanError::Sql {
            extended_code: Some(_)
        }
    ));
    assert!(!error.to_string().contains("private source"));
}

#[test]
fn capture_change_and_cancellation_between_hash_and_copy_fail_closed() {
    let fixture = Fixture::new();
    let artifacts = fixture.backup("changed-capture");
    let key = artifacts.key_directory.join("metadata.key");
    capture::after_inventory(move || {
        std::fs::write(key, [0x55; 96]).unwrap();
    });
    assert!(matches!(
        RestrictedScanner::capture_finalized_backup(
            &artifacts,
            CaptureLimits::default(),
            &control()
        ),
        Err(ScanError::Unsupported("backup changed during capture"))
    ));
    let artifacts = fixture.backup("cancelled-capture");
    let ctl = control();
    let flag = ctl.cancelled.clone();
    capture::after_inventory(move || {
        flag.store(true, Ordering::Release);
    });
    assert!(matches!(
        RestrictedScanner::capture_finalized_backup(&artifacts, CaptureLimits::default(), &ctl),
        Err(ScanError::Cancelled)
    ));
    let artifacts = fixture.backup("extra-capture");
    let path = artifacts.database.parent().unwrap().join("unexpected");
    capture::after_inventory(move || {
        std::fs::write(path, b"extra").unwrap();
    });
    assert!(RestrictedScanner::capture_finalized_backup(
        &artifacts,
        CaptureLimits::default(),
        &control()
    )
    .is_err());
}

/// Runs an unbounded statement under `job` with `budget` until the progress
/// hook stops it, and returns the classified error.
fn interrupted(
    job: &Arc<Control>,
    budget: usize,
    during: impl FnOnce() + Send + 'static,
) -> ScanError {
    let db =
        scan::Database::private_target(Connection::open_in_memory().unwrap(), job.clone()).unwrap();
    job.statement_with_budget(budget).unwrap();
    let side = std::thread::spawn(during);
    let error = db
        .connection()
        .query_row(
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM c) SELECT count(*) FROM c",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_err();
    side.join().unwrap();
    let error = ScanError::from(error);
    assert!(
        matches!(
            error,
            ScanError::Sql {
                extended_code: Some(9)
            }
        ),
        "{error:?}"
    );
    job.classify(error)
}

#[test]
fn stop_inside_a_statement_keeps_cancellation_deadline_and_budget_classes() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let ctl = ScanControl::new(
        Deadline::capture(Instant::now() + Duration::from_secs(300)).unwrap(),
        cancelled.clone(),
    );
    let job = Control::new(&ctl).unwrap();
    let error = interrupted(&job, usize::MAX, move || {
        std::thread::sleep(Duration::from_millis(100));
        cancelled.store(true, Ordering::Release);
    });
    assert!(matches!(error, ScanError::Cancelled), "{error:?}");
    assert!(matches!(job.check(), Err(ScanError::Cancelled)));

    let ctl = ScanControl::new(
        Deadline::capture(Instant::now() + Duration::from_millis(200)).unwrap(),
        Arc::new(AtomicBool::new(false)),
    );
    let job = Control::new(&ctl).unwrap();
    let error = interrupted(&job, usize::MAX, || {});
    assert!(matches!(error, ScanError::Deadline(_)), "{error:?}");
    assert!(matches!(job.check(), Err(ScanError::Deadline(_))));

    let job = Control::new(&control()).unwrap();
    let error = interrupted(&job, 10_000, || {});
    assert!(
        matches!(
            error,
            ScanError::Unsupported("SQLite operation/deadline limit")
        ),
        "{error:?}"
    );
    // An error the hook did not cause is left as it is.
    assert!(matches!(
        job.classify(ScanError::Sql {
            extended_code: Some(1)
        }),
        ScanError::Sql {
            extended_code: Some(1)
        }
    ));
}
