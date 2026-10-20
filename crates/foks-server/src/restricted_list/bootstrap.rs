//! Consume one real private import. No hashes, paths or caller-created authority
//! rows can substitute for this ownership transition.

use super::{
    certificate::{CertificateWorkspace, MAX_DER},
    connection::{FixedExpected, FIXED_CHECKS},
    profile::Profile,
};
use crate::{
    keys::{DirectoryKeyProvider, HostKeyProvider, KeyGenerationId, KeyPurpose, SecretKey},
    net::memory::{Pools, Request},
    restricted_storage::{Control, Result, ScanControl, ScanError},
};
use foks_metadata_core::{
    authority::{Activation, Identity, Number},
    Limits, Statement,
};
use foks_schema_contract::{MatchPolicy, SchemaObject};
use rusqlite::{types::ValueRef, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{io, sync::Arc};

use super::super::{certificates, ImportQualification, NativePersonalListImport};

const MAX_JSON: usize = 2048;
// Same source-reviewed PreviousKey growth allowance as the C4 streaming scan.
const VERIFIER_BYTES: usize = 2080;
// serde_json 1.0.151 boxes ErrorImpl { ErrorCode, line, column }. ErrorCode's
// largest payload is a two-word Box<str>; include its tag and both coordinates.
// Our writer produces only the nonallocating ErrorKind form of io::Error.
const JSON_ERROR_BYTES: usize = 5 * std::mem::size_of::<usize>();

pub(super) struct ExpectedState {
    pub(super) identity_json: String,
    pub(super) activation_json: String,
    pub(super) owner: [u8; 16],
    pub(super) floor: u64,
}

/// Drop order closes SQLite before releasing keys, locks or the private directory.
/// The construction memory reservation survives every allocation it admits.
pub(super) struct NativeOwner {
    pub(super) connection: Connection,
    pub(super) profile: Profile,
    pub(super) expected: ExpectedState,
    _config: foks_server_db::Config,
    identity: foks_server_db::DatabasePathIdentity,
    _keys: DirectoryKeyProvider,
    _writer: crate::DatabaseWriterGuard,
    _storage: crate::storage_lease::StorageLease,
    _directory: tempfile::TempDir,
    qualification: ImportQualification,
    canonical_name: String,
    _import_control: Arc<Control>,
    _construction: Arc<Request>,
}

impl NativeOwner {
    pub(super) fn qualification(&self) -> ImportQualification {
        self.qualification
    }

    pub(super) fn canonical_name(&self) -> &str {
        &self.canonical_name
    }

    pub(super) fn recheck_path(&self) -> foks_metadata_core::Result<()> {
        self.identity
            .recheck()
            .map_err(|_| foks_metadata_core::Error::Storage)
    }
}

fn unsupported(reason: &'static str) -> ScanError {
    ScanError::Unsupported(reason)
}

fn core(error: foks_metadata_core::Error) -> ScanError {
    match error {
        foks_metadata_core::Error::Bounds => ScanError::Allocation,
        _ => ScanError::Incomplete,
    }
}

fn reserve(owner: &Request, bytes: usize) -> Result<()> {
    if owner.failed() || !foks_snowpack::AllocationBudget::reserve(owner, bytes) {
        return Err(ScanError::Allocation);
    }
    Ok(())
}

struct JsonBuffer {
    bytes: [u8; MAX_JSON],
    length: usize,
}

impl io::Write for JsonBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .length
            .checked_add(bytes.len())
            .filter(|end| *end <= MAX_JSON)
            .ok_or_else(|| io::Error::from(io::ErrorKind::WriteZero))?;
        self.bytes[self.length..end].copy_from_slice(bytes);
        self.length = end;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json(value: &impl Serialize, owner: &Request) -> Result<String> {
    reserve(owner, JSON_ERROR_BYTES)?;
    let mut buffer = JsonBuffer {
        bytes: [0; MAX_JSON],
        length: 0,
    };
    serde_json::to_writer(&mut buffer, value)
        .map_err(|_| unsupported("bootstrap authority JSON bounds"))?;
    let bytes = &buffer.bytes[..buffer.length];
    if !bytes.is_ascii() {
        return Err(unsupported("bootstrap authority JSON encoding"));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| unsupported("bootstrap authority JSON encoding"))?;
    reserve(owner, text.len())?;
    let mut result = String::new();
    result
        .try_reserve_exact(text.len())
        .map_err(|_| ScanError::Allocation)?;
    result.push_str(text);
    Ok(result)
}

fn limits(config: &foks_server_db::Config) -> Result<Limits> {
    let bounded = |value: usize, maximum: u64| -> Result<usize> {
        let value =
            u64::try_from(value).map_err(|_| unsupported("bootstrap configured limit range"))?;
        usize::try_from(value.min(maximum))
            .map_err(|_| unsupported("bootstrap configured limit range"))
    };
    let limits = Limits {
        blob_bytes: bounded(config.maximum_kv_node_bytes, 65_536)?,
        dirent_bytes: bounded(config.maximum_kv_dirent_bytes, 4096)?,
        namespace_bytes: config.maximum_kv_namespace_bytes,
        namespace_objects: config.maximum_kv_namespace_objects,
        directories: config.maximum_kv_directories,
        dirents: config.maximum_kv_dirents,
    };
    if limits.blob_bytes == 0
        || limits.dirent_bytes == 0
        || limits.namespace_bytes == 0
        || limits.namespace_objects == 0
        || limits.directories == 0
        || limits.dirents == 0
    {
        return Err(unsupported("bootstrap configured zero limit"));
    }
    Ok(limits)
}

fn label(control: &Control) -> Result<[u8; 16]> {
    control.check()?;
    let mut value = [0; 16];
    getrandom::fill(&mut value).map_err(|_| unsupported("bootstrap OS entropy"))?;
    control.check()?;
    Ok(value)
}

fn state(
    import: &NativePersonalListImport,
    profile: &Profile,
    control: &Control,
    memory: &Request,
) -> Result<ExpectedState> {
    let installation = label(control)?;
    let tenant = label(control)?;
    let owner = label(control)?;
    let storage_namespace = label(control)?;
    let mut cell = Sha256::new();
    cell.update(b"foks-native-personal-list-cell-v1\0");
    cell.update(import.qualification.target_incarnation);
    cell.update(tenant);
    reserve(memory, import.binding.host.len())?;
    let mut host = Vec::new();
    host.try_reserve_exact(import.binding.host.len())
        .map_err(|_| ScanError::Allocation)?;
    host.extend_from_slice(&import.binding.host);
    let identity = Identity {
        installation,
        tenant,
        host,
        cell: cell.finalize().into(),
    };
    let mut placement = Sha256::new();
    placement.update(b"foks-native-personal-list-inline-placement-v1\0");
    placement.update(import.qualification.target_incarnation);
    placement.update(storage_namespace);
    placement.update(import.qualification.source_digest);
    let activation = Activation {
        version: Number(1),
        schema: Number(59),
        schema_hash: *profile.contract.hash(),
        kernel_hash: profile.digest,
        catalog_hash: profile.catalog_digest,
        key_generation: Number(1),
        key_digest: import.binding.key_bundle_digest,
        storage_namespace,
        storage_placement: placement.finalize().into(),
        limits: limits(&import.config)?,
    };
    // Number's existing canonical serializer calls i64::to_string. Its three
    // fixed values (1, 59, 1) each need at most String's pinned eight-byte minimum.
    reserve(memory, 3 * 8)?;
    let expected = ExpectedState {
        identity_json: json(&identity, memory)?,
        activation_json: json(&activation, memory)?,
        owner,
        floor: 0,
    };
    control.check()?;
    Ok(expected)
}

/// Construction has the original finite maintenance job, not a newly captured
/// request deadline. No row/statement borrow spans a statement counter reset.
fn settings(connection: &Connection, control: &Control) -> Result<()> {
    for check in FIXED_CHECKS {
        control.statement()?;
        let mut statement = connection.prepare(check.sql)?;
        if statement.column_count() != 1 {
            return Err(unsupported("bootstrap connection shape"));
        }
        let mut rows = statement.query([])?;
        let row = rows
            .next()?
            .ok_or(unsupported("bootstrap connection setting absent"))?;
        let matches = match (row.get_ref(0)?, check.expected) {
            (ValueRef::Integer(value), FixedExpected::Integer(expected)) => value == expected,
            (ValueRef::Text(value), FixedExpected::Text(expected)) => value == expected.as_bytes(),
            _ => false,
        };
        if !matches || rows.next()?.is_some() {
            return Err(unsupported("bootstrap connection setting"));
        }
        control.check()?;
    }
    Ok(())
}

fn stored_state(
    connection: &Connection,
    control: &Control,
    expected: &ExpectedState,
) -> Result<()> {
    control.statement()?;
    let mut statement = connection.prepare(Statement::AuthorityState.spec().sql)?;
    if statement.column_count() != 11 {
        return Err(unsupported("bootstrap authority shape"));
    }
    let mut rows = statement.query([])?;
    let row = rows
        .next()?
        .ok_or(unsupported("bootstrap authority absent"))?;
    if row.get_ref(0)? != ValueRef::Text(expected.identity_json.as_bytes())
        || row.get_ref(1)? != ValueRef::Text(expected.activation_json.as_bytes())
        || row.get_ref(2)? != ValueRef::Blob(&expected.owner)
        || expected.floor != 0
    {
        return Err(unsupported("bootstrap authority binding"));
    }
    for (column, value) in [b"1".as_slice(), b"1", b"0", b"0", b"0", b"1", b"4608", b"0"]
        .into_iter()
        .enumerate()
    {
        if row.get_ref(column + 3)? != ValueRef::Text(value) {
            return Err(unsupported("bootstrap authority coordinates"));
        }
    }
    if rows.next()?.is_some() {
        return Err(unsupported("bootstrap authority multiplicity"));
    }
    control.check()
}

fn empty_tables(connection: &Connection, control: &Control) -> Result<()> {
    for sql in [
        "SELECT EXISTS(SELECT 1 FROM main.metadata_principals)",
        "SELECT EXISTS(SELECT 1 FROM main.metadata_intents)",
    ] {
        control.statement()?;
        let mut statement = connection.prepare(sql)?;
        let mut rows = statement.query([])?;
        let row = rows
            .next()?
            .ok_or(unsupported("bootstrap empty table result"))?;
        if row.get_ref(0)? != ValueRef::Integer(0) || rows.next()?.is_some() {
            return Err(unsupported("bootstrap nonempty authority tables"));
        }
        control.check()?;
    }
    Ok(())
}

fn cookie(connection: &Connection, control: &Control) -> Result<i64> {
    control.statement()?;
    let mut statement = connection.prepare("PRAGMA main.schema_version")?;
    let mut rows = statement.query([])?;
    let row = rows
        .next()?
        .ok_or(unsupported("bootstrap schema cookie absent"))?;
    let ValueRef::Integer(value) = row.get_ref(0)? else {
        return Err(unsupported("bootstrap schema cookie type"));
    };
    if value < 0 || rows.next()?.is_some() {
        return Err(unsupported("bootstrap schema cookie range"));
    }
    control.check()?;
    Ok(value)
}

fn union(
    connection: &Connection,
    control: &Control,
    profile: &Profile,
    memory: &Request,
) -> Result<()> {
    reserve(memory, VERIFIER_BYTES)?;
    let mut verifier = profile.contract.verifier(MatchPolicy::ExactUnion);
    control.statement()?;
    let mut statement = connection.prepare("SELECT type,name,tbl_name,sql FROM main.sqlite_schema WHERE typeof(name)!='text' OR substr(name,1,7)!='sqlite_' COLLATE NOCASE ORDER BY type COLLATE BINARY,name COLLATE BINARY,tbl_name COLLATE BINARY LIMIT 1025")?;
    let mut rows = statement.query([])?;
    let mut count = 0usize;
    let mut total = 0usize;
    while let Some(row) = rows.next()? {
        control.check()?;
        count += 1;
        if count > 1024 {
            return Err(unsupported("bootstrap schema object count"));
        }
        let mut fields = [""; 4];
        for (column, maximum) in [8, 256, 256, 65536].into_iter().enumerate() {
            let ValueRef::Text(bytes) = row.get_ref(column)? else {
                return Err(unsupported("bootstrap schema field type"));
            };
            if bytes.len() > maximum {
                return Err(unsupported("bootstrap schema field bounds"));
            }
            fields[column] = std::str::from_utf8(bytes)
                .map_err(|_| unsupported("bootstrap schema field UTF-8"))?;
        }
        if !matches!(fields[0], "table" | "index" | "trigger" | "view") || fields[1].is_empty() {
            return Err(unsupported("bootstrap schema kind or name"));
        }
        total = total
            .checked_add(fields[3].len())
            .ok_or(ScanError::Allocation)?;
        if total > 2 * 1024 * 1024 {
            return Err(unsupported("bootstrap aggregate DDL bounds"));
        }
        verifier.observe(SchemaObject {
            kind: fields[0],
            name: fields[1],
            table: fields[2],
            sql: fields[3],
        })?;
    }
    verifier.finish()?;
    control.check()
}

fn dependency(
    connection: &Connection,
    control: &Control,
    qualification: ImportQualification,
) -> Result<()> {
    if qualification.target_incarnation == [0; 32] || qualification.target_revision != 1 {
        return Err(unsupported("bootstrap fresh dependency"));
    }
    control.statement()?;
    let mut statement = connection.prepare(Statement::ReadDependency.spec().sql)?;
    let mut rows = statement.query([])?;
    let row = rows
        .next()?
        .ok_or(unsupported("bootstrap dependency absent"))?;
    if row.get_ref(0)? != ValueRef::Text(b"blob")
        || row.get_ref(1)? != ValueRef::Text(b"32")
        || row.get_ref(2)? != ValueRef::Blob(&qualification.target_incarnation)
        || row.get_ref(3)? != ValueRef::Text(b"integer")
        || row.get_ref(4)? != ValueRef::Text(b"1")
        || rows.next()?.is_some()
    {
        return Err(unsupported("bootstrap dependency binding"));
    }
    control.check()
}

struct ReadOnlyKeys<'a> {
    keys: &'a DirectoryKeyProvider,
    control: &'a Control,
}

impl ReadOnlyKeys<'_> {
    fn checked<T>(&self, read: impl FnOnce() -> crate::Result<T>) -> crate::Result<T> {
        let check = || {
            self.control
                .check()
                .map_err(|_| crate::Error::Config("restricted TLS construction ended"))
        };
        check()?;
        let result = read()?;
        check()?;
        Ok(result)
    }
}

impl HostKeyProvider for ReadOnlyKeys<'_> {
    fn is_pristine(&self) -> crate::Result<bool> {
        self.checked(|| self.keys.is_pristine())
    }
    fn load_or_create(&self, purpose: KeyPurpose) -> crate::Result<SecretKey> {
        self.load_existing(purpose)
    }
    fn load_existing(&self, purpose: KeyPurpose) -> crate::Result<SecretKey> {
        self.checked(|| self.keys.load_existing(purpose))
    }
    fn create_generation(&self, _purpose: KeyPurpose) -> crate::Result<SecretKey> {
        Err(crate::Error::Key("restricted TLS keys are read-only"))
    }
    fn load_generation(
        &self,
        purpose: KeyPurpose,
        generation: KeyGenerationId,
    ) -> crate::Result<SecretKey> {
        self.checked(|| self.keys.load_generation(purpose, generation))
    }
    fn list_generations(&self, purpose: KeyPurpose) -> crate::Result<Vec<KeyGenerationId>> {
        self.checked(|| self.keys.list_generations(purpose))
    }
    fn remove_generation(
        &self,
        _purpose: KeyPurpose,
        _generation: KeyGenerationId,
    ) -> crate::Result<()> {
        Err(crate::Error::Key("restricted TLS keys are read-only"))
    }
}

pub(super) fn consume(
    import: NativePersonalListImport,
    control: &ScanControl,
    pools: &Arc<Pools>,
) -> Result<(NativeOwner, crate::pki::HostTls)> {
    import.verify_owned()?;
    let job = import.database.control();
    job.check()?;
    job.intersect(control)?;
    job.check()?;
    // The immutable native connection profile is qualified for this engine,
    // not whichever SQLite happens to satisfy a link-time library lookup.
    if rusqlite::version_number() != 3_053_002 || rusqlite::version() != "3.53.2" {
        return Err(unsupported("bootstrap native SQLite engine"));
    }
    let memory = pools.admitted_request(None).map_err(core)?;
    let mut certificates = CertificateWorkspace::new(Arc::clone(&memory), MAX_DER).map_err(core)?;
    certificates::audit(&import.database, &mut certificates)?;
    drop(certificates);
    job.check()?;
    let profile = Profile::new(&memory).map_err(core)?;
    let expected = state(&import, &profile, &job, &memory)?;
    let connection = import.database.connection();
    if !connection.is_autocommit() {
        return Err(unsupported("bootstrap unexpected active transaction"));
    }
    // This is trusted private construction only. Serving uses readbacks and
    // never repairs a changed connection policy.
    for sql in [
        "PRAGMA foreign_keys=ON",
        "PRAGMA ignore_check_constraints=OFF",
        "PRAGMA recursive_triggers=ON",
        "PRAGMA trusted_schema=OFF",
        "PRAGMA main.journal_mode=WAL",
        "PRAGMA main.synchronous=FULL",
    ] {
        job.statement()?;
        connection.execute_batch(sql)?;
    }
    job.statement()?;
    connection.execute_batch("BEGIN IMMEDIATE")?;
    job.statement()?;
    connection.execute_batch(foks_metadata_core::authority::SCHEMA)?;
    job.statement()?;
    if connection.execute(
        Statement::AuthorityInitialize.spec().sql,
        rusqlite::params![
            expected.identity_json,
            expected.activation_json,
            expected.owner.as_slice(),
            1i64,
            4608i64,
            0i64,
        ],
    )? != 1
    {
        return Err(unsupported("bootstrap authority insertion"));
    }
    #[cfg(test)]
    tests::stage(tests::Stage::Initialized, connection, &job)?;
    stored_state(connection, &job, &expected)?;
    empty_tables(connection, &job)?;
    settings(connection, &job)?;
    let before = cookie(connection, &job)?;
    union(connection, &job, &profile, &memory)?;
    if cookie(connection, &job)? != before {
        return Err(unsupported("bootstrap schema changed"));
    }
    dependency(connection, &job, import.qualification)?;
    import.verify_owned()?;
    #[cfg(test)]
    tests::stage(tests::Stage::BeforeCommit, connection, &job)?;
    job.statement()?;
    connection.execute_batch("COMMIT")?;
    if !connection.is_autocommit() {
        return Err(unsupported("bootstrap commit incomplete"));
    }
    #[cfg(test)]
    tests::stage(tests::Stage::Committed, connection, &job)?;
    job.check()?;
    // TLS/SQLite library allocations remain outside the payload accounting
    // scope of net::memory. The newly owned JSON/profile/parser/verifier memory
    // above is charged before allocation, and retained until owner destruction.
    let tls = crate::pki::build_host_tls(
        &ReadOnlyKeys {
            keys: &import.keys,
            control: &job,
        },
        &import.binding.canonical_name,
    );
    job.check()?;
    let tls = tls.map_err(super::super::server)?;
    import.verify_owned()?;
    let NativePersonalListImport {
        database,
        config,
        identity,
        keys,
        writer,
        storage,
        directory,
        qualification,
        binding,
    } = import;
    let (connection, original_job) = database.into_owned_connection(control)?;
    let owner = NativeOwner {
        connection,
        profile,
        expected,
        _config: config,
        identity,
        _keys: keys,
        _writer: writer,
        _storage: storage,
        _directory: directory,
        qualification,
        canonical_name: binding.canonical_name,
        _import_control: original_job,
        _construction: memory,
    };
    // No statement survives this transition. This hook is supplemental; only
    // the private admitted operation path may replace it and use the connection.
    owner.connection.progress_handler(1, Some(|| true))?;
    job.check()?;
    Ok((owner, tls))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::restricted_storage::{CaptureLimits, RestrictedScanner};
    use std::{
        cell::RefCell,
        path::PathBuf,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };

    #[derive(Clone, Copy, Eq, PartialEq)]
    pub(super) enum Stage {
        Initialized,
        BeforeCommit,
        Committed,
    }
    type Action = Box<dyn FnOnce(&Connection, &Arc<Control>) -> Result<()>>;
    thread_local! {
        static FAULT: RefCell<Option<(Stage, Action)>> = const { RefCell::new(None) };
    }
    pub(super) fn stage(which: Stage, db: &Connection, job: &Arc<Control>) -> Result<()> {
        let action = FAULT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_some_and(|(wanted, _)| *wanted == which) {
                slot.take().map(|(_, action)| action)
            } else {
                None
            }
        });
        match action {
            Some(action) => action(db, job),
            None => Ok(()),
        }
    }
    fn hook(which: Stage, action: impl FnOnce(&Connection, &Arc<Control>) -> Result<()> + 'static) {
        FAULT.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some((which, Box::new(action)));
        });
    }
    fn imported() -> (NativePersonalListImport, ScanControl) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let database = root.join("source.sqlite");
        let keys = root.join("keys");
        let provider = DirectoryKeyProvider::open(&keys, [0x35; 32]).unwrap();
        let mut db = foks_server_db::Database::open(&database, Default::default()).unwrap();
        crate::host::load_or_bootstrap(
            &mut db,
            &provider,
            &crate::host::BootstrapInput {
                canonical_name: "localhost".into(),
                endpoints: crate::host::BootstrapEndpoints {
                    probe: "localhost:4430".into(),
                    public_services: "localhost:4431".into(),
                    authenticated: "localhost:4432".into(),
                },
                ttl_seconds: 60,
                now_microseconds: 1000,
            },
        )
        .unwrap();
        drop(db);
        drop(provider);
        let artifacts = crate::backup_standalone_installation(
            &database,
            &keys,
            [0x35; 32],
            root.join("backup"),
            Default::default(),
        )
        .unwrap();
        let control = ScanControl::new(
            foks_local_deadline::Deadline::capture(Instant::now() + Duration::from_secs(300))
                .unwrap(),
            Arc::new(AtomicBool::new(false)),
        );
        let mut scanner = RestrictedScanner::capture_finalized_backup(
            &artifacts,
            CaptureLimits::default(),
            &control,
        )
        .unwrap();
        while !scanner.advance(&control).unwrap().complete {}
        let import = super::super::super::import_personal_list(
            scanner.finish(&control).unwrap(),
            zeroize::Zeroizing::new([0x35; 32]),
            Default::default(),
            &control,
        )
        .unwrap();
        (import, control)
    }
    fn pools() -> Arc<Pools> {
        crate::net::memory::pools(Default::default()).unwrap()
    }
    fn failed_path(import: &NativePersonalListImport) -> PathBuf {
        import.identity.path().to_path_buf()
    }

    #[test]
    fn genuine_bootstrap_binds_profile_keys_config_and_retains_private_leases() {
        assert_eq!(rusqlite::version_number(), 3_053_002);
        assert_eq!(rusqlite::version(), "3.53.2");
        let (import, control) = imported();
        let path = failed_path(&import);
        let host = import.binding.host;
        let digest = import.binding.key_bundle_digest;
        let qualification = import.qualification;
        let (owner, tls) = consume(import, &control, &pools()).unwrap();
        assert_eq!(owner.qualification(), qualification);
        assert_eq!(owner.canonical_name(), "localhost");
        owner.recheck_path().unwrap();
        assert!(!tls.client_ca.is_empty());
        assert!(!tls.delegated_ca.is_empty());
        let identity: Identity = serde_json::from_str(&owner.expected.identity_json).unwrap();
        let activation: Activation = serde_json::from_str(&owner.expected.activation_json).unwrap();
        assert_eq!(identity.host, host);
        assert_eq!(activation.key_digest, digest);
        assert_eq!(activation.kernel_hash, owner.profile.digest);
        assert_eq!(activation.schema_hash, *owner.profile.contract.hash());
        assert_eq!(activation.catalog_hash, owner.profile.catalog_digest);
        assert_eq!(activation.key_generation, Number(1));
        assert_eq!(activation.limits, limits(&Default::default()).unwrap());
        let mut cell = Sha256::new();
        cell.update(b"foks-native-personal-list-cell-v1\0");
        cell.update(qualification.target_incarnation);
        cell.update(identity.tenant);
        assert_eq!(identity.cell, <[u8; 32]>::from(cell.finalize()));
        let mut placement = Sha256::new();
        placement.update(b"foks-native-personal-list-inline-placement-v1\0");
        placement.update(qualification.target_incarnation);
        placement.update(activation.storage_namespace);
        placement.update(qualification.source_digest);
        assert_eq!(
            activation.storage_placement,
            <[u8; 32]>::from(placement.finalize())
        );
        assert!(crate::DatabaseWriterGuard::acquire(&path).is_err());
        assert!(crate::storage_lease::StorageLease::with_store(&path, None, true).is_err());
        // Independent SQLite visibility verifies an actual commit. The idle
        // owner's raw SQL path is exercised only here, as a supplemental check.
        let reader = Connection::open(&path).unwrap();
        let job = Control::new(&control).unwrap();
        stored_state(&reader, &job, &owner.expected).unwrap();
        dependency(&reader, &job, qualification).unwrap();
        assert!(owner
            .connection
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .is_err());
        drop(reader);
        drop(owner);
        assert!(!path.exists());
    }

    #[test]
    fn construction_does_not_refresh_cancelled_or_different_import_control() {
        for different in [false, true] {
            let (import, control) = imported();
            let path = failed_path(&import);
            let mut passed = control.clone();
            if different {
                passed.cancelled = Arc::new(AtomicBool::new(false));
            } else {
                control.cancelled.store(true, Ordering::Release);
            }
            assert!(consume(import, &passed, &pools()).is_err());
            assert!(!path.exists());
        }
    }

    #[test]
    fn bootstrap_refuses_changed_state_schema_settings_and_dependency() {
        for sql in [
            "UPDATE metadata_authority SET clock_floor=1",
            "UPDATE metadata_authority SET identity='{}'",
            "INSERT INTO metadata_principals VALUES(zeroblob(32),zeroblob(33),X'',1)",
            "CREATE TABLE uncompiled_extra(x)",
            "PRAGMA ignore_check_constraints=ON",
            "UPDATE metadata_read_dependency SET revision=revision+1",
        ] {
            let (import, control) = imported();
            let path = failed_path(&import);
            hook(Stage::Initialized, move |db, job| {
                job.statement()?;
                db.execute_batch(sql)?;
                Ok(())
            });
            assert!(consume(import, &control, &pools()).is_err(), "{sql}");
            assert!(!path.exists());
        }
    }

    #[test]
    fn cancellation_before_or_after_commit_and_panic_destroy_private_owner() {
        for which in [Stage::BeforeCommit, Stage::Committed] {
            let (import, control) = imported();
            let path = failed_path(&import);
            let cancelled = Arc::clone(&control.cancelled);
            hook(which, move |_, _| {
                cancelled.store(true, Ordering::Release);
                Ok(())
            });
            assert!(consume(import, &control, &pools()).is_err());
            assert!(!path.exists());
        }
        let (import, control) = imported();
        let path = failed_path(&import);
        hook(Stage::BeforeCommit, |_, _| {
            panic!("deterministic bootstrap panic")
        });
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            consume(import, &control, &pools())
        }))
        .is_err());
        assert!(!path.exists());
    }

    #[test]
    fn failed_sqlite_commit_never_returns_a_ready_owner() {
        let (import, control) = imported();
        let path = failed_path(&import);
        hook(Stage::BeforeCommit, |db, job| {
            job.statement()?;
            db.execute_batch("PRAGMA defer_foreign_keys=ON; INSERT INTO user_chain_heads(uid,seqno,link_hash) VALUES(zeroblob(33),1,zeroblob(32))")?;
            Ok(())
        });
        assert!(matches!(
            consume(import, &control, &pools()),
            Err(ScanError::Sql {
                extended_code: Some(787)
            })
        ));
        assert!(!path.exists());
    }

    #[test]
    fn bounded_json_and_config_admission_fail_closed() {
        let memory = pools().admitted_request(None).unwrap();
        assert!(json(&"x".repeat(MAX_JSON), &memory).is_err());
        assert_eq!(json(&[1u8, 2, 3], &memory).unwrap(), "[1,2,3]");
        assert!(!foks_snowpack::AllocationBudget::reserve(
            memory.as_ref(),
            usize::MAX
        ));
        assert!(matches!(json(&[1u8], &memory), Err(ScanError::Allocation)));
        let mut config = foks_server_db::Config {
            maximum_kv_node_bytes: usize::MAX,
            maximum_kv_dirent_bytes: usize::MAX,
            ..Default::default()
        };
        let result = limits(&config).unwrap();
        assert_eq!((result.blob_bytes, result.dirent_bytes), (65_536, 4096));
        config.maximum_kv_namespace_objects = 0;
        assert!(limits(&config).is_err());
    }

    #[test]
    fn tls_key_adapter_never_creates_or_removes_purpose_keys() {
        let (import, control) = imported();
        let job = import.database.control();
        let keys = ReadOnlyKeys {
            keys: &import.keys,
            control: &job,
        };
        let before = keys.load_existing(KeyPurpose::DelegatedTls).unwrap();
        let after = keys.load_or_create(KeyPurpose::DelegatedTls).unwrap();
        assert_eq!(before.generation(), after.generation());
        assert_eq!(before.expose(), after.expose());
        assert!(keys.create_generation(KeyPurpose::DelegatedTls).is_err());
        assert!(keys
            .remove_generation(KeyPurpose::DelegatedTls, before.generation())
            .is_err());
        control.cancelled.store(true, Ordering::Release);
        assert!(keys.load_or_create(KeyPurpose::DelegatedTls).is_err());

        let directory = tempfile::tempdir().unwrap();
        let pristine =
            DirectoryKeyProvider::open(directory.path().join("empty"), [0x42; 32]).unwrap();
        let fresh = ScanControl::new(
            foks_local_deadline::Deadline::capture(Instant::now() + Duration::from_secs(30))
                .unwrap(),
            Arc::new(AtomicBool::new(false)),
        );
        let job = Control::new(&fresh).unwrap();
        let keys = ReadOnlyKeys {
            keys: &pristine,
            control: &job,
        };
        assert!(keys.is_pristine().unwrap());
        assert!(keys.load_or_create(KeyPurpose::DelegatedTls).is_err());
        assert!(keys.is_pristine().unwrap());
    }
}
