#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, Transaction};

use crate::{schema, Config, Result};

pub struct Database {
    pub(crate) connection: Connection,
    pub(crate) config: Config,
    pub(crate) path: PathBuf,
}

pub struct ReadDatabase {
    pub(crate) connection: Connection,
    pub(crate) config: Config,
}

/// Stable identity for a regular, single-link SQLite database path.
///
/// Server startup captures this before taking the adjacent writer lock and
/// rechecks it under that lock before SQLite is allowed to open the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabasePathIdentity {
    path: PathBuf,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl DatabasePathIdentity {
    /// Creates the database leaf when absent, then records its filesystem
    /// identity without following a symlink at the leaf.
    #[cfg(unix)]
    pub fn prepare(path: &Path) -> Result<Self> {
        create_database_leaf(path)?;
        Self::existing(path)
    }

    #[cfg(not(unix))]
    pub fn prepare(_path: &Path) -> Result<Self> {
        Err(crate::Error::UnsafeDatabasePath(
            "reliable device, inode, and link-count checks are unavailable",
        ))
    }

    /// Records an already-existing database leaf without following symlinks.
    pub fn existing(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            let (_, device, inode) = validated_metadata(path)?;
            let canonical = path.canonicalize()?;
            let (_, canonical_device, canonical_inode) = validated_metadata(&canonical)?;
            if device != canonical_device || inode != canonical_inode {
                return Err(crate::Error::UnsafeDatabasePath(
                    "database path identity changed during inspection",
                ));
            }
            Ok(Self {
                path: canonical,
                device,
                inode,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(crate::Error::UnsafeDatabasePath(
                "reliable device, inode, and link-count checks are unavailable",
            ))
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Rejects replacement, symlink substitution, and hardlink creation since
    /// this identity was captured.
    pub fn recheck(&self) -> Result<()> {
        #[cfg(unix)]
        {
            let (_, device, inode) = validated_metadata(&self.path)?;
            if device != self.device || inode != self.inode {
                return Err(crate::Error::UnsafeDatabasePath(
                    "database path identity changed",
                ));
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            Err(crate::Error::UnsafeDatabasePath(
                "reliable device, inode, and link-count checks are unavailable",
            ))
        }
    }
}

/// A short-lived, request-scoped view of one SQLite WAL snapshot.
///
/// Dropping the value ends its read-only transaction. Callers should not hold
/// it across network I/O or other potentially unbounded work because an open
/// reader can delay WAL checkpoints.
#[must_use = "repository reads are pinned only while the snapshot is alive"]
pub struct ReadSnapshot<'connection> {
    transaction: Transaction<'connection>,
}

impl ReadSnapshot<'_> {
    pub(crate) fn connection(&self) -> &Connection {
        &self.transaction
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pragmas {
    pub foreign_keys: bool,
    pub journal_mode: String,
    pub synchronous: i64,
    pub busy_timeout_millis: u64,
    pub trusted_schema: bool,
}

impl Database {
    pub fn open(path: impl AsRef<Path>, config: Config) -> Result<Self> {
        let identity = DatabasePathIdentity::prepare(path.as_ref())?;
        Self::open_with_identity(identity, config)
    }

    /// Opens the exact identity captured by the caller. The server writer uses
    /// this after acquiring its adjacent lock so a replacement cannot become a
    /// newly accepted baseline between the lock-time check and SQLite open.
    pub fn open_with_identity(identity: DatabasePathIdentity, config: Config) -> Result<Self> {
        identity.recheck()?;
        let path = identity.path().to_path_buf();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = Connection::open_with_flags(&path, flags)?;
        identity.recheck()?;
        configure(&connection, &config)?;
        schema::initialize(&mut connection)?;
        crate::kv::validate_kv_tree_capacity(&connection, &config)?;
        Ok(Self {
            connection,
            config,
            path,
        })
    }

    /// Opens an already-initialized writer database without creating a new
    /// file. Offline administration commands use this to avoid leaving a
    /// blank installation behind after a mistyped or premature invocation.
    pub fn open_existing(path: impl AsRef<Path>, config: Config) -> Result<Self> {
        let identity = DatabasePathIdentity::existing(path.as_ref())?;
        identity.recheck()?;
        let path = identity.path().to_path_buf();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = Connection::open_with_flags(&path, flags)?;
        identity.recheck()?;
        configure(&connection, &config)?;
        schema::validate_connection(&connection)?;
        crate::kv::validate_kv_tree_capacity(&connection, &config)?;
        Ok(Self {
            connection,
            config,
            path,
        })
    }

    pub fn open_reader(&self) -> Result<Connection> {
        let identity = DatabasePathIdentity::existing(&self.path)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = Connection::open_with_flags(identity.path(), flags)?;
        identity.recheck()?;
        configure(&connection, &self.config)?;
        Ok(connection)
    }

    pub fn pragmas(&self) -> Result<Pragmas> {
        pragmas(&self.connection, &self.config)
    }

    pub fn integrity_check(&self) -> Result<bool> {
        let result: String =
            self.connection
                .pragma_query_value(None, "integrity_check", |row| row.get(0))?;
        Ok(result == "ok")
    }

    pub fn quick_check(&self) -> Result<bool> {
        let result: String = self
            .connection
            .pragma_query_value(None, "quick_check", |row| row.get(0))?;
        Ok(result == "ok")
    }

    pub fn online_backup(&self, destination: impl AsRef<Path>) -> Result<()> {
        online_backup(&self.connection, destination.as_ref(), || false)
    }
}

impl ReadDatabase {
    pub fn open(path: impl AsRef<Path>, config: Config) -> Result<Self> {
        let identity = DatabasePathIdentity::existing(path.as_ref())?;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = Connection::open_with_flags(identity.path(), flags)?;
        identity.recheck()?;
        configure_reader(&connection, &config)?;
        schema::validate_connection(&connection)?;
        crate::kv::validate_kv_tree_capacity(&connection, &config)?;
        Ok(Self { connection, config })
    }

    pub fn integrity_check(&self) -> Result<bool> {
        let result: String =
            self.connection
                .pragma_query_value(None, "integrity_check", |row| row.get(0))?;
        Ok(result == "ok")
    }

    pub fn maximum_team_role_bands(&self) -> usize {
        self.config.maximum_team_role_bands
    }

    /// Pins subsequent repository reads to one database state.
    pub fn snapshot(&self) -> Result<ReadSnapshot<'_>> {
        Ok(ReadSnapshot {
            transaction: self.connection.unchecked_transaction()?,
        })
    }

    pub fn online_backup(&self, destination: impl AsRef<Path>) -> Result<()> {
        online_backup(&self.connection, destination.as_ref(), || false)
    }

    pub fn online_backup_until(
        &self,
        destination: impl AsRef<Path>,
        cancelled: impl FnMut() -> bool,
    ) -> Result<()> {
        online_backup(&self.connection, destination.as_ref(), cancelled)
    }
}

#[cfg(unix)]
fn create_database_leaf(path: &Path) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    match options.open(path) {
        Ok(file) => {
            file.sync_all()?;
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn validated_metadata(path: &Path) -> Result<(std::fs::Metadata, u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(crate::Error::UnsafeDatabasePath(
            "database path is a symlink",
        ));
    }
    if !metadata.is_file() {
        return Err(crate::Error::UnsafeDatabasePath(
            "database path is not a regular file",
        ));
    }
    if metadata.nlink() != 1 {
        return Err(crate::Error::UnsafeDatabasePath(
            "database file must have exactly one hardlink",
        ));
    }
    if metadata.dev() == 0 || metadata.ino() == 0 {
        return Err(crate::Error::UnsafeDatabasePath(
            "filesystem did not provide a reliable database identity",
        ));
    }
    let device = metadata.dev();
    let inode = metadata.ino();
    Ok((metadata, device, inode))
}

fn online_backup(
    source: &Connection,
    destination: &Path,
    mut cancelled: impl FnMut() -> bool,
) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(destination)?.sync_all()?;
    let mut target = Connection::open(destination)?;
    let backup = rusqlite::backup::Backup::new(source, &mut target)?;
    loop {
        if cancelled() {
            return Err(crate::Error::Invalid("online backup cancelled"));
        }
        match backup.step(128)? {
            rusqlite::backup::StepResult::Done => break,
            rusqlite::backup::StepResult::More
            | rusqlite::backup::StepResult::Busy
            | rusqlite::backup::StepResult::Locked => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
    drop(backup);
    target.close().map_err(|(_, error)| error)?;
    std::fs::OpenOptions::new()
        .read(true)
        .open(destination)?
        .sync_all()?;
    Ok(())
}

fn configure(connection: &Connection, config: &Config) -> Result<()> {
    if config.maximum_database_bytes == 0
        || config.maximum_kv_namespace_bytes == 0
        || config.maximum_kv_namespace_objects == 0
        || config.maximum_kv_node_bytes == 0
        || config.maximum_kv_dirent_bytes == 0
        || config.maximum_kv_directories == 0
        || config.maximum_kv_dirents == 0
        || config.maximum_active_credentials_per_user == 0
        || config.maximum_backup_credentials_per_user == 0
        || config.maximum_user_chain_links == 0
        || config.maximum_team_chain_links == 0
        || config.maximum_teams == 0
        || config.maximum_team_members == 0
        || config.maximum_team_role_bands == 0
        || config.maximum_team_name_reservations == 0
        || config.maximum_boxes_per_mutation == 0
        || config.maximum_active_recovery_challenges == 0
        || config.maximum_recovery_challenges_per_entity == 0
        || config.maximum_passphrase_generations == 0
        || config.maximum_active_passphrase_challenges == 0
        || config.maximum_passphrase_challenges_per_user == 0
        || config.maximum_bad_passphrase_attempts == 0
        || config.bad_passphrase_window.is_zero()
        || config.maximum_team_view_capabilities_per_pair == 0
        || config.maximum_active_team_view_capabilities == 0
        || config.maximum_team_admin_capabilities_per_pair == 0
        || config.maximum_active_team_admin_capabilities == 0
        || config.maximum_remote_user_view_permissions_per_user == 0
        || config.maximum_active_remote_user_view_permissions == 0
        || config.maximum_remote_team_view_permissions_per_team == 0
        || config.maximum_active_remote_team_view_permissions == 0
    {
        return Err(crate::Error::Invalid("zero database capacity limit"));
    }
    if config.maximum_kv_node_bytes > foks_proto::MAXIMUM_KV_NODE_BYTES
        || config.maximum_kv_dirent_bytes > foks_proto::MAXIMUM_KV_DIRENT_BYTES
        || config.maximum_kv_directories > foks_proto::MAXIMUM_KV_DIRECTORIES as u64
        || config.maximum_kv_dirents > foks_proto::MAXIMUM_KV_DIRENTS as u64
    {
        return Err(crate::Error::Invalid(
            "KV capacity exceeds the client synchronization limits",
        ));
    }
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "trusted_schema", false)?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
    let page_size = u64::try_from(page_size).map_err(|_| crate::Error::IntegerRange)?;
    let page_count: i64 = connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
    let current_bytes = u64::try_from(page_count)
        .map_err(|_| crate::Error::IntegerRange)?
        .checked_mul(page_size)
        .ok_or(crate::Error::IntegerRange)?;
    if current_bytes > config.maximum_database_bytes {
        return Err(crate::Error::QuotaExceeded);
    }
    let maximum_pages = config
        .maximum_database_bytes
        .checked_add(page_size - 1)
        .ok_or(crate::Error::IntegerRange)?
        / page_size;
    connection.pragma_update(
        None,
        "max_page_count",
        i64::try_from(maximum_pages).map_err(|_| crate::Error::IntegerRange)?,
    )?;
    connection.busy_timeout(config.busy_timeout)?;
    Ok(())
}

fn configure_reader(connection: &Connection, config: &Config) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    connection.busy_timeout(config.busy_timeout)?;
    Ok(())
}

fn pragmas(connection: &Connection, config: &Config) -> Result<Pragmas> {
    Ok(Pragmas {
        foreign_keys: connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?,
        journal_mode: connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?,
        synchronous: connection.pragma_query_value(None, "synchronous", |row| row.get(0))?,
        busy_timeout_millis: u64::try_from(config.busy_timeout.as_millis()).unwrap_or(u64::MAX),
        trusted_schema: connection.pragma_query_value(None, "trusted_schema", |row| row.get(0))?,
    })
}
