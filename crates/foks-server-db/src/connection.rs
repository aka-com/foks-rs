use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::{schema, Config, Result};

pub struct Database {
    pub(crate) connection: Connection,
    pub(crate) config: Config,
    path: PathBuf,
}

pub struct ReadDatabase {
    pub(crate) connection: Connection,
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
        let path = path.as_ref().to_path_buf();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connection = Connection::open_with_flags(&path, flags)?;
        configure(&connection, &config)?;
        schema::initialize(&mut connection)?;
        Ok(Self {
            connection,
            config,
            path,
        })
    }

    pub fn open_reader(&self) -> Result<Connection> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(&self.path, flags)?;
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

    pub fn online_backup(&self, destination: impl AsRef<Path>) -> Result<()> {
        let mut target = Connection::open(destination)?;
        let backup = rusqlite::backup::Backup::new(&self.connection, &mut target)?;
        backup.run_to_completion(128, std::time::Duration::from_millis(1), None)?;
        Ok(())
    }
}

impl ReadDatabase {
    pub fn open(path: impl AsRef<Path>, config: Config) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(path, flags)?;
        configure_reader(&connection, &config)?;
        schema::validate_connection(&connection)?;
        Ok(Self { connection })
    }
}

fn configure(connection: &Connection, config: &Config) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "trusted_schema", false)?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
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
