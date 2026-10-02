//! Finite operational codes; never format paths, SQL, manifests, or error text.
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
        _ => Reason::Internal,
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

    use foks_server_db::Error as Db;
}
