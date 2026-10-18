use crate::allocation::RowAllocationExt as _;
use rusqlite::{params, Connection, OptionalExtension as _, TransactionBehavior};

use crate::error::unsigned;
use crate::{Database, Error, ReadDatabase, Result};

const WAITLIST_ID_BYTES: usize = 13;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaitlistEntry {
    pub waitlist_id: [u8; WAITLIST_ID_BYTES],
    pub email: String,
    pub created_at: u64,
}

/// Selects the entries an operator purge deletes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitlistPurge<'a> {
    All,
    /// Entries whose `created_at` is earlier than this time, in microseconds.
    CreatedBefore(u64),
    /// The entry for this address, compared under ASCII case folding.
    Email(&'a str),
}

impl Database {
    /// Returns the stored identifier for `email`: `id` when a new entry is
    /// inserted, or the identifier of the existing entry when the address is
    /// already listed under ASCII case folding. Every join is refused with
    /// `Capacity` once the table holds `maximum_waitlist_entries` rows, so a
    /// full table does not reveal which addresses it contains.
    pub fn join_waitlist(
        &mut self,
        id: &[u8; WAITLIST_ID_BYTES],
        email: &str,
        now: u64,
    ) -> Result<[u8; WAITLIST_ID_BYTES]> {
        if id[0] != 1 || !valid_email(email) {
            return Err(Error::Invalid("waitlist entry"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let maximum = integer(self.config.maximum_waitlist_entries)?;
        // Triggers maintain this one-row count, so the check does not scan
        // the table on an unauthenticated route.
        let count: i64 = transaction.query_row(
            "SELECT entries FROM waitlist_count WHERE singleton = 0",
            [],
            |row| row.budgeted_get(0),
        )?;
        if count >= maximum {
            return Err(Error::Capacity("waitlist"));
        }
        let existing = transaction
            .query_row(
                "SELECT waitlist_id FROM waitlist_entries WHERE email = ?1 COLLATE NOCASE",
                [email],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return stored_id(existing);
        }
        transaction
            .execute(
                "INSERT INTO waitlist_entries(waitlist_id, email, created_at)
                 VALUES (?1, ?2, ?3)",
                params![id, email, integer(now)?],
            )
            .map_err(|error| map_duplicate(error, "waitlist entry"))?;
        transaction.commit()?;
        Ok(*id)
    }

    /// Deletes the selected entries and returns the number deleted.
    pub fn purge_waitlist(&mut self, selection: WaitlistPurge<'_>) -> Result<u64> {
        let deleted = match selection {
            WaitlistPurge::All => self
                .connection
                .execute("DELETE FROM waitlist_entries", [])?,
            WaitlistPurge::CreatedBefore(before) => self.connection.execute(
                "DELETE FROM waitlist_entries WHERE created_at < ?1",
                [integer(before)?],
            )?,
            WaitlistPurge::Email(email) => self.connection.execute(
                "DELETE FROM waitlist_entries WHERE email = ?1 COLLATE NOCASE",
                [email],
            )?,
        };
        u64::try_from(deleted).map_err(|_| Error::IntegerRange)
    }

    pub fn waitlist_entries(&self) -> Result<Vec<WaitlistEntry>> {
        waitlist_entries(&self.connection)
    }
}

impl ReadDatabase {
    /// Every entry, oldest first.
    pub fn waitlist_entries(&self) -> Result<Vec<WaitlistEntry>> {
        waitlist_entries(&self.connection)
    }
}

fn waitlist_entries(connection: &Connection) -> Result<Vec<WaitlistEntry>> {
    let mut statement = connection.prepare(
        "SELECT waitlist_id, email, created_at FROM waitlist_entries
         ORDER BY created_at, rowid",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.budgeted_get::<_, String>(1)?,
            row.budgeted_get::<_, i64>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (id, email, created_at) = row?;
        Ok(WaitlistEntry {
            waitlist_id: stored_id(id)?,
            email,
            created_at: unsigned(created_at)?,
        })
    })
    .collect()
}

fn stored_id(id: Vec<u8>) -> Result<[u8; WAITLIST_ID_BYTES]> {
    id.try_into()
        .map_err(|_| Error::Invalid("stored waitlist identifier"))
}

fn valid_email(email: &str) -> bool {
    email.len() <= 320
        && email.len() >= 3
        && !email.chars().any(char::is_whitespace)
        && !email.chars().any(char::is_control)
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
}

fn integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::Invalid("integer exceeds SQLite range"))
}

fn map_duplicate(error: rusqlite::Error, subject: &'static str) -> Error {
    if matches!(
        &error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                || inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    ) {
        Error::Duplicate(subject)
    } else {
        Error::Sql(error)
    }
}
