//! Server error adapters around the single shared monotonic request context.
//!
//! The request deadline starts at the first byte of a request frame and covers
//! body reading, reader checkout, handler execution, queued writes and chunk
//! transfers. The response write of a completed request gets its own
//! allowance of the session I/O timeout instead. An expired deadline is
//! reported as [`crate::Error::DeadlineExceeded`], which the status funnel
//! maps to `RateLimited`; framing reports it as a timed-out I/O error and
//! closes the connection.
pub(crate) use foks_hosted_transport::deadline::{current, scope, Deadline};

pub(crate) fn expired() -> crate::Error {
    crate::Error::DeadlineExceeded
}

pub(crate) fn check() -> crate::Result<()> {
    foks_hosted_transport::deadline::check().map_err(|_| expired())
}
