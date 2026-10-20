use crate::allocation::RowAllocationExt as _;
use rusqlite::{Connection, TransactionBehavior};

use crate::{Error, Result};

pub const APPLICATION_ID: i64 = 0x464f_4b53;
pub const SCHEMA_VERSION: i64 = 60;
#[path = "schema/chunk_incarnations.rs"]
mod chunk_incarnations;
#[path = "schema/kv_current.rs"]
mod kv_current;
#[path = "schema/kv_usage.rs"]
mod kv_usage;
#[path = "schema/read_dependency.rs"]
pub(crate) mod read_dependency;

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
    include_str!("schema/kv_accounting.sql"),
    include_str!("schema/kv_external.sql"),
    include_str!("schema/chunk_binding.sql"),
    include_str!("schema/chunk_ledger.sql"),
    include_str!("schema/realtime.sql"),
    include_str!("schema/realtime_invalidation.sql"),
    include_str!("schema/waitlist.sql"),
    include_str!("schema/log_upload.sql"),
    include_str!("schema/expiry_indexes.sql"),
);

fn create_domain_schema(connection: &rusqlite::Transaction<'_>) -> Result<()> {
    connection.execute_batch(SCHEMA)?;
    chunk_incarnations::migrate(connection)?;
    connection.execute_batch(include_str!("schema/chunk_authority.sql"))?;
    connection.execute_batch(include_str!("schema/chunk_publication.sql"))?;
    kv_current::migrate(connection)?;
    connection.execute_batch(include_str!("schema/kv_historical.sql"))?;
    kv_usage::migrate(connection)?;
    migrate_waitlist(connection)?;
    Ok(())
}

/// Version 60 makes waitlist addresses unique under ASCII case folding. The
/// earliest entry for each address is kept and later duplicates are removed
/// before the unique index is built and the entry count is initialised.
fn migrate_waitlist(connection: &rusqlite::Transaction<'_>) -> Result<()> {
    connection.execute_batch(
        "DELETE FROM waitlist_entries WHERE rowid IN (
             SELECT entry FROM (
                 SELECT rowid AS entry, row_number() OVER (
                     PARTITION BY email COLLATE NOCASE
                     ORDER BY created_at, rowid
                 ) AS position
                 FROM waitlist_entries
             ) WHERE position > 1
         );
         DROP INDEX waitlist_entries_email_created;",
    )?;
    connection.execute_batch(include_str!("schema/waitlist_indexes.sql"))?;
    connection.execute_batch(include_str!("schema/waitlist_count.sql"))?;
    Ok(())
}

/// Non-serving, in-memory DDL reference. Its fixed coordinate is never published
/// or returned to callers; reopening a real store does not require fresh entropy.
pub(crate) fn reference_connection() -> Result<Connection> {
    let mut connection = Connection::open_in_memory()?;
    let transaction = connection.transaction()?;
    create_domain_schema(&transaction)?;
    read_dependency::install(&transaction, &[1; 32])?;
    transaction.commit()?;
    Ok(connection)
}

pub(crate) fn initialize(connection: &mut Connection) -> Result<()> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.budgeted_get(0))?;
    let version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.budgeted_get(0))?;
    if application_id == 0 && version == 0 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        create_domain_schema(&transaction)?;
        read_dependency::migrate(&transaction)?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        crate::read_dependency::verify_in_transaction(&transaction)?;
        transaction.commit()?;
        return Ok(());
    }
    if application_id == APPLICATION_ID && matches!(version, 43..=59) {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Recheck under the write lock: another opener may already have upgraded.
        let application_id =
            transaction.pragma_query_value(None, "application_id", |r| r.budgeted_get(0))?;
        let version =
            transaction.pragma_query_value(None, "user_version", |r| r.budgeted_get(0))?;
        if application_id == APPLICATION_ID && matches!(version, 43..=59) {
            if version == 49 && !has_kv_accounting(&transaction)? {
                // Before the celld port this branch numbered its waitlist step
                // 49. Such a store has no accounting columns and cannot take
                // the version-49 upgrade path; it must be recreated.
                return Err(Error::SchemaVersion { found: version });
            }
            if version <= 47 {
                if version == 43 {
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
                }
                if version <= 44 {
                    // A failed index build also rolls back the version-43
                    // reconciliation work.
                    transaction.execute_batch(include_str!("schema/expiry_indexes.sql"))?;
                }
                let has_fence: bool = transaction.query_row(
                    "SELECT EXISTS(
                    SELECT 1 FROM pragma_table_info('sso_policy') WHERE name='fence'
                )",
                    [],
                    |row| row.budgeted_get(0),
                )?;
                if has_fence {
                    transaction.execute_batch(
                        "ALTER TABLE sso_policy RENAME COLUMN fence TO blocked_reason;",
                    )?;
                }
                let has_admission_hash: bool = transaction.query_row(
                    "SELECT EXISTS(
                    SELECT 1 FROM pragma_table_info('sso_sessions')
                    WHERE name='admission_hash'
                )",
                    [],
                    |row| row.budgeted_get(0),
                )?;
                if has_admission_hash {
                    transaction.execute_batch(
                        "DROP INDEX sso_session_admission;
                     ALTER TABLE sso_sessions
                     RENAME COLUMN admission_hash TO source_hash;
                     CREATE INDEX sso_session_source
                     ON sso_sessions(host, source_hash, expires_at_ms);
                     ALTER TABLE sso_identity_challenges
                     RENAME COLUMN admission_hash TO source_hash;",
                    )?;
                }
                // Version 48 changes the SSO provider fingerprint and the
                // authenticated-encryption domains. Old browser flows cannot be
                // resumed and old account links must be established again under
                // the new provider identity.
                transaction.execute_batch(
                    "DELETE FROM sso_binding_receipts;
                 DELETE FROM sso_identity_challenges;
                 DELETE FROM sso_access;
                 DELETE FROM sso_sessions;",
                )?;
            }
            if version <= 48 {
                transaction.execute_batch(include_str!("schema/kv_accounting.sql"))?;
            }
            if version <= 49 {
                transaction.execute_batch(include_str!("schema/kv_external.sql"))?;
            }
            if version <= 50 {
                transaction.execute_batch(include_str!("schema/chunk_binding.sql"))?;
            }
            if version <= 51 {
                transaction.execute_batch(include_str!("schema/chunk_ledger.sql"))?;
            }
            if version <= 52 {
                chunk_incarnations::migrate(&transaction)?;
            }
            if version <= 53 {
                transaction.execute_batch(include_str!("schema/chunk_authority.sql"))?;
            }
            if version <= 54 {
                transaction.execute_batch(include_str!("schema/chunk_publication.sql"))?;
            }
            if version <= 55 {
                kv_current::migrate(&transaction)?;
            }
            if version <= 56 {
                transaction.execute_batch(include_str!("schema/kv_historical.sql"))?;
            }
            if version <= 57 {
                kv_usage::migrate(&transaction)?;
            }
            if version <= 58 {
                read_dependency::migrate(&transaction)?;
            }
            migrate_waitlist(&transaction)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else {
            validate(application_id, version)?;
        }
        crate::read_dependency::verify_in_transaction(&transaction)?;
        transaction.commit()?;
        return Ok(());
    }
    validate(application_id, version)
}

fn has_kv_accounting(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info('kv_namespaces') WHERE name='accounted_bytes'
        )",
        [],
        |row| row.budgeted_get(0),
    )?)
}

pub(crate) fn validate_connection(connection: &Connection) -> Result<()> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.budgeted_get(0))?;
    let version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.budgeted_get(0))?;
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

#[cfg(test)]
mod publication_migration_tests {
    use super::*;

    #[test]
    fn genuine_schema54_migration_preserves_unknown_success_and_schema_parity() {
        let mut old = Connection::open_in_memory().unwrap();
        old.pragma_update(None, "foreign_keys", true).unwrap();
        let tx = old.transaction().unwrap();
        tx.execute_batch(SCHEMA).unwrap();
        chunk_incarnations::migrate(&tx).unwrap();
        tx.execute_batch(include_str!("schema/chunk_authority.sql"))
            .unwrap();
        tx.pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        tx.pragma_update(None, "user_version", 54).unwrap();
        tx.execute_batch(
            "INSERT INTO chunk_objects(object_id,object_key,digest,encoded_bytes,state)
            VALUES(zeroblob(32),'legacy',zeroblob(32),10,3);
            INSERT INTO chunk_object_attempts(attempt_id,object_id,namespace_id,outcome,created_at)
            VALUES(zeroblob(16),zeroblob(32),zeroblob(16),3,1);",
        )
        .unwrap();
        tx.commit().unwrap();
        initialize(&mut old).unwrap();
        let row: (i64, i64) = old
            .query_row(
                "SELECT publication_policy,outcome FROM chunk_object_attempts",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, (0, 3));
        initialize(&mut old).unwrap(); // reopen does not reseed provenance
        let mut fresh = Connection::open_in_memory().unwrap();
        initialize(&mut fresh).unwrap();
        fn schema(c: &Connection) -> Vec<(String, Option<String>)> {
            c.prepare("SELECT name,sql FROM sqlite_master ORDER BY name")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        }
        assert_eq!(schema(&old), schema(&fresh));
        assert_eq!(
            old.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
    }
}
