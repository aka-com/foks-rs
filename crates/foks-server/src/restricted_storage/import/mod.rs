//! Real native import qualification. No public readiness capability or production route.
mod certificates;
mod copy;
mod keys;
#[path = "../../restricted_list/mod.rs"]
pub(crate) mod list;
mod policy;
mod semantic;
#[cfg(test)]
mod tests;

use super::{capture, scan, Result, ScanControl, ScanError, StorageScanComplete, StorageSummary};
use foks_schema_contract::storage::{Component, HOSTED_SMALL_V1};
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use std::{path::PathBuf, sync::Arc};
use zeroize::Zeroizing;

fn unsupported(reason: &'static str) -> ScanError {
    ScanError::Unsupported(reason)
}
fn server(error: crate::Error) -> ScanError {
    match error {
        crate::Error::Io(error) => ScanError::Io(error),
        crate::Error::Database(foks_server_db::Error::Capacity(_)) => ScanError::Allocation,
        crate::Error::Database(foks_server_db::Error::Sql(error)) => error.into(),
        crate::Error::Database(foks_server_db::Error::Io(error)) => error.into(),
        crate::Error::Protocol(error) => protocol(error),
        crate::Error::Snowpack(error) => snowpack(error),
        crate::Error::Verify(error) => verification(error),
        _ => ScanError::Incomplete,
    }
}

fn snowpack(error: foks_snowpack::Error) -> ScanError {
    if error.kind == foks_snowpack::ErrorKind::AllocationLimit {
        ScanError::Allocation
    } else {
        unsupported("import canonical encoding")
    }
}
fn protocol(error: foks_proto::Error) -> ScanError {
    match error {
        foks_proto::Error::Snowpack(error) => snowpack(error),
        _ => unsupported("import protocol validation"),
    }
}
fn verification(error: foks_verify::Error) -> ScanError {
    match error {
        foks_verify::Error::Snowpack(error) => snowpack(error),
        foks_verify::Error::Protocol(error) => protocol(error),
        _ => unsupported("import public host verification"),
    }
}

/// Fixed qualification diagnostics. This is not a readiness token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportQualification {
    pub source_digest: [u8; 32],
    pub policy_digest: [u8; 32],
    pub target_incarnation: [u8; 32],
    pub target_revision: u64,
    pub rows: u64,
    pub authority_users: u64,
}

/// Field order is deliberate: close SQLite before releasing every physical lease.
/// No public constructor, serialization, clone, path or raw connection loan.
pub(crate) struct NativePersonalListImport {
    database: scan::Database,
    config: foks_server_db::Config,
    identity: foks_server_db::DatabasePathIdentity,
    keys: crate::keys::DirectoryKeyProvider,
    writer: crate::DatabaseWriterGuard,
    storage: crate::storage_lease::StorageLease,
    directory: tempfile::TempDir,
    qualification: ImportQualification,
    binding: ImportBinding,
}
/// Created only after the real imported key inventory and public host are verified.
/// This remains inside the consumed owner; no digest or identity getter mints readiness.
struct ImportBinding {
    host: [u8; 33],
    canonical_name: String,
    key_bundle_digest: [u8; 32],
}
impl NativePersonalListImport {
    fn verify_owned(&self) -> Result<()> {
        self.identity
            .recheck()
            .map_err(|_| unsupported("import destination identity"))?;
        self.database.control().check()?;
        // All retained resources are intentional, not borrowed readiness. The
        // consuming List constructor keeps the original job through construction
        // and moves the connection and these leases together into its closed owner.
        let _retained = (
            &self.config,
            &self.keys,
            &self.writer,
            &self.storage,
            &self.directory,
        );
        Ok(())
    }
}

#[cfg(feature = "celld-proof")]
pub fn qualify_personal_list_import(
    artifacts: &crate::BackupArtifacts,
    root_key: Zeroizing<[u8; 32]>,
    config: foks_server_db::Config,
    control: &ScanControl,
) -> Result<ImportQualification> {
    let mut scanner = super::RestrictedScanner::capture_finalized_backup(
        artifacts,
        super::CaptureLimits {
            maximum_database_bytes: config.maximum_database_bytes.min(super::MAX_DATABASE),
        },
        control,
    )?;
    while !scanner.advance(control)?.complete {}
    let source = scanner.finish(control)?;
    let owner = import_personal_list(source, root_key, config, control)?;
    owner.verify_owned()?;
    let result = owner.qualification;
    drop(owner);
    control.check()?;
    Ok(result)
}

fn triggers(db: &scan::Database) -> Result<Vec<(String, String)>> {
    db.verify_schema()?;
    db.control().statement()?;
    let mut stmt = db
        .connection()
        .prepare("SELECT name,sql FROM main.sqlite_schema WHERE type='trigger' ORDER BY name")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    let mut total = 0usize;
    while let Some(row) = rows.next()? {
        db.control().check()?;
        if out.len() >= 1024 {
            return Err(unsupported("trusted trigger count"));
        }
        let name = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| unsupported("trusted trigger name"))?;
        let sql = row
            .get_ref(1)?
            .as_str()
            .map_err(|_| unsupported("trusted trigger SQL"))?;
        total = total.checked_add(sql.len()).ok_or(ScanError::Allocation)?;
        if name.len() > 256 || sql.len() > 65536 || total > 2 * 1024 * 1024 {
            return Err(unsupported("trusted trigger bytes"));
        }
        let mut n = String::new();
        n.try_reserve_exact(name.len())
            .map_err(|_| ScanError::Allocation)?;
        n.push_str(name);
        let mut s = String::new();
        s.try_reserve_exact(sql.len())
            .map_err(|_| ScanError::Allocation)?;
        s.push_str(sql);
        out.try_reserve(1).map_err(|_| ScanError::Allocation)?;
        out.push((n, s));
    }
    Ok(out)
}
fn set_fk(db: &scan::Database, on: bool) -> Result<()> {
    db.control().statement()?;
    db.connection().pragma_update(None, "foreign_keys", on)?;
    db.control().statement()?;
    let observed: i64 = db
        .connection()
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
    if observed != i64::from(on) {
        return Err(unsupported("import FK setting"));
    }
    Ok(())
}
/// Upper bound, over the tables of the verified schema, of one plus the
/// table's index count plus its foreign-key count. Checked against the schema
/// by a unit test.
const MAXIMUM_TABLE_FANOUT: u64 = 10;
/// Opcodes allowed per row and per b-tree or foreign key. integrity_check
/// measured about 12 per row per b-tree on the bundled SQLite;
/// foreign_key_check and the policy's full-table scans cost less.
const WHOLE_DATABASE_OPERATIONS_PER_ROW: u64 = 64;

/// Opcode budget of one statement that may visit every row of the database
/// (integrity_check, foreign_key_check, the policy's full-table scans). The
/// keyset probes keep `STATEMENT_OPERATIONS`; that cap is crossed by
/// integrity_check at a few tens of thousands of rows.
fn whole_database_operations(rows: u64) -> usize {
    let operations = rows
        .saturating_mul(WHOLE_DATABASE_OPERATIONS_PER_ROW * MAXIMUM_TABLE_FANOUT)
        .saturating_add(super::STATEMENT_OPERATIONS as u64);
    usize::try_from(operations).unwrap_or(usize::MAX)
}

fn integrity(db: &scan::Database, operations: usize) -> Result<()> {
    db.control().statement_with_budget(operations)?;
    {
        let mut statement = db.connection().prepare("PRAGMA main.foreign_key_check")?;
        let mut rows = statement.query([])?;
        if rows.next()?.is_some() {
            return Err(unsupported("import foreign key violation"));
        }
    }
    db.control().statement_with_budget(operations)?;
    let mut stmt = db.connection().prepare("PRAGMA main.integrity_check(1)")?;
    let mut rows = stmt.query([])?;
    let row = rows
        .next()?
        .ok_or(unsupported("import integrity result missing"))?;
    if row.get_ref(0)? != ValueRef::Text(b"ok") {
        return Err(unsupported("import integrity failure"));
    }
    if rows.next()?.is_some() {
        return Err(unsupported("import integrity extra result"));
    }
    db.control().check()
}
fn rescan(db: &scan::Database, package: &capture::Package) -> Result<StorageSummary> {
    let mut cursor = scan::Cursor::default();
    let mut summary = StorageSummary {
        tables: 0,
        rows: 0,
        maximum_table_bytes: 0,
        maximum_index_bytes: 0,
        external_chunk_references: 0,
        admin_audit_sequence: None,
    };
    while !db.advance(&mut cursor, &mut summary, super::STEP_ROWS)? {}
    summary.admin_audit_sequence = db.sequence(&cursor)?;
    db.verify_companions(package, &cursor)?;
    Ok(summary)
}

pub(crate) fn import_personal_list(
    mut source: StorageScanComplete,
    root_key: Zeroizing<[u8; 32]>,
    config: foks_server_db::Config,
    control: &ScanControl,
) -> Result<NativePersonalListImport> {
    // Every read helper finishes/drops its cursor before another statement resets
    // this shared per-statement counter. No source/target cursor crosses the boundary.
    let job = source._database.control();
    job.intersect(control)?;
    source._package.verify(&job)?;
    source._database.verify_schema()?;
    if source.summary.external_chunk_references != 0 {
        return Err(unsupported("import external chunks"));
    }
    let whole_database = whole_database_operations(source.summary.rows);
    policy::verify(&source._database, whole_database)?;
    let source_coordinate = semantic::coordinate(&source._database)?;
    let directory = tempfile::Builder::new()
        .prefix("foks-personal-import-")
        .tempdir_in(std::env::temp_dir().canonicalize()?)?;
    let path = directory.path().join("foks-server.sqlite");
    job.check()?;
    let writer = crate::DatabaseWriterGuard::acquire(&path).map_err(server)?;
    let fresh = {
        let fresh = writer.open_database(config.clone()).map_err(server)?;
        fresh.read_dependency().map_err(|_| ScanError::Incomplete)?
    };
    if fresh.incarnation == source_coordinate.incarnation
        || fresh.incarnation == [0; 32]
        || fresh.revision != 1
    {
        return Err(unsupported("fresh import coordinate"));
    }
    job.check()?;
    let identity = foks_server_db::DatabasePathIdentity::existing(&path)
        .map_err(|_| unsupported("import path identity"))?;
    let storage = crate::storage_lease::StorageLease::with_store(identity.path(), None, true)
        .map_err(server)?;
    identity
        .recheck()
        .map_err(|_| unsupported("import identity before open"))?;
    let connection = Connection::open_with_flags(
        identity.path(),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    identity
        .recheck()
        .map_err(|_| unsupported("import identity after open"))?;
    connection.busy_timeout(std::time::Duration::ZERO)?;
    let target = scan::Database::private_target(connection, Arc::clone(&job))?;
    #[cfg(test)]
    tests::stage(tests::Stage::Opened, &path)?;
    let trusted_triggers = triggers(&target)?;
    set_fk(&target, false)?;
    job.statement()?;
    target.connection().execute_batch("BEGIN IMMEDIATE")?;
    // On any early return, connection destruction rolls back before its directory/leases.
    for (name, _) in &trusted_triggers {
        job.statement()?;
        target
            .connection()
            .execute(&format!("DROP TRIGGER main.{}", copy::quote(name)), [])?;
    }
    for table in HOSTED_SMALL_V1.descriptor(Component::Foks).tables() {
        job.statement()?;
        target.connection().execute(
            &format!("DELETE FROM main.{}", copy::quote(table.name())),
            [],
        )?;
    }
    job.statement()?;
    target
        .connection()
        .execute("DELETE FROM main.sqlite_sequence", [])?;
    let rows = copy::copy_all(&source._database, &target)?;
    copy::sequence(&target, source.summary.admin_audit_sequence)?;
    #[cfg(test)]
    tests::stage(tests::Stage::Copied, &path)?;
    if rows != source.summary.rows || copy::compare(&source._database, &target, None)? != rows {
        return Err(unsupported("import source count changed"));
    }
    if semantic::coordinate(&target)? != source_coordinate {
        return Err(unsupported("copied source coordinate mismatch"));
    }
    job.statement()?;
    let changed = target.connection().execute(
        "UPDATE main.metadata_read_dependency SET incarnation=?1,revision=1 WHERE singleton=1 AND incarnation=?2 AND revision=?3",
        rusqlite::params![
            fresh.incarnation.as_slice(),
            source_coordinate.incarnation.as_slice(),
            i64::try_from(source_coordinate.revision).map_err(|_| unsupported("source coordinate range"))?,
        ],
    )?;
    if changed != 1 {
        return Err(unsupported("import coordinate CAS"));
    }
    if semantic::coordinate(&target)? != fresh {
        return Err(unsupported("import fresh coordinate readback"));
    }
    for (_, sql) in &trusted_triggers {
        job.statement()?;
        target.connection().execute_batch(sql)?;
    }
    if copy::compare(&source._database, &target, Some(&fresh))? != rows {
        return Err(unsupported("import deliberate difference count"));
    }
    copy::verify_sequence(&target, source.summary.admin_audit_sequence)?;
    job.statement()?;
    target.connection().execute_batch("COMMIT")?;
    set_fk(&target, true)?;
    integrity(&target, whole_database)?;
    target.verify_schema()?;
    policy::verify(&target, whole_database)?;
    let audited = rescan(&target, &source._package)?;
    if audited != source.summary {
        return Err(unsupported("import storage summary mismatch"));
    }
    let authority_users = semantic::verify(&target)?;
    #[cfg(test)]
    tests::stage(tests::Stage::Audited, &path)?;
    let key_path: PathBuf = directory.path().join("keys");
    source._package.copy_keys(&key_path, &job)?;
    let keys =
        crate::keys::DirectoryKeyProvider::open_existing(&key_path, root_key).map_err(server)?;
    let (host, canonical_name) = keys::validate_existing_import(&target, &keys, &source._package)?;
    #[cfg(test)]
    tests::stage(tests::Stage::Keys, &path)?;
    let key_bundle_digest = source._package.key_bundle_digest(&job)?;
    identity
        .recheck()
        .map_err(|_| unsupported("import final identity"))?;
    job.check()?;
    let qualification = ImportQualification {
        source_digest: source.digest,
        policy_digest: policy::fingerprint(),
        target_incarnation: fresh.incarnation,
        target_revision: fresh.revision,
        rows,
        authority_users,
    };
    let owner = NativePersonalListImport {
        database: target,
        config,
        identity,
        keys,
        writer,
        storage,
        directory,
        qualification,
        binding: ImportBinding {
            host,
            canonical_name,
            key_bundle_digest,
        },
    };
    owner.verify_owned()?;
    Ok(owner)
}
