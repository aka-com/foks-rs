//! RPC statuses for failures that every write route reports the same way.
//!
//! Each route family maps its own domain variants, such as a KV version race,
//! a stale team head or a full waitlist, and passes every other error through
//! [`shared`]. A full database, a saturated writer queue, a credential revoked
//! while its write was queued and an SQLite fault that the request did not
//! cause therefore carry the same status on every route. `Capacity` stays
//! with each family: some of its values are retained-storage limits and
//! others are concurrency caps that the route reports as rate limiting.
//!
//! Statuses carry fixed strings. The `Display` text of [`Error`] can include
//! SQLite messages and must not reach clients. The epoch-limit detail is the
//! one exception; it contains only the epoch and the back-pointer count.

use foks_rpc::RpcStatus;

use crate::Error;

/// Returns the status for an error class that is the same on every route, or
/// `None` when the route family maps the error itself.
pub(crate) fn shared(error: &Error) -> Option<RpcStatus> {
    match error {
        Error::Sso(message) => Some(RpcStatus::OAuth2Auth(Box::new(RpcStatus::OAuth2(
            (*message).into(),
        )))),
        Error::AuthorizationChanged => Some(RpcStatus::PermissionDenied(
            "credential authorization changed".into(),
        )),
        Error::Merkle(foks_merkle_store::Error::EpochLimitExceeded { .. }) => {
            Some(RpcStatus::MerkleVerify(error.to_string()))
        }
        Error::WriterQueue | Error::ReaderPool => Some(RpcStatus::RateLimited),
        Error::Database(error) => storage(error),
        _ => None,
    }
}

/// Returns the status for a storage failure that is the same on every route:
/// the configured quota or the `max_page_count` ceiling is exhausted, or SQLite
/// reports a busy, I/O, permission or corruption fault that is not caused by
/// the request.
///
/// The v0.1.9 status set has no non-retryable status for a server-side storage
/// fault, so a corrupt database is reported as a transaction retry like the
/// other faults.
pub(crate) fn storage(error: &foks_server_db::Error) -> Option<RpcStatus> {
    use rusqlite::ErrorCode as Code;
    if error.is_quota() {
        return Some(RpcStatus::QuotaExceeded);
    }
    let foks_server_db::Error::Sql(error) = error else {
        return None;
    };
    match error.sqlite_error_code()? {
        Code::DatabaseBusy
        | Code::DatabaseLocked
        | Code::OutOfMemory
        | Code::OperationInterrupted
        | Code::SystemIoFailure
        | Code::CannotOpen
        | Code::FileLockingProtocolFailed
        | Code::ReadOnly
        | Code::PermissionDenied
        | Code::DatabaseCorrupt
        | Code::NotADatabase => Some(RpcStatus::TransactionRetry),
        _ => None,
    }
}

/// Test helpers that every route family's mapper is checked against.
#[cfg(test)]
pub(crate) mod contract {
    use foks_rpc::RpcStatus;

    use crate::Error;

    /// An SQLite result-code failure, as the writer connection reports it.
    pub(crate) fn sqlite(code: std::ffi::c_int) -> Error {
        Error::Database(foks_server_db::Error::Sql(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            None,
        )))
    }

    /// `SQLITE_FULL`, which the `max_page_count` ceiling produces.
    pub(crate) fn database_full() -> Error {
        sqlite(rusqlite::ffi::SQLITE_FULL)
    }

    /// Asserts that `map` reports the shared failure classes with the shared
    /// statuses. `sso` is the status the mapper must return for `Error::Sso`.
    pub(crate) fn assert_shared_classes(
        route: &str,
        sso: fn(&'static str) -> RpcStatus,
        map: impl Fn(Error) -> RpcStatus,
    ) {
        let cases = [
            (database_full(), RpcStatus::QuotaExceeded),
            (
                Error::Database(foks_server_db::Error::QuotaExceeded),
                RpcStatus::QuotaExceeded,
            ),
            (Error::WriterQueue, RpcStatus::RateLimited),
            (Error::ReaderPool, RpcStatus::RateLimited),
            (
                Error::AuthorizationChanged,
                RpcStatus::PermissionDenied("credential authorization changed".into()),
            ),
            (
                Error::Sso("provider session expired"),
                sso("provider session expired"),
            ),
            (
                sqlite(rusqlite::ffi::SQLITE_BUSY),
                RpcStatus::TransactionRetry,
            ),
            (
                sqlite(rusqlite::ffi::SQLITE_CORRUPT),
                RpcStatus::TransactionRetry,
            ),
        ];
        for (error, expected) in cases {
            let label = format!("{error:?}");
            assert_eq!(map(error), expected, "{route}: {label}");
        }
    }

    /// The `Error::Sso` status on authenticated mutation routes.
    pub(crate) fn wrapped_sso(message: &'static str) -> RpcStatus {
        RpcStatus::OAuth2Auth(Box::new(RpcStatus::OAuth2(message.into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_limit_maps_to_explicit_verification_status() {
        let error = Error::Merkle(foks_merkle_store::Error::EpochLimitExceeded {
            epoch: 65_536,
            pointer_count: 16,
        });
        assert!(matches!(
            shared(&error),
            Some(RpcStatus::MerkleVerify(detail))
                if detail.contains("epoch 65536") && detail.contains("back-pointer count 16")
        ));
    }

    #[test]
    fn database_full_is_quota_and_request_faults_stay_with_the_route() {
        assert_eq!(
            shared(&contract::database_full()),
            Some(RpcStatus::QuotaExceeded)
        );
        // A constraint violation and a domain refusal are the family's to map.
        assert_eq!(
            shared(&contract::sqlite(rusqlite::ffi::SQLITE_CONSTRAINT)),
            None
        );
        assert_eq!(
            shared(&Error::Database(foks_server_db::Error::Capacity(
                "waitlist"
            ))),
            None
        );
        assert_eq!(
            shared(&Error::Signup("team member PUK is not current")),
            None
        );
    }

    #[test]
    fn shared_statuses_carry_no_storage_text() {
        let error = Error::Database(foks_server_db::Error::Sql(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("database is locked by /srv/foks/server.db".into()),
        )));
        assert_eq!(shared(&error), Some(RpcStatus::TransactionRetry));
    }
}
