//! Finite operational codes; never format paths, SQL, manifests, or error text.
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    BackupRead,
    BackupCreate,
    BackupPublish,
    BackupRetention,
    MaintenanceAdmission,
    MaintenanceExpiry,
    MaintenanceAdmin,
    MaintenanceCheckpoint,
}

impl Phase {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::BackupRead => "backup_read",
            Self::BackupCreate => "backup_create",
            Self::BackupPublish => "backup_publish",
            Self::BackupRetention => "backup_retention",
            Self::MaintenanceAdmission => "maintenance_admission",
            Self::MaintenanceExpiry => "maintenance_expiry",
            Self::MaintenanceAdmin => "maintenance_admin",
            Self::MaintenanceCheckpoint => "maintenance_checkpoint",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Reason {
    DiskFull,
    PermissionDenied,
    Io,
    SchemaMismatch,
    CorruptStorage,
    Busy,
    Capacity,
    InvalidState,
    Storage,
    Internal,
}

impl Reason {
    fn code(self) -> &'static str {
        match self {
            Self::DiskFull => "disk_full",
            Self::PermissionDenied => "permission_denied",
            Self::Io => "io",
            Self::SchemaMismatch => "schema_mismatch",
            Self::CorruptStorage => "corrupt_storage",
            Self::Busy => "busy",
            Self::Capacity => "capacity",
            Self::InvalidState => "invalid_state",
            Self::Storage => "storage",
            Self::Internal => "internal",
        }
    }
}

fn io_reason(error: &std::io::Error) -> Reason {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => Reason::PermissionDenied,
        std::io::ErrorKind::StorageFull => Reason::DiskFull,
        _ => Reason::Io,
    }
}

fn reason(error: &Error) -> Reason {
    match error {
        Error::Io(error) => io_reason(error),
        Error::Database(error) => database_reason(error),
        Error::Merkle(foks_merkle_store::Error::Storage(_)) => Reason::Storage,
        Error::WriterQueue | Error::DeadlineExceeded | Error::ReaderPool => Reason::Busy,
        Error::Key(_) | Error::KeyCrypto | Error::Config(_) => Reason::InvalidState,
        _ => Reason::Internal,
    }
}

fn database_reason(error: &foks_server_db::Error) -> Reason {
    use foks_server_db::Error as Db;
    match error {
        Db::Io(error) => io_reason(error),
        Db::SchemaVersion { .. } | Db::ApplicationId { .. } => Reason::SchemaMismatch,
        Db::Sql(error) => match error.sqlite_error_code() {
            Some(rusqlite::ErrorCode::DiskFull) => Reason::DiskFull,
            Some(rusqlite::ErrorCode::PermissionDenied | rusqlite::ErrorCode::ReadOnly) => {
                Reason::PermissionDenied
            }
            Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
                Reason::CorruptStorage
            }
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
                Reason::Busy
            }
            _ => Reason::Storage,
        },
        Db::QuotaExceeded | Db::Capacity(_) => Reason::Capacity,
        Db::Invalid(_) | Db::UnsafeDatabasePath(_) => Reason::InvalidState,
        _ => Reason::Storage,
    }
}

pub(crate) fn report(phase: Phase, error: &Error) {
    eprintln!(
        "FOKS operation failed: phase={}, reason={}",
        phase.code(),
        reason(error).code()
    );
}

pub(crate) fn checked<T>(phase: Phase, result: crate::Result<T>) -> crate::Result<T> {
    result.inspect_err(|error| report(phase, error))
}

/// The shortest interval between two stderr lines for one site and reason.
const REQUEST_FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(60);

/// Request failures that handlers report to clients as a transaction retry,
/// keyed by handler site and reason code. The counts are process-wide because
/// the service functions that map errors have no handle on `ServerMetrics`;
/// a deployment runs one server per process.
static REQUEST_FAILURES: Mutex<BTreeMap<(&'static str, &'static str), RequestFailures>> =
    Mutex::new(BTreeMap::new());

#[derive(Default)]
struct RequestFailures {
    total: u64,
    logged_at: Option<Instant>,
    unlogged: u64,
}

/// Counts a server fault that a request handler at `site` reports to the
/// client as a transaction retry, and writes a stderr line at most once per
/// [`REQUEST_FAILURE_LOG_INTERVAL`] for each site and reason. The line carries
/// the number of failures counted since the previous line, never error text.
pub(crate) fn request_failed(site: &'static str, error: &Error) {
    record_request_failure(site, reason(error));
}

/// [`request_failed`] for a storage error that is not yet a server [`Error`].
pub(crate) fn request_storage_failed(site: &'static str, error: &foks_server_db::Error) {
    record_request_failure(site, database_reason(error));
}

/// [`request_failed`] for a violated invariant that has no error value.
pub(crate) fn request_invariant_failed(site: &'static str) {
    record_request_failure(site, Reason::Internal);
}

fn record_request_failure(site: &'static str, reason: Reason) {
    let reason = reason.code();
    let logged = {
        let mut failures = REQUEST_FAILURES
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        count_request_failure(&mut failures, site, reason, Instant::now())
    };
    if let Some(count) = logged {
        eprintln!(
            "FOKS request failed: site={site}, reason={reason}, count_since_last_line={count}"
        );
    }
}

/// Counts one failure and returns the number of failures that the next stderr
/// line reports, or `None` while the interval since the last line is running.
fn count_request_failure(
    failures: &mut BTreeMap<(&'static str, &'static str), RequestFailures>,
    site: &'static str,
    reason: &'static str,
    now: Instant,
) -> Option<u64> {
    let entry = failures.entry((site, reason)).or_default();
    entry.total = entry.total.saturating_add(1);
    entry.unlogged = entry.unlogged.saturating_add(1);
    if entry
        .logged_at
        .is_some_and(|at| now.saturating_duration_since(at) < REQUEST_FAILURE_LOG_INTERVAL)
    {
        return None;
    }
    entry.logged_at = Some(now);
    Some(std::mem::take(&mut entry.unlogged))
}

/// Returns the request failure counts as `(site, reason, count)`, ordered by
/// site and reason.
pub(crate) fn request_failures() -> Vec<(&'static str, &'static str, u64)> {
    REQUEST_FAILURES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(&(site, reason), failures)| (site, reason, failures.total))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_storage_failures_have_bounded_codes_without_error_content() {
        let cases = [
            (
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "secret/path",
                )),
                "permission_denied",
            ),
            (
                Error::Database(Db::SchemaVersion { found: 47 }),
                "schema_mismatch",
            ),
            (Error::Key("secret manifest"), "invalid_state"),
            (Error::WriterQueue, "busy"),
            (Error::Database(Db::QuotaExceeded), "capacity"),
        ];
        for (error, expected) in cases {
            assert_eq!(reason(&error).code(), expected);
        }
        for (sqlite, expected) in [
            (rusqlite::ffi::SQLITE_FULL, "disk_full"),
            (rusqlite::ffi::SQLITE_CORRUPT, "corrupt_storage"),
            (rusqlite::ffi::SQLITE_BUSY, "busy"),
        ] {
            let error = Error::Database(Db::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(sqlite),
                Some("secret SQL".into()),
            )));
            assert_eq!(reason(&error).code(), expected);
        }
    }

    #[test]
    fn request_failure_lines_are_limited_per_site_and_reason() {
        let mut failures = BTreeMap::new();
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        assert_eq!(
            count_request_failure(&mut failures, "kv_write", "busy", at(0)),
            Some(1)
        );
        assert_eq!(
            count_request_failure(&mut failures, "kv_write", "busy", at(1)),
            None
        );
        assert_eq!(
            count_request_failure(&mut failures, "kv_write", "busy", at(59)),
            None
        );
        // Another site or reason has its own interval.
        assert_eq!(
            count_request_failure(&mut failures, "kv_write", "corrupt_storage", at(1)),
            Some(1)
        );
        assert_eq!(
            count_request_failure(&mut failures, "team_admin", "busy", at(1)),
            Some(1)
        );
        // The next line reports the failures counted since the previous one.
        assert_eq!(
            count_request_failure(&mut failures, "kv_write", "busy", at(60)),
            Some(3)
        );
        assert_eq!(
            count_request_failure(&mut failures, "kv_write", "busy", at(61)),
            None
        );
        assert_eq!(failures[&("kv_write", "busy")].total, 5);
    }

    #[test]
    fn request_failures_are_counted_by_site_and_reason() {
        let site = "operational_test_site";
        let count = |reason| {
            request_failures()
                .into_iter()
                .find(|&(s, r, _)| s == site && r == reason)
                .map_or(0, |(_, _, count)| count)
        };
        request_failed(site, &Error::Database(Db::QuotaExceeded));
        request_failed(site, &Error::Database(Db::QuotaExceeded));
        request_storage_failed(
            site,
            &Db::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                Some("secret SQL".into()),
            )),
        );
        request_failed(site, &Error::Signup("secret"));
        request_failed(
            site,
            &Error::Merkle(foks_merkle_store::Error::Storage("secret".into())),
        );
        assert_eq!(count("capacity"), 2);
        assert_eq!(count("corrupt_storage"), 1);
        assert_eq!(count("internal"), 1);
        assert_eq!(count("storage"), 1);
    }

    use foks_server_db::Error as Db;
}
