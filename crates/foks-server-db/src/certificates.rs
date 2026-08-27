use rusqlite::{params, OptionalExtension as _};

use crate::{error::sql_integer, Database, Error, Result};

impl Database {
    pub fn record_certificate(
        &mut self,
        serial: &[u8],
        uid: &[u8],
        device_id: &[u8],
        not_before: u64,
        not_after: u64,
        certificate: &[u8],
    ) -> Result<()> {
        if !(1..=20).contains(&serial.len()) || not_after <= not_before {
            return Err(Error::Invalid("certificate record"));
        }
        self.connection.execute(
            "INSERT INTO issued_certificates
             (serial, uid, device_id, not_before, not_after, exact_certificate)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                serial,
                uid,
                device_id,
                sql_integer(not_before)?,
                sql_integer(not_after)?,
                certificate
            ],
        )?;
        Ok(())
    }

    pub fn certificate_for_device(&self, uid: &[u8], device_id: &[u8]) -> Result<Option<Vec<u8>>> {
        self.connection
            .query_row(
                "SELECT exact_certificate FROM issued_certificates
                 WHERE uid = ?1 AND device_id = ?2 ORDER BY not_after DESC LIMIT 1",
                params![uid, device_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }
}
