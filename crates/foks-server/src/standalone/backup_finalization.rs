//! Final publication of a fresh, privately owned backup, never a live/source DB.
use crate::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

fn check_cancelled(cancelled: Option<&AtomicBool>) -> Result<()> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err(Error::Chunk("backup cancelled"));
    }
    Ok(())
}
fn sqlite<T>(value: rusqlite::Result<T>) -> Result<T> {
    value.map_err(|error| Error::Database(error.into()))
}
fn sidecar(database: &Path, suffix: &str) -> PathBuf {
    let mut name = database.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}
fn verify_sidecars(database: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        match std::fs::symlink_metadata(sidecar(database, suffix)) {
            Ok(metadata) if metadata.is_file() && metadata.len() == 0 => {}
            Ok(_) => return Err(Error::Chunk("backup sidecar remains after finalization")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// The caller owns a newly created destination and has closed every backup
/// reader/writer. No existing completed backup may be repaired by this helper.
pub(super) fn finalize_and_publish(
    database: &Path,
    key_manifest: &Path,
    manifest: &[u8],
    cancelled: Option<&AtomicBool>,
) -> Result<()> {
    check_cancelled(cancelled)?;
    match std::fs::symlink_metadata(key_manifest) {
        Ok(_) => return Err(Error::Chunk("backup completion marker already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // Validate the leaf before canonicalizing parent aliases (e.g. /tmp on
    // macOS), using the same identity contract as the native database opener.
    let identity = foks_server_db::DatabasePathIdentity::existing(database)?;
    let database = identity.path();
    identity.recheck()?;
    // No CREATE or URI, no Database::open/migration, no busy retries. This is
    // exclusively the fresh backup destination, not an arbitrary source path.
    let connection = sqlite(Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ))?;
    identity.recheck()?;
    sqlite(connection.busy_timeout(Duration::ZERO))?;
    check_cancelled(cancelled)?;
    {
        let mut statement = sqlite(connection.prepare("PRAGMA wal_checkpoint(TRUNCATE)"))?;
        let mut rows = sqlite(statement.query([]))?;
        let row = sqlite(rows.next())?.ok_or(Error::Chunk("backup checkpoint result"))?;
        let busy: i64 = sqlite(row.get(0))?;
        let log: i64 = sqlite(row.get(1))?;
        let checkpointed: i64 = sqlite(row.get(2))?;
        if busy != 0 || log < -1 || log != checkpointed || sqlite(rows.next())?.is_some() {
            return Err(Error::Chunk("backup final checkpoint incomplete"));
        }
    }
    check_cancelled(cancelled)?;
    let mode: String =
        sqlite(connection.query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0)))?;
    let observed: String =
        sqlite(connection.query_row("PRAGMA journal_mode", [], |row| row.get(0)))?;
    if mode != "delete" || observed != "delete" {
        return Err(Error::Chunk("backup final journal mode"));
    }
    sqlite(connection.close().map_err(|(_, error)| error))?;
    check_cancelled(cancelled)?;
    identity.recheck()?;
    verify_sidecars(database)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    let database_file = options.open(database)?;
    if !database_file.metadata()?.is_file() {
        return Err(Error::Chunk("backup database is not regular"));
    }
    database_file.sync_all()?;
    identity.recheck()?;
    check_cancelled(cancelled)?;
    // Preserve existing create-new, manifest-last publication. Errors after
    // creation can leave a partial marker; no atomic-rename claim is made.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut manifest_file = options.open(key_manifest)?;
    manifest_file.write_all(manifest)?;
    manifest_file.sync_all()?;
    File::open(
        key_manifest
            .parent()
            .ok_or(Error::Chunk("backup destination parent"))?,
    )?
    .sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: tempfile::TempDir,
        database: PathBuf,
        manifest: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let database = root.path().join("foks-server.sqlite");
            let manifest = root.path().join("key-manifest.txt");
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch("CREATE TABLE item(value INTEGER); INSERT INTO item VALUES(1)")
                .unwrap();
            connection.close().unwrap();
            Self {
                root,
                database,
                manifest,
            }
        }
        fn finish(&self) -> Result<()> {
            finalize_and_publish(&self.database, &self.manifest, b"fixture-manifest", None)
        }
        fn values(&self) -> Vec<i64> {
            let connection = Connection::open(&self.database).unwrap();
            let mut statement = connection
                .prepare("SELECT value FROM item ORDER BY value")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        }
    }
    #[test]
    fn wal_frames_checkpoint_before_manifest_and_delete_mode_is_supported() {
        let fixture = Fixture::new();
        let writer = Connection::open(&fixture.database).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO item VALUES(2)",
            )
            .unwrap();
        // A readonly last closer reproduces the public backup's residual files.
        let reader =
            Connection::open_with_flags(&fixture.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert_eq!(
            reader
                .query_row("SELECT count(*) FROM item", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        writer.close().unwrap();
        reader.close().unwrap();
        assert!(
            std::fs::metadata(sidecar(&fixture.database, "-wal"))
                .unwrap()
                .len()
                > 0
        );
        fixture.finish().unwrap();
        verify_sidecars(&fixture.database).unwrap();
        assert_eq!(
            std::fs::read(&fixture.manifest).unwrap(),
            b"fixture-manifest"
        );
        assert_eq!(fixture.values(), vec![1, 2]);
        let connection = Connection::open(&fixture.database).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "delete"
        );
        connection.close().unwrap();
        // The finalization operation also accepts a fresh DELETE-mode DB.
        let delete_fixture = Fixture::new();
        delete_fixture.finish().unwrap();
        assert_eq!(delete_fixture.values(), vec![1]);
    }
    #[test]
    fn pinned_reader_or_writer_prevents_marker_without_losing_rows() {
        for write_lock in [false, true] {
            let fixture = Fixture::new();
            let writer = Connection::open(&fixture.database).unwrap();
            writer
                .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0")
                .unwrap();
            let reader = Connection::open(&fixture.database).unwrap();
            reader.execute_batch("BEGIN; SELECT * FROM item").unwrap();
            writer.execute_batch("INSERT INTO item VALUES(2)").unwrap();
            if write_lock {
                writer.execute_batch("BEGIN IMMEDIATE").unwrap();
            }
            assert!(fixture.finish().is_err());
            assert!(!fixture.manifest.exists());
            if write_lock {
                writer.execute_batch("ROLLBACK").unwrap();
            }
            reader.execute_batch("ROLLBACK").unwrap();
            reader.close().unwrap();
            writer.close().unwrap();
            assert_eq!(fixture.values(), vec![1, 2]);
            fixture.finish().unwrap();
        }
    }
    #[test]
    fn missing_source_cancellation_and_existing_marker_refuse_without_mutation() {
        let fixture = Fixture::new();
        let before = std::fs::read(&fixture.database).unwrap();
        let cancelled = AtomicBool::new(true);
        assert!(finalize_and_publish(
            &fixture.database,
            &fixture.manifest,
            b"bad",
            Some(&cancelled)
        )
        .is_err());
        assert!(!fixture.manifest.exists());
        std::fs::write(&fixture.manifest, b"original").unwrap();
        assert!(fixture.finish().is_err());
        assert_eq!(std::fs::read(&fixture.manifest).unwrap(), b"original");
        assert_eq!(std::fs::read(&fixture.database).unwrap(), before);
        let missing = fixture.root.path().join("missing.sqlite");
        let missing_marker = fixture.root.path().join("missing-marker");
        assert!(finalize_and_publish(&missing, &missing_marker, b"bad", None).is_err());
        assert!(!missing.exists());
        assert!(!missing_marker.exists());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_database_or_completion_marker_is_not_followed() {
        let fixture = Fixture::new();
        let alias = fixture.root.path().join("alias.sqlite");
        std::os::unix::fs::symlink(&fixture.database, &alias).unwrap();
        assert!(finalize_and_publish(&alias, &fixture.manifest, b"bad", None).is_err());
        assert!(!fixture.manifest.exists());
        std::os::unix::fs::symlink("absent-target", &fixture.manifest).unwrap();
        assert!(fixture.finish().is_err());
        assert!(!fixture.root.path().join("absent-target").exists());
    }
    #[cfg(unix)]
    #[test]
    fn canonical_parent_alias_is_supported_but_hardlinked_leaf_is_rejected() {
        let fixture = Fixture::new();
        let outer = tempfile::tempdir().unwrap();
        let alias = outer.path().join("parent-alias");
        std::os::unix::fs::symlink(fixture.root.path(), &alias).unwrap();
        finalize_and_publish(
            &alias.join("foks-server.sqlite"),
            &alias.join("key-manifest.txt"),
            b"parent-alias",
            None,
        )
        .unwrap();
        assert_eq!(std::fs::read(&fixture.manifest).unwrap(), b"parent-alias");
        verify_sidecars(&fixture.database).unwrap();
        assert_eq!(fixture.values(), vec![1]);

        let hardlinked = Fixture::new();
        std::fs::hard_link(&hardlinked.database, hardlinked.root.path().join("other")).unwrap();
        let before = std::fs::read(&hardlinked.database).unwrap();
        assert!(hardlinked.finish().is_err());
        assert!(!hardlinked.manifest.exists());
        assert_eq!(std::fs::read(&hardlinked.database).unwrap(), before);
    }
    #[test]
    fn public_backup_is_self_contained_and_restores_unchanged_keys_and_rows() {
        use crate::host::{BootstrapEndpoints, BootstrapInput};
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("source.sqlite");
        let keys = root.path().join("source-keys");
        let provider = crate::keys::DirectoryKeyProvider::open(&keys, [0x35; 32]).unwrap();
        let mut writer =
            foks_server_db::Database::open(&database, foks_server_db::Config::default()).unwrap();
        crate::host::load_or_bootstrap(
            &mut writer,
            &provider,
            &BootstrapInput {
                canonical_name: "localhost".into(),
                endpoints: BootstrapEndpoints {
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
        drop(provider);
        let source_before = std::fs::read(&database).unwrap();
        let artifacts = crate::backup_standalone_installation(
            &database,
            &keys,
            [0x35; 32],
            root.path().join("backup"),
            foks_server_db::Config::default(),
        )
        .unwrap();
        verify_sidecars(&artifacts.database).unwrap();
        let backup = foks_server_db::ReadDatabase::open(
            &artifacts.database,
            foks_server_db::Config::default(),
        )
        .unwrap();
        assert!(backup.integrity_check().unwrap());
        let saved = backup.host_bootstrap().unwrap().unwrap();
        drop(backup);
        verify_sidecars(&artifacts.database).unwrap();
        assert_eq!(
            std::fs::read(&artifacts.key_manifest).unwrap(),
            saved.key_manifest
        );
        assert_eq!(std::fs::read(&database).unwrap(), source_before);
        for entry in std::fs::read_dir(&artifacts.key_directory).unwrap() {
            let entry = entry.unwrap();
            assert_eq!(
                std::fs::read(entry.path()).unwrap(),
                std::fs::read(keys.join(entry.file_name())).unwrap()
            );
        }
        let restored = root.path().join("restored.sqlite");
        crate::restore_backup(
            &artifacts,
            &restored,
            root.path().join("restored-keys"),
            foks_server_db::Config::default(),
        )
        .unwrap();
        let restored =
            foks_server_db::ReadDatabase::open(restored, foks_server_db::Config::default())
                .unwrap();
        assert_eq!(
            restored.host_bootstrap().unwrap().unwrap().host_id,
            saved.host_id
        );
    }
}
