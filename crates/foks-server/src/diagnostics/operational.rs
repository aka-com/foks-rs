//! Finite operational codes; never format paths, SQL, manifests, or error text.
use crate::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    BackupLease,
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
            Self::BackupLease => "backup_lease",
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
    use foks_server_db::Error as Db;
    match error {
        Error::Io(error) | Error::Database(Db::Io(error)) => io_reason(error),
        Error::Database(Db::SchemaVersion { .. } | Db::ApplicationId { .. }) => {
            Reason::SchemaMismatch
        }
        Error::Database(Db::Sql(error)) => match error.sqlite_error_code() {
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
        Error::Database(Db::QuotaExceeded | Db::Capacity(_)) => Reason::Capacity,
        Error::WriterQueue | Error::ReaderPool => Reason::Busy,
        Error::Key(_)
        | Error::KeyCrypto
        | Error::Config(_)
        | Error::Database(Db::Invalid(_) | Db::UnsafeDatabasePath(_)) => Reason::InvalidState,
        Error::Database(_) => Reason::Storage,
        Error::Chunk(message) => chunk_reason(message),
        _ => Reason::Internal,
    }
}

/// The storage lease and the object namespace lease refuse with these
/// messages only when the lock is held (WouldBlock); an I/O failure while
/// locking is reported as `Error::Io`.
const CHUNK_BUSY: [&str; 2] = [
    crate::storage_lease::STORAGE_IN_USE,
    crate::chunks::NAMESPACE_IN_USE,
];

fn chunk_reason(message: &str) -> Reason {
    if CHUNK_BUSY.contains(&message) {
        Reason::Busy
    } else {
        Reason::Storage
    }
}

#[cfg(test)]
thread_local! {
    static REPORTED: std::cell::RefCell<Vec<(&'static str, &'static str)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The (phase, reason) records this thread has reported since the last call.
#[cfg(test)]
pub(crate) fn take_reported() -> Vec<(&'static str, &'static str)> {
    REPORTED.with(|reported| std::mem::take(&mut *reported.borrow_mut()))
}

pub(crate) fn report(phase: Phase, error: &Error) {
    #[cfg(test)]
    REPORTED.with(|reported| {
        reported
            .borrow_mut()
            .push((phase.code(), reason(error).code()))
    });
    eprintln!(
        "FOKS operation failed: phase={}, reason={}",
        phase.code(),
        reason(error).code()
    );
}

pub(crate) fn checked<T>(phase: Phase, result: crate::Result<T>) -> crate::Result<T> {
    result.inspect_err(|error| report(phase, error))
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
            (
                Error::Chunk("storage lock is not a regular file"),
                "storage",
            ),
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
    fn held_storage_and_namespace_leases_are_busy() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("foks-server.sqlite");
        std::fs::write(&database, b"").unwrap();
        let held = crate::storage_lease::StorageLease::exclusive(&database).unwrap();
        let Err(error) = crate::storage_lease::StorageLease::shared(&database) else {
            panic!("a held storage lease must refuse");
        };
        assert_eq!(reason(&error).code(), "busy");
        drop(held);
        let store = crate::chunks::ChunkStoreConfig::Filesystem {
            directory: root.path().join("objects"),
        }
        .build()
        .unwrap();
        let held = store.namespace_lease(true).unwrap();
        let Err(error) = store.namespace_lease(false) else {
            panic!("a held namespace lease must refuse");
        };
        assert_eq!(reason(&error).code(), "busy");
        drop(held);
        // A lock attempt that fails with I/O is not reported as contention.
        let error = Error::Io(std::io::Error::from_raw_os_error(libc::ENOLCK));
        assert_eq!(reason(&error).code(), "io");
    }

    use foks_server_db::Error as Db;
}
