use rusqlite::{params, OptionalExtension as _};

use crate::{Database, Error, ReadDatabase, ReadSnapshot, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceNagSnapshot {
    pub num_active_devices: u64,
    pub cleared: bool,
}

impl Database {
    pub fn set_device_nag_cleared(
        &mut self,
        uid: &[u8],
        credential_id: &[u8],
        cleared: bool,
    ) -> Result<()> {
        let updated = self.connection.execute(
            "UPDATE users SET device_nag_cleared = ?3
             WHERE uid = ?1 AND EXISTS (
                 SELECT 1 FROM devices
                 WHERE uid = ?1 AND active = 1
                   AND (device_id = ?2 OR subkey_id = ?2)
             )",
            params![uid, credential_id, cleared],
        )?;
        if updated != 1 {
            return Err(Error::AuthorizationChanged);
        }
        Ok(())
    }

    pub fn device_nag(&self, uid: &[u8]) -> Result<Option<DeviceNagSnapshot>> {
        device_nag(&self.connection, uid)
    }
}

impl ReadDatabase {
    pub fn device_nag(&self, uid: &[u8]) -> Result<Option<DeviceNagSnapshot>> {
        device_nag(&self.connection, uid)
    }
}

impl ReadSnapshot<'_> {
    pub fn device_nag(&self, uid: &[u8]) -> Result<Option<DeviceNagSnapshot>> {
        device_nag(self.connection(), uid)
    }
}

fn device_nag(connection: &rusqlite::Connection, uid: &[u8]) -> Result<Option<DeviceNagSnapshot>> {
    connection
        .query_row(
            "SELECT u.device_nag_cleared, count(d.device_id)
             FROM users u
             LEFT JOIN devices d ON d.uid = u.uid AND d.active = 1
             WHERE u.uid = ?1
             GROUP BY u.uid, u.device_nag_cleared",
            [uid],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?
        .map(|(cleared, num_active_devices)| {
            let cleared = match cleared {
                0 => false,
                1 => true,
                _ => return Err(Error::Invalid("device nag cleared flag")),
            };
            Ok(DeviceNagSnapshot {
                num_active_devices: crate::error::unsigned(num_active_devices)?,
                cleared,
            })
        })
        .transpose()
}
