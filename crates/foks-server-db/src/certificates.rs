use rusqlite::{params, OptionalExtension as _};

use crate::{error::sql_integer, Database, Error, ReadDatabase, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredCertificate {
    pub not_before: u64,
    pub not_after: u64,
    pub exact_certificate: Vec<u8>,
}

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

    pub fn certificate_for_device(
        &self,
        uid: &[u8],
        device_id: &[u8],
    ) -> Result<Option<StoredCertificate>> {
        self.connection
            .query_row(
                "SELECT not_before, not_after, exact_certificate FROM issued_certificates
                 WHERE uid = ?1 AND device_id = ?2 ORDER BY not_after DESC LIMIT 1",
                params![uid, device_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(Into::into)
            .and_then(|stored| {
                stored
                    .map(|(not_before, not_after, exact_certificate)| {
                        Ok(StoredCertificate {
                            not_before: crate::error::unsigned(not_before)?,
                            not_after: crate::error::unsigned(not_after)?,
                            exact_certificate,
                        })
                    })
                    .transpose()
            })
    }

    pub fn is_active_device(&self, uid: &[u8], device_id: &[u8]) -> Result<bool> {
        is_active_device(&self.connection, uid, device_id)
    }
}

impl ReadDatabase {
    pub fn is_active_device(&self, uid: &[u8], device_id: &[u8]) -> Result<bool> {
        is_active_device(&self.connection, uid, device_id)
    }
}

fn is_active_device(
    connection: &rusqlite::Connection,
    uid: &[u8],
    device_id: &[u8],
) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM devices WHERE uid = ?1 AND device_id = ?2 AND active = 1",
            params![uid, device_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
