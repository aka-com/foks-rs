use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::*;

impl HardStateStore {
    /// Opens or initializes a FOKS hard-state database at `path`.
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
        // macOS exposes /tmp as a symlink to /private/tmp. SQLite's NOFOLLOW
        // rejects symlinks in any path component, so resolve safe parent
        // aliases after separately rejecting a symlink at the database leaf.
        let database_path = path.canonicalize()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = Connection::open_with_flags(database_path, flags)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "trusted_schema", false)?;
        connection.pragma_update(None, "secure_delete", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
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
