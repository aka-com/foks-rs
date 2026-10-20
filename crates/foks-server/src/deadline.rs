//! Server error adapters around the single shared monotonic request context.
pub(crate) use foks_hosted_transport::deadline::{current, scope, Deadline};

pub(crate) fn expired() -> crate::Error {
    foks_hosted_transport::deadline::expired().into()
}
pub(crate) fn check() -> crate::Result<()> {
    foks_hosted_transport::deadline::check().map_err(Into::into)
}
