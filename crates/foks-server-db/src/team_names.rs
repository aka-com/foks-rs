use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, Result};

impl Database {
    pub fn reserve_team_name(
        &mut self,
        normalized_name: &[u8],
        token: &[u8; 17],
        sequence: u64,
        now: u64,
        expires_at: u64,
    ) -> Result<()> {
        if normalized_name.is_empty()
            || normalized_name.len() > self.config.maximum_name_bytes
            || sequence == 0
            || expires_at <= now
        {
            return Err(Error::Invalid("team-name reservation"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM team_names WHERE team_id IS NULL AND expires_at <= ?1",
            [sql_integer(now)?],
        )?;
        let count: i64 = transaction.query_row(
            "SELECT count(*) FROM team_names WHERE team_id IS NULL",
            [],
            |row| row.get(0),
        )?;
        if usize::try_from(count).unwrap_or(usize::MAX)
            >= self.config.maximum_team_name_reservations
        {
            return Err(Error::QuotaExceeded);
        }
        let existing: Option<(Option<Vec<u8>>, Option<i64>)> = transaction
            .query_row(
                "SELECT team_id, expires_at FROM team_names WHERE normalized_name = ?1",
                [normalized_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if existing.is_some() {
            return Err(Error::NameInUse);
        }
        transaction.execute(
            "INSERT INTO team_names
             (normalized_name, reservation_token, reservation_sequence, expires_at, team_id)
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![
                normalized_name,
                token,
                sql_integer(sequence)?,
                sql_integer(expires_at)?
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}
