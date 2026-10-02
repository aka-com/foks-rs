use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use foks_server_db::{Config, Database, ReadDatabase, SCHEMA_VERSION};
use rusqlite::Connection;

use crate::host::{load_or_bootstrap, BootstrapEndpoints, BootstrapInput};
use crate::keys::DirectoryKeyProvider;
use crate::{backup_standalone_installation, restore_backup, BackupArtifacts};

struct Fixture {
    _temporary: tempfile::TempDir,
    database: PathBuf,
    keys: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("server.sqlite");
        let keys = temporary.path().join("keys");
        let provider = DirectoryKeyProvider::open(&keys, [0x35; 32]).unwrap();
        let mut writer = Database::open(&database, Config::default()).unwrap();
        load_or_bootstrap(&mut writer, &provider, &bootstrap_input()).unwrap();
        Self {
            _temporary: temporary,
            database,
            keys,
        }
    }

    fn backup(&self, destination: &Path) -> BackupArtifacts {
        backup_standalone_installation(
            &self.database,
            &self.keys,
            [0x35; 32],
            destination,
            Config::default(),
        )
        .unwrap()
    }
}

fn bootstrap_input() -> BootstrapInput {
    BootstrapInput {
        canonical_name: "localhost".to_owned(),
        endpoints: BootstrapEndpoints {
            probe: "localhost:4430".to_owned(),
            public_services: "localhost:4431".to_owned(),
            authenticated: "localhost:4432".to_owned(),
        },
        ttl_seconds: 60,
        now_microseconds: 1_000,
    }
}

// Reconstruct the same predecessor schemas exercised by server-db's migration
// tests, rather than changing only user_version on the current schema.
fn predecessor(connection: &Connection, version: i64) {
    assert!((43..=48).contains(&version));
    if version <= 44 {
        for index in [
            "names_reservation_expiry",
            "team_names_reservation_expiry",
            "recovery_challenges_cleanup",
            "team_view_tokens_expiry",
            "team_view_challenges_expiry",
            "team_admin_tokens_expiry",
            "log_sends_created_at",
        ] {
            connection
                .execute_batch(&format!("DROP INDEX {index}"))
                .unwrap();
        }
    }
    if version == 43 {
        connection
            .execute_batch(
                "DROP TRIGGER rt_membership_insert;
             DROP TRIGGER rt_membership_delete;
             DROP TRIGGER rt_membership_update;
             DROP TRIGGER rt_team_access_update;
             ALTER TABLE rt_user_inboxes DROP COLUMN reconcile_dirty;",
            )
            .unwrap();
    }
    if version <= 45 {
        connection
            .execute_batch("ALTER TABLE sso_policy RENAME COLUMN blocked_reason TO fence;")
            .unwrap();
    }
    if version <= 46 {
        connection
            .execute_batch(
                "DROP INDEX sso_session_source;
             ALTER TABLE sso_sessions RENAME COLUMN source_hash TO admission_hash;
             CREATE INDEX sso_session_admission ON sso_sessions(host,admission_hash,expires_at_ms);
             ALTER TABLE sso_identity_challenges RENAME COLUMN source_hash TO admission_hash;",
            )
            .unwrap();
    }
    connection
        .pragma_update(None, "user_version", version)
        .unwrap();
}

fn add_old_browser_challenge(connection: &Connection) {
    connection
        .execute_batch(
            "INSERT INTO sso_identity_challenges(challenge,claim_hash,source_hash,expires_at_ms)
         VALUES(zeroblob(32),zeroblob(32),zeroblob(32),1000);",
        )
        .unwrap();
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &entry.path(), files);
            } else {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

fn assert_restored_identity(path: &Path, keys: &Path, expected_probe: &[u8], challenges: i64) {
    let reader = ReadDatabase::open(path, Config::default()).unwrap();
    assert!(reader.integrity_check().unwrap());
    assert_eq!(
        reader.host_bootstrap().unwrap().unwrap().probe_response,
        expected_probe
    );
    drop(reader);
    let connection = Connection::open(path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM sso_identity_challenges", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        challenges
    );
    drop(connection);
    // Restore must preserve the actual encrypted key set and signed host identity,
    // not merely produce a SQLite file whose schema version is acceptable.
    let provider = DirectoryKeyProvider::open(keys, [0x35; 32]).unwrap();
    let mut database = Database::open(path, Config::default()).unwrap();
    let state = load_or_bootstrap(&mut database, &provider, &bootstrap_input()).unwrap();
    assert_eq!(state.probe_response, expected_probe);
}

#[test]
fn supported_backup_schemas_validate_and_restore_without_changing_originals() {
    let fixture = Fixture::new();
    let temporary = tempfile::tempdir().unwrap();
    let expected_probe = ReadDatabase::open(&fixture.database, Config::default())
        .unwrap()
        .host_bootstrap()
        .unwrap()
        .unwrap()
        .probe_response;
    for version in 43..=48 {
        let backup_root = temporary.path().join(format!("backup-{version}"));
        let artifacts = fixture.backup(&backup_root);
        let connection = Connection::open(&artifacts.database).unwrap();
        add_old_browser_challenge(&connection);
        predecessor(&connection, version);
        drop(connection);
        let before = snapshot(&backup_root);
        crate::standalone::validate_completed_backup(&artifacts, Config::default()).unwrap();
        assert_eq!(
            snapshot(&backup_root),
            before,
            "validation mutated schema-{version} artifacts"
        );
        let destination = temporary.path().join(format!("restored-{version}"));
        restore_backup(
            &artifacts,
            destination.join("server.sqlite"),
            destination.join("keys"),
            Config::default(),
        )
        .unwrap();
        assert_eq!(
            snapshot(&backup_root),
            before,
            "restore mutated schema-{version} artifacts"
        );
        assert_restored_identity(
            &destination.join("server.sqlite"),
            &destination.join("keys"),
            &expected_probe,
            if version == SCHEMA_VERSION { 1 } else { 0 },
        );
    }
}

#[test]
fn backup_wal_is_recovered_only_in_private_copy_and_included_in_restore() {
    let fixture = Fixture::new();
    let temporary = tempfile::tempdir().unwrap();
    let expected_probe = ReadDatabase::open(&fixture.database, Config::default())
        .unwrap()
        .host_bootstrap()
        .unwrap()
        .unwrap()
        .probe_response;
    for version in [47, 48] {
        let root = temporary.path().join(format!("backup-{version}"));
        let artifacts = fixture.backup(&root);
        // Keep this idle writer open so the committed WAL is not checkpointed on
        // close. No writes race with snapshot/validation/restore in this test.
        let connection = Connection::open(&artifacts.database).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        add_old_browser_challenge(&connection);
        predecessor(&connection, version);
        assert!(
            root.join("foks-server.sqlite-wal")
                .metadata()
                .unwrap()
                .len()
                > 0
        );
        let before = snapshot(&root);
        crate::standalone::validate_completed_backup(&artifacts, Config::default()).unwrap();
        let destination = temporary.path().join(format!("restored-{version}"));
        restore_backup(
            &artifacts,
            destination.join("server.sqlite"),
            destination.join("keys"),
            Config::default(),
        )
        .unwrap();
        assert_eq!(
            snapshot(&root),
            before,
            "original WAL/SHM or database changed"
        );
        assert_restored_identity(
            &destination.join("server.sqlite"),
            &destination.join("keys"),
            &expected_probe,
            if version == SCHEMA_VERSION { 1 } else { 0 },
        );
    }
}

#[test]
fn invalid_or_unsupported_backup_never_publishes_or_changes_original() {
    let fixture = Fixture::new();
    let temporary = tempfile::tempdir().unwrap();
    for case in [
        "old",
        "future",
        "wrong-app",
        "empty",
        "truncated",
        "migration",
        "manifest",
        "key",
    ] {
        let root = temporary.path().join(format!("backup-{case}"));
        let artifacts = fixture.backup(&root);
        let connection = Connection::open(&artifacts.database).unwrap();
        predecessor(&connection, 43);
        match case {
            "old" => connection.pragma_update(None, "user_version", 42).unwrap(),
            "future" => connection
                .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                .unwrap(),
            "wrong-app" => connection
                .pragma_update(None, "application_id", 123)
                .unwrap(),
            "migration" => connection
                .execute_batch("DROP TABLE sso_binding_receipts;")
                .unwrap(),
            _ => {}
        }
        drop(connection);
        match case {
            "empty" => std::fs::write(&artifacts.database, []).unwrap(),
            "truncated" => std::fs::write(&artifacts.database, b"SQLite format 3\0broken").unwrap(),
            "manifest" => std::fs::write(&artifacts.key_manifest, b"incorrect manifest").unwrap(),
            "key" => {
                std::fs::remove_file(artifacts.key_directory.join(crate::keys::WRAPPING_KEY_FILE))
                    .unwrap()
            }
            _ => {}
        }
        let before = snapshot(&root);
        assert!(
            crate::standalone::validate_completed_backup(&artifacts, Config::default()).is_err(),
            "{case}"
        );
        let destination = temporary.path().join(format!("restored-{case}"));
        assert!(
            restore_backup(
                &artifacts,
                destination.join("server.sqlite"),
                destination.join("keys"),
                Config::default()
            )
            .is_err(),
            "{case}"
        );
        assert!(!destination.exists(), "failed restore published {case}");
        assert_eq!(
            snapshot(&root),
            before,
            "failed migration/validation mutated {case}"
        );
    }
}

#[test]
fn oversized_database_or_wal_is_rejected_without_publishing() {
    let fixture = Fixture::new();
    let temporary = tempfile::tempdir().unwrap();
    for wal in [false, true] {
        let root = temporary.path().join(format!("backup-{wal}"));
        let artifacts = fixture.backup(&root);
        let database_bytes = std::fs::metadata(&artifacts.database).unwrap().len();
        let maximum_database_bytes = if wal {
            let path = root.join("foks-server.sqlite-wal");
            std::fs::File::create(path)
                .unwrap()
                .set_len(database_bytes + 1)
                .unwrap();
            database_bytes
        } else {
            database_bytes - 1
        };
        let before = snapshot(&root);
        let destination = temporary.path().join(format!("restored-{wal}"));
        let result = restore_backup(
            &artifacts,
            destination.join("server.sqlite"),
            destination.join("keys"),
            Config {
                maximum_database_bytes,
                ..Config::default()
            },
        );
        assert!(matches!(
            result,
            Err(crate::Error::Database(foks_server_db::Error::Capacity(_)))
        ));
        assert!(!destination.exists());
        assert_eq!(snapshot(&root), before);
    }
}

#[cfg(unix)]
#[test]
fn symlinked_database_and_sidecars_are_rejected_without_touching_targets() {
    let fixture = Fixture::new();
    let temporary = tempfile::tempdir().unwrap();
    for name in [
        "foks-server.sqlite",
        "foks-server.sqlite-wal",
        "foks-server.sqlite-shm",
    ] {
        let root = temporary.path().join(format!("backup-{name}"));
        let artifacts = fixture.backup(&root);
        let target = temporary.path().join(format!("target-{name}"));
        let link = root.join(name);
        if link.exists() {
            std::fs::rename(&link, &target).unwrap();
        } else {
            std::fs::write(&target, b"untouched target").unwrap();
        }
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let before = snapshot(&root);
        let target_before = std::fs::read(&target).unwrap();
        let destination = temporary.path().join(format!("restored-{name}"));
        assert!(restore_backup(
            &artifacts,
            destination.join("server.sqlite"),
            destination.join("keys"),
            Config::default(),
        )
        .is_err());
        assert!(!destination.exists());
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(target).unwrap(), target_before);
        assert_eq!(snapshot(&root), before);
    }
}

#[test]
fn retention_accepts_supported_prior_schemas_and_preserves_retained_artifacts() {
    let fixture = Fixture::new();
    let root = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for (index, version) in [43, 44, 48].into_iter().enumerate() {
        let path = root
            .path()
            .join(format!("backup-{:020}-00000000000000000001", index + 1));
        let artifacts = fixture.backup(&path);
        let connection = Connection::open(&artifacts.database).unwrap();
        predecessor(&connection, version);
        drop(connection);
        paths.push(path);
    }
    let before = snapshot(root.path());
    super::enforce_retention(root.path(), 3, &Config::default()).unwrap();
    assert_eq!(snapshot(root.path()), before);
    let retained_old = snapshot(&paths[1]);
    let retained_current = snapshot(&paths[2]);
    super::enforce_retention(root.path(), 2, &Config::default()).unwrap();
    assert!(!paths[0].exists());
    assert_eq!(snapshot(&paths[1]), retained_old);
    assert_eq!(snapshot(&paths[2]), retained_current);
    // Fail closed on an unsupported archive rather than deleting it or other
    // snapshots merely to meet the configured count.
    let unsupported = fixture.backup(&paths[0]);
    let connection = Connection::open(&unsupported.database).unwrap();
    connection.pragma_update(None, "user_version", 42).unwrap();
    drop(connection);
    let before = snapshot(root.path());
    assert!(super::enforce_retention(root.path(), 1, &Config::default()).is_err());
    assert_eq!(snapshot(root.path()), before);
}
