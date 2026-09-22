use rusqlite::params;

use crate::{Database, Error, Result};

impl Database {
    pub fn join_waitlist(&mut self, id: &[u8; 13], email: &str, now: u64) -> Result<()> {
        if id[0] != 1 || !valid_email(email) {
            return Err(Error::Invalid("waitlist entry"));
        }
        self.connection
            .execute(
                "INSERT INTO waitlist_entries(waitlist_id, email, created_at)
                 VALUES (?1, ?2, ?3)",
                params![id, email, integer(now)?],
            )
            .map_err(|error| map_duplicate(error, "waitlist entry"))?;
        Ok(())
    }
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
