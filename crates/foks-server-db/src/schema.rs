use rusqlite::{Connection, TransactionBehavior};

use crate::{Error, Result};

pub const APPLICATION_ID: i64 = 0x464f_4b53;
pub const SCHEMA_VERSION: i64 = 44;

const SCHEMA: &str = concat!(
    include_str!("schema/core.sql"),
    include_str!("schema/sso.sql"),
    include_str!("schema/identity.sql"),
    include_str!("schema/invites.sql"),
    include_str!("schema/web_admin.sql"),
    include_str!("schema/passphrases.sql"),
    include_str!("schema/yubi.sql"),
    include_str!("schema/team.sql"),
    include_str!("schema/team_invitations.sql"),
    include_str!("schema/generic.sql"),
    include_str!("schema/merkle.sql"),
    include_str!("schema/certificates.sql"),
    include_str!("schema/receipts.sql"),
    include_str!("schema/capabilities.sql"),
    include_str!("schema/federation.sql"),
    include_str!("schema/kv.sql"),
    include_str!("schema/realtime.sql"),
    include_str!("schema/realtime_invalidation.sql"),
    include_str!("schema/peripheral.sql"),
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
    if application_id == APPLICATION_ID && version == 43 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Recheck under the write lock, before changing anything.
        let application_id =
            transaction.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version = transaction.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if application_id == APPLICATION_ID && version == 43 {
            transaction.execute_batch(
                "ALTER TABLE rt_user_inboxes ADD COLUMN reconcile_dirty INTEGER NOT NULL
                 DEFAULT 1 CHECK (reconcile_dirty IN (0,1));",
            )?;
            transaction.execute_batch(include_str!("schema/realtime_invalidation.sql"))?;
            transaction.execute(
                "UPDATE rt_user_inboxes SET reconcile_dirty=1,
                 reconcile_after=NULL,reconcile_memberships=NULL",
                [],
            )?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else {
            validate(application_id, version)?;
        }
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
