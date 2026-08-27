use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("SQLite storage failed: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("database application id {found:#x} is not a FOKS server store")]
    ApplicationId { found: i64 },
    #[error("database schema version {found} is unsupported")]
    SchemaVersion { found: i64 },
    #[error("unsigned value exceeds SQLite's signed integer range")]
    IntegerRange,
    #[error("invalid storage input: {0}")]
    Invalid(&'static str),
    #[error("name is already reserved or claimed")]
    NameInUse,
    #[error("reservation is missing, expired, or does not match")]
    Reservation,
    #[error("idempotency identity was reused with different request bytes")]
    ReceiptConflict,
    #[error("idempotency receipt expired; mutation outcome requires operator reconciliation")]
    ReceiptExpired,
    #[error("expected Merkle head does not match the authoritative head")]
    StaleRoot,
    #[error("KV object conflicts with authoritative state")]
    KvConflict,
    #[error("KV lock is held by another token")]
    KvLocked,
    #[error("configured storage quota is exhausted")]
    QuotaExceeded,
    #[error("injected transaction failure at {0:?}")]
    Injected(crate::FailurePoint),
    #[error("injected user-mutation failure at {0:?}")]
    UserMutationInjected(crate::UserMutationFailurePoint),
    #[error("injected team-mutation failure at {0:?}")]
    TeamMutationInjected(crate::TeamMutationFailurePoint),
    #[error("Merkle storage failed: {0}")]
    Merkle(#[from] foks_merkle_store::Error),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn is_quota(&self) -> bool {
        matches!(self, Self::QuotaExceeded)
            || matches!(
                self,
                Self::Sql(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == rusqlite::ErrorCode::DiskFull
            )
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn sql_integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::IntegerRange)
}

pub(crate) fn unsigned(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::Invalid("negative stored integer"))
}
