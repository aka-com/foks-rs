//! accept() failure policy shared by the server's TCP listeners.

use std::time::Duration;

use crate::ServerMetrics;

/// Pause after a transient accept() failure that does not belong to a single
/// connection. A listening socket stays readable while a connection waits in
/// its backlog, so an immediate retry would repeat the same EMFILE until a
/// descriptor is released.
pub(crate) const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(50);

/// accept() failures after which the listening socket remains usable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum AcceptErrorKind {
    /// EMFILE: the process descriptor limit (RLIMIT_NOFILE) is reached.
    ProcessDescriptors,
    /// ENFILE: the system-wide descriptor limit is reached.
    SystemDescriptors,
    /// ENOBUFS: socket buffer memory is exhausted.
    Buffers,
    /// ENOMEM: kernel memory is exhausted.
    Memory,
    /// ECONNABORTED: a queued connection was reset before accept() took it.
    ConnectionAborted,
    /// Any other error code that does not invalidate the listening socket,
    /// including the pending network errors that Linux accept() reports for
    /// the new connection.
    Other,
}

impl AcceptErrorKind {
    pub const ALL: [Self; 6] = [
        Self::ProcessDescriptors,
        Self::SystemDescriptors,
        Self::Buffers,
        Self::Memory,
        Self::ConnectionAborted,
        Self::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ProcessDescriptors => "emfile",
            Self::SystemDescriptors => "enfile",
            Self::Buffers => "enobufs",
            Self::Memory => "enomem",
            Self::ConnectionAborted => "econnaborted",
            Self::Other => "other",
        }
    }

    /// ECONNABORTED concerns only the aborted connection, so the next queued
    /// connection is accepted at once. Every other kind pauses, which also
    /// bounds the retry rate for an unclassified error that persists.
    fn retry_delay(self) -> Duration {
        match self {
            Self::ConnectionAborted => Duration::ZERO,
            _ => ACCEPT_RETRY_DELAY,
        }
    }
}

/// Handles a failed accept(). Returns the error when it invalidates the
/// listening socket; otherwise counts it and waits until the listener should
/// accept again.
pub(crate) async fn retry_accept(
    error: std::io::Error,
    metrics: &ServerMetrics,
) -> std::io::Result<()> {
    let Some(kind) = transient_accept_error(&error) else {
        return Err(error);
    };
    metrics.accept_failed(kind);
    let delay = kind.retry_delay();
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    Ok(())
}

/// Classifies an accept() failure. `None` means the listening socket itself
/// is unusable (closed, not a socket, or not listening), or the error did not
/// come from the operating system, and the accept loop must end.
fn transient_accept_error(error: &std::io::Error) -> Option<AcceptErrorKind> {
    let code = error.raw_os_error()?;
    #[cfg(unix)]
    {
        Some(match code {
            libc::EBADF | libc::EINVAL | libc::ENOTSOCK | libc::EFAULT => return None,
            libc::EMFILE => AcceptErrorKind::ProcessDescriptors,
            libc::ENFILE => AcceptErrorKind::SystemDescriptors,
            libc::ENOBUFS => AcceptErrorKind::Buffers,
            libc::ENOMEM => AcceptErrorKind::Memory,
            libc::ECONNABORTED => AcceptErrorKind::ConnectionAborted,
            _ => AcceptErrorKind::Other,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = code;
        match error.kind() {
            std::io::ErrorKind::ConnectionAborted => Some(AcceptErrorKind::ConnectionAborted),
            std::io::ErrorKind::OutOfMemory => Some(AcceptErrorKind::Memory),
            _ => None,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn listener_invalidation_is_fatal_and_resource_exhaustion_is_retried() {
        for (code, kind) in [
            (libc::EMFILE, AcceptErrorKind::ProcessDescriptors),
            (libc::ENFILE, AcceptErrorKind::SystemDescriptors),
            (libc::ENOBUFS, AcceptErrorKind::Buffers),
            (libc::ENOMEM, AcceptErrorKind::Memory),
            (libc::ECONNABORTED, AcceptErrorKind::ConnectionAborted),
            (libc::EPROTO, AcceptErrorKind::Other),
            (libc::ECONNRESET, AcceptErrorKind::Other),
            (libc::EPERM, AcceptErrorKind::Other),
        ] {
            assert_eq!(
                transient_accept_error(&std::io::Error::from_raw_os_error(code)),
                Some(kind)
            );
        }
        for code in [libc::EBADF, libc::EINVAL, libc::ENOTSOCK, libc::EFAULT] {
            assert_eq!(
                transient_accept_error(&std::io::Error::from_raw_os_error(code)),
                None
            );
        }
        assert_eq!(
            transient_accept_error(&std::io::Error::other("not an OS error")),
            None
        );
    }
}
