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

    pub fn metadata(&self) -> Result<HardStateMetadata> {
        self.connection
            .query_row(
                "SELECT database_id, hard_state_revision
                 FROM hard_state_metadata WHERE singleton = 1",
                [],
                |row| {
                    let database_id = row.get::<_, Vec<u8>>(0)?;
                    let revision = row.get::<_, i64>(1)?;
                    Ok((database_id, revision))
                },
            )
            .map_err(Error::from)
            .and_then(|(database_id, revision)| {
                let database_id = database_id
                    .try_into()
                    .map_err(|_| Error::InvalidHardStateMetadata)?;
                let revision = stored_unsigned("hard-state revision", revision)?;
                Ok(HardStateMetadata {
                    database_id,
                    revision,
                })
            })
    }
}
