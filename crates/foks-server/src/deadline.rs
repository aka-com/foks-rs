//! Server error adapters around the single shared monotonic request context.
//!
//! The request deadline starts at the first byte of a request frame and covers
//! body reading, reader checkout, handler execution, queued writes, chunk
//! transfers and the response flush. An expired deadline is reported as
//! [`crate::Error::DeadlineExceeded`], which the status funnel maps to
//! `RateLimited`; framing and response writes report it as a timed-out I/O
//! error and close the connection.
pub(crate) use foks_hosted_transport::deadline::{current, scope, Deadline};

pub(crate) fn expired() -> crate::Error {
    crate::Error::DeadlineExceeded
}

pub(crate) fn check() -> crate::Result<()> {
    foks_hosted_transport::deadline::check().map_err(|_| expired())
}
