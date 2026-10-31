use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::*;

impl HardStateStore {
    /// Opens or initializes a FOKS hard-state database at `path`.
    ///
    /// An absent file is created and initialized with a new database id.
    /// Exclusive admission of a pristine profile relies on this. Reads that
    /// can run outside the profile lock use [`HardStateStore::open_existing`].
    pub fn open(path: &Path) -> Result<Self> {
        // SQLite cannot combine CREATE and NOFOLLOW on every supported build.
        // create_new is itself symlink-safe and gives NOFOLLOW an existing
        // inode to open without weakening the subsequent database open.
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(Error::SymlinkDatabase);
        }
        let mut connection = connect(path)?;
        configure(&connection)?;
        initialize_or_verify(&mut connection)?;
        Ok(Self { connection })
    }

    /// Opens an initialized FOKS hard-state database at `path` without
    /// creating or initializing one.
    ///
    /// Returns [`Error::StateMissing`] when the file is absent or is an empty
    /// file left before initialization. Neither case writes to the
    /// filesystem, so a read after a profile reset does not recreate the
    /// database with a new database id outside the profile lock. A file
    /// without the FOKS application id is rejected before anything is
    /// written to it. A FOKS database is verified and migrated as
    /// [`HardStateStore::open`] does.
    pub fn open_existing(path: &Path) -> Result<Self> {
        let missing = |error: &std::io::Error| error.kind() == ErrorKind::NotFound;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::SymlinkDatabase);
            }
            Ok(_) => {}
            Err(error) if missing(&error) => return Err(Error::StateMissing),
            Err(error) => return Err(error.into()),
        }
        let mut connection = match connect(path) {
            Ok(connection) => connection,
            // The file was removed after the check above. Without
            // SQLITE_OPEN_CREATE the open fails instead of recreating it.
            Err(_) if std::fs::symlink_metadata(path).is_err_and(|e| missing(&e)) => {
                return Err(Error::StateMissing);
            }
            Err(error) => return Err(error),
        };
        // Check before configure: switching an empty or foreign file to WAL
        // writes a database header and creates sidecar files.
        if hard_state_version(&connection)?.is_none() {
            return Err(Error::StateMissing);
        }
        configure(&connection)?;
        initialize_or_verify(&mut connection)?;
        Ok(Self { connection })
    }

    /// Checkpoints deliberate import-time writes while the caller owns exclusive
    /// maintenance access. Existing-source inspection never invokes recovery.
    pub fn checkpoint_for_snapshot(&mut self) -> Result<()> {
        let (busy, log, checkpointed): (i64, i64, i64) =
            self.connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
        if busy != 0 || log != 0 || checkpointed != 0 {
            return Err(Error::SnapshotInspection(
                "database checkpoint remains busy",
            ));
        }
        Ok(())
    }

    pub fn metadata(&self) -> Result<HardStateMetadata> {
        self.connection
            .query_row(
                "SELECT database_id, hard_state_revision, write_token
                 FROM hard_state_metadata WHERE singleton = 1",
                [],
                |row| {
                    let database_id = row.get::<_, Vec<u8>>(0)?;
                    let revision = row.get::<_, i64>(1)?;
                    let write_token = row.get::<_, Vec<u8>>(2)?;
                    Ok((database_id, revision, write_token))
                },
            )
            .map_err(Error::from)
            .and_then(|(database_id, revision, write_token)| {
                let database_id = database_id
                    .try_into()
                    .map_err(|_| Error::InvalidHardStateMetadata)?;
                let revision = stored_unsigned("hard-state revision", revision)?;
                let write_token = write_token
                    .try_into()
                    .map_err(|_| Error::InvalidHardStateMetadata)?;
                Ok(HardStateMetadata {
                    database_id,
                    revision,
                    write_token,
                })
            })
    }
}

/// Opens an existing database file without SQLITE_OPEN_CREATE.
fn connect(path: &Path) -> Result<Connection> {
    // macOS exposes /tmp as a symlink to /private/tmp. SQLite's NOFOLLOW
    // rejects symlinks in any path component, so resolve safe parent
    // aliases after separately rejecting a symlink at the database leaf.
    let database_path = path.canonicalize()?;
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_FULL_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let connection = Connection::open_with_flags(database_path, flags)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    connection.pragma_update(None, "secure_delete", true)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifacts(path: &Path) -> Vec<std::path::PathBuf> {
        ["", "-wal", "-shm", "-journal"]
            .into_iter()
            .map(|suffix| {
                let mut artifact = path.as_os_str().to_owned();
                artifact.push(suffix);
                std::path::PathBuf::from(artifact)
            })
            .filter(|artifact| std::fs::symlink_metadata(artifact).is_ok())
            .collect()
    }

    #[test]
    fn open_existing_reports_a_missing_database_without_creating_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite3");

        assert!(matches!(
            HardStateStore::open_existing(&path),
            Err(Error::StateMissing)
        ));
        assert!(artifacts(&path).is_empty());
    }

    // Removing the file between the existence check and the SQLite open
    // takes a separate branch in open_existing; reaching it would need a
    // hook between the two, so no test covers it.
    #[test]
    fn open_existing_does_not_recreate_a_removed_database_or_its_sidecars() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite3");
        drop(HardStateStore::open(&path).unwrap());
        for artifact in artifacts(&path) {
            std::fs::remove_file(artifact).unwrap();
        }

        assert!(matches!(
            HardStateStore::open_existing(&path),
            Err(Error::StateMissing)
        ));
        assert!(artifacts(&path).is_empty());
    }

    #[test]
    fn open_existing_leaves_an_uninitialized_file_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite3");
        std::fs::File::create(&path).unwrap();

        assert!(matches!(
            HardStateStore::open_existing(&path),
            Err(Error::StateMissing)
        ));
        assert_eq!(artifacts(&path), vec![path.clone()]);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }

    #[test]
    fn open_existing_rejects_a_foreign_database_without_initializing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite3");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE foreign_data (value INTEGER);")
            .unwrap();
        let before = std::fs::read(&path).unwrap();

        assert!(matches!(
            HardStateStore::open_existing(&path),
            Err(Error::WrongApplicationId { found: 0, .. })
        ));
        assert_eq!(artifacts(&path), vec![path.clone()]);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn open_existing_rejects_another_application_id_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite3");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "application_id", 7)
            .unwrap();
        let before = std::fs::read(&path).unwrap();

        assert!(matches!(
            HardStateStore::open_existing(&path),
            Err(Error::WrongApplicationId { found: 7, .. })
        ));
        assert_eq!(artifacts(&path), vec![path.clone()]);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn open_existing_opens_an_initialized_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite3");
        let created = HardStateStore::open(&path).unwrap().metadata().unwrap();

        let reopened = HardStateStore::open_existing(&path)
            .unwrap()
            .metadata()
            .unwrap();
        assert_eq!(reopened.database_id, created.database_id);
        assert_eq!(reopened.revision, created.revision);
    }

    #[cfg(unix)]
    #[test]
    fn open_existing_rejects_a_symlinked_database() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.sqlite3");
        drop(HardStateStore::open(&target).unwrap());
        let link = directory.path().join("hard.sqlite3");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(matches!(
            HardStateStore::open_existing(&link),
            Err(Error::SymlinkDatabase)
        ));
    }
}
