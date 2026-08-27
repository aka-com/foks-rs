use rusqlite::{params, OptionalExtension as _};

use crate::{error::sql_integer, Database, Error, ReadDatabase, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredCertificate {
    pub not_before: u64,
    pub not_after: u64,
    pub exact_certificate: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredCredentialBinding {
    pub uid: Vec<u8>,
    pub credential_id: Vec<u8>,
    pub role_type: u64,
    pub visibility: i64,
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn record_certificate(
        &mut self,
        serial: &[u8],
        uid: &[u8],
        device_id: &[u8],
        credential_id: &[u8],
        not_before: u64,
        not_after: u64,
        certificate: &[u8],
    ) -> Result<()> {
        if !(1..=20).contains(&serial.len()) || not_after <= not_before {
            return Err(Error::Invalid("certificate record"));
        }
        self.connection.execute(
            "INSERT INTO issued_certificates
             (serial, uid, device_id, credential_id, not_before, not_after, exact_certificate)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                serial,
                uid,
                device_id,
                credential_id,
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
                 WHERE uid = ?1 AND credential_id = ?2 ORDER BY not_after DESC LIMIT 1",
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

    pub fn active_credential_owner(
        &self,
        uid: &[u8],
        credential_id: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        active_credential_owner(&self.connection, uid, credential_id)
    }
}

impl ReadDatabase {
    pub fn is_active_device(&self, uid: &[u8], device_id: &[u8]) -> Result<bool> {
        is_active_device(&self.connection, uid, device_id)
    }

    pub fn credential_for_certificate(
        &self,
        exact_certificate: &[u8],
        now: u64,
    ) -> Result<Option<StoredCredentialBinding>> {
        self.connection
            .query_row(
                "SELECT d.uid, c.credential_id, d.role_type, d.visibility
                 FROM issued_certificates c
                 JOIN devices d ON d.device_id = c.device_id AND d.uid = c.uid
                 WHERE c.exact_certificate = ?1 AND c.not_before <= ?2 AND c.not_after > ?2
                   AND d.active = 1",
                params![exact_certificate, sql_integer(now)?],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?
            .map(|(uid, credential_id, role_type, visibility)| {
                Ok(StoredCredentialBinding {
                    uid,
                    credential_id,
                    role_type: crate::error::unsigned(role_type)?,
                    visibility,
                })
            })
            .transpose()
    }

    pub fn active_credential_owner(
        &self,
        uid: &[u8],
        credential_id: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        active_credential_owner(&self.connection, uid, credential_id)
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

fn active_credential_owner(
    connection: &rusqlite::Connection,
    uid: &[u8],
    credential_id: &[u8],
) -> Result<Option<Vec<u8>>> {
    connection
        .query_row(
            "SELECT device_id FROM devices
             WHERE uid = ?1 AND active = 1 AND (device_id = ?2 OR subkey_id = ?2)",
            params![uid, credential_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}
