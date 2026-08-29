use rusqlite::{Connection, TransactionBehavior};

use crate::{Error, Result};

pub const APPLICATION_ID: i64 = 0x464f_4b53;
pub const SCHEMA_VERSION: i64 = 25;

const SCHEMA: &str = concat!(
    include_str!("schema/core.sql"),
    include_str!("schema/identity.sql"),
    include_str!("schema/invites.sql"),
    include_str!("schema/passphrases.sql"),
    include_str!("schema/yubi.sql"),
    include_str!("schema/team.sql"),
    include_str!("schema/merkle.sql"),
    include_str!("schema/certificates.sql"),
    include_str!("schema/receipts.sql"),
    include_str!("schema/capabilities.sql"),
    include_str!("schema/federation.sql"),
    include_str!("schema/kv.sql"),
);

pub(crate) fn initialize(connection: &mut Connection) -> Result<()> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id == 0 && version == 0 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA)?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;
        return Ok(());
    }
    validate(application_id, version)
}

pub(crate) fn validate_connection(connection: &Connection) -> Result<()> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    validate(application_id, version)
}

fn validate(application_id: i64, version: i64) -> Result<()> {
    if application_id != APPLICATION_ID {
        return Err(Error::ApplicationId {
            found: application_id,
        });
    }
    if version != SCHEMA_VERSION {
        return Err(Error::SchemaVersion { found: version });
    }
    Ok(())
}
