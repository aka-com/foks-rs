//! Unix-only storage-fit scanning of owned finalized native backups. A successful receipt
//! is not authentication, physical integrity, import readiness, or serving authority.
mod capture;
#[cfg(any(test, feature = "celld-proof"))]
mod import;
mod scan;
#[cfg(feature = "celld-proof")]
pub use import::list::{
    start_native_personal_list_qualification, CertificateParserQualification,
    CertificateQualification, NativeListQualification, NativeListQualificationReport,
};
#[cfg(feature = "celld-proof")]
pub use import::{qualify_personal_list_import, ImportQualification};
#[cfg(test)]
mod tests;

use foks_local_deadline::Deadline;
use foks_schema_contract::storage::{TableId, Violation, HOSTED_SMALL_V1};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

const MAX_DATABASE: u64 = 64 * 1024 * 1024 * 1024;
const MAX_KEYS: usize = 4096;
const STEP_ROWS: usize = 128;
/// Opcode budget of one ordinary statement: the scanner's and the import's
/// keyset `LIMIT 1` probes and other bounded reads.
const STATEMENT_OPERATIONS: usize = 1_000_000;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("restricted storage: {0}")]
    Unsupported(&'static str),
    #[error("restricted storage scan cancelled")]
    Cancelled,
    #[error("restricted storage scanner failed or is incomplete")]
    Incomplete,
    #[error("restricted storage allocation failed")]
    Allocation,
    #[error("restricted storage row in {table:?}: {violation}")]
    Row {
        table: TableId,
        violation: Violation,
    },
    #[error("{0}")]
    Deadline(#[from] foks_local_deadline::Error),
    #[error("restricted storage I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("restricted storage SQLite failure (extended code {extended_code:?})")]
    Sql { extended_code: Option<i32> },
    #[error("restricted storage schema: {0}")]
    Schema(#[from] foks_schema_contract::ContractError),
}
impl From<rusqlite::Error> for ScanError {
    fn from(error: rusqlite::Error) -> Self {
        // SQLite messages may contain source schema/payload. Retain a numeric
        // class only; bounded diagnostics must not echo those bytes.
        Self::Sql {
            extended_code: error.sqlite_error().map(|v| v.extended_code),
        }
    }
}
type Result<T> = std::result::Result<T, ScanError>;

/// A finite maintenance/import job, captured before capture or hashing begins.
#[derive(Clone)]
pub struct ScanControl {
    deadline: Deadline,
    cancelled: Arc<AtomicBool>,
}
impl ScanControl {
    pub fn new(deadline: Deadline, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            deadline,
            cancelled,
        }
    }
    fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ScanError::Cancelled);
        }
        self.deadline.check()?;
        Ok(())
    }
}
/// May lower, never enlarge, the first profile's native database file ceiling.
#[derive(Clone, Copy)]
pub struct CaptureLimits {
    pub maximum_database_bytes: u64,
}
impl Default for CaptureLimits {
    fn default() -> Self {
        Self {
            maximum_database_bytes: MAX_DATABASE,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanProgress {
    pub tables_complete: u16,
    pub rows_checked: u64,
    pub complete: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageSummary {
    pub tables: u16,
    pub rows: u64,
    pub maximum_table_bytes: u64,
    pub maximum_index_bytes: u64,
    pub external_chunk_references: u64,
    pub admin_audit_sequence: Option<i64>,
}
/// Owns the exact private source through the future import. No public constructor,
/// raw connection/path accessor, Clone, or deserialization can mint this receipt.
pub struct StorageScanComplete {
    _database: scan::Database,
    _package: capture::Package,
    digest: [u8; 32],
    summary: StorageSummary,
}
impl StorageScanComplete {
    pub fn source_digest(&self) -> &[u8; 32] {
        &self.digest
    }
    pub fn database_header_modes(&self) -> [u8; 2] {
        self._package.header_modes()
    }
    pub fn profile(&self) -> &'static str {
        HOSTED_SMALL_V1.name()
    }
    pub fn summary(&self) -> StorageSummary {
        self.summary
    }
}

/// Why the progress hook stopped a statement. The first cause is kept, so a
/// later cancellation or deadline does not relabel a budget stop.
#[derive(Clone)]
enum Stop {
    Budget,
    Cancelled,
    Deadline(foks_local_deadline::Error),
    Failed,
}

struct Control {
    control: Mutex<ScanControl>,
    operations: AtomicUsize,
    budget: AtomicUsize,
    interrupted: AtomicBool,
    stop: Mutex<Option<Stop>>,
}
impl Control {
    fn new(value: &ScanControl) -> Result<Arc<Self>> {
        value.check()?;
        Ok(Arc::new(Self {
            control: Mutex::new(value.clone()),
            operations: AtomicUsize::new(0),
            budget: AtomicUsize::new(STATEMENT_OPERATIONS),
            interrupted: AtomicBool::new(false),
            stop: Mutex::new(None),
        }))
    }
    fn check(&self) -> Result<()> {
        if self.interrupted.load(Ordering::Acquire) {
            return Err(self.stopped());
        }
        self.job_check()
    }
    fn job_check(&self) -> Result<()> {
        self.control
            .lock()
            .map_err(|_| ScanError::Incomplete)?
            .check()
    }
    /// The error for the cause recorded when the hook stopped a statement.
    fn stopped(&self) -> ScanError {
        match self.stop.lock().ok().and_then(|stop| stop.clone()) {
            Some(Stop::Cancelled) => ScanError::Cancelled,
            Some(Stop::Deadline(error)) => ScanError::Deadline(error),
            Some(Stop::Failed) => ScanError::Incomplete,
            Some(Stop::Budget) | None => ScanError::Unsupported("SQLite operation/deadline limit"),
        }
    }
    /// SQLite reports a statement stopped by the progress hook as
    /// SQLITE_INTERRUPT. Replace that with the cause the hook recorded:
    /// Cancelled, Deadline, or the operation limit.
    fn classify(&self, error: ScanError) -> ScanError {
        match error {
            ScanError::Sql {
                extended_code: Some(rusqlite::ffi::SQLITE_INTERRUPT),
            } if self.interrupted.load(Ordering::Acquire) => self.stopped(),
            error => error,
        }
    }
    fn intersect(&self, value: &ScanControl) -> Result<()> {
        value.check()?;
        let mut original = self.control.lock().map_err(|_| ScanError::Incomplete)?;
        if !Arc::ptr_eq(&original.cancelled, &value.cancelled) {
            return Err(ScanError::Unsupported("different cancellation owner"));
        }
        original.deadline = original.deadline.intersect(&value.deadline)?;
        original.check()
    }
    fn statement(&self) -> Result<()> {
        self.statement_with_budget(STATEMENT_OPERATIONS)
    }
    /// Starts one statement with its own opcode budget. Cancellation and the
    /// deadline are still checked by the progress hook whatever the budget.
    fn statement_with_budget(&self, operations: usize) -> Result<()> {
        self.check()?;
        self.budget.store(operations, Ordering::Release);
        self.operations.store(0, Ordering::Release);
        Ok(())
    }
    fn progress(&self) -> bool {
        if self.interrupted.load(Ordering::Acquire) {
            return true;
        }
        let used = self
            .operations
            .fetch_add(1000, Ordering::AcqRel)
            .saturating_add(1000);
        let cause = if used >= self.budget.load(Ordering::Acquire) {
            Some(Stop::Budget)
        } else {
            match self.job_check() {
                Ok(()) => None,
                Err(ScanError::Cancelled) => Some(Stop::Cancelled),
                Err(ScanError::Deadline(error)) => Some(Stop::Deadline(error)),
                Err(_) => Some(Stop::Failed),
            }
        };
        let Some(cause) = cause else {
            return false;
        };
        if let Ok(mut stop) = self.stop.lock() {
            stop.get_or_insert(cause);
        }
        self.interrupted.store(true, Ordering::Release);
        true
    }
}

pub struct RestrictedScanner {
    // Database drops before package and its private path.
    database: scan::Database,
    package: capture::Package,
    control: Arc<Control>,
    cursor: scan::Cursor,
    summary: StorageSummary,
    failed: bool,
    complete: bool,
}
impl RestrictedScanner {
    pub fn capture_finalized_backup(
        artifacts: &crate::BackupArtifacts,
        limits: CaptureLimits,
        control: &ScanControl,
    ) -> Result<Self> {
        if limits.maximum_database_bytes == 0 || limits.maximum_database_bytes > MAX_DATABASE {
            return Err(ScanError::Unsupported("database capture ceiling"));
        }
        let control = Control::new(control)?;
        let package = capture::Package::capture(artifacts, limits, &control)
            .map_err(|error| control.classify(error))?;
        let database = scan::Database::open(package.database(), control.clone())
            .map_err(|error| control.classify(error))?;
        database
            .verify_schema()
            .map_err(|error| control.classify(error))?;
        Ok(Self {
            database,
            package,
            control,
            cursor: scan::Cursor::default(),
            summary: StorageSummary {
                tables: 0,
                rows: 0,
                maximum_table_bytes: 0,
                maximum_index_bytes: 0,
                external_chunk_references: 0,
                admin_audit_sequence: None,
            },
            failed: false,
            complete: false,
        })
    }
    pub fn advance(&mut self, control: &ScanControl) -> Result<ScanProgress> {
        if self.failed {
            return Err(ScanError::Incomplete);
        }
        let result = (|| {
            self.control.intersect(control)?;
            if !self.complete {
                self.complete =
                    self.database
                        .advance(&mut self.cursor, &mut self.summary, STEP_ROWS)?;
                if self.complete {
                    self.summary.admin_audit_sequence = self.database.sequence(&self.cursor)?;
                    self.database
                        .verify_companions(&self.package, &self.cursor)?;
                }
            }
            self.control.check()?;
            Ok(ScanProgress {
                tables_complete: self.summary.tables,
                rows_checked: self.summary.rows,
                complete: self.complete,
            })
        })();
        if result.is_err() {
            self.failed = true;
        }
        result.map_err(|error| self.control.classify(error))
    }
    pub fn finish(mut self, control: &ScanControl) -> Result<StorageScanComplete> {
        if self.failed || !self.complete {
            return Err(ScanError::Incomplete);
        }
        self.control.intersect(control)?;
        self.package
            .verify(&self.control)
            .map_err(|error| self.control.classify(error))?;
        self.control.check()?;
        let digest = self.package.digest();
        Ok(StorageScanComplete {
            _database: self.database,
            _package: self.package,
            digest,
            summary: self.summary,
        })
    }
}
