use crate::rpc::Listener;
use crate::Error;

/// Stable, deliberately coarse failure classes suitable for operator logs.
///
/// Diagnostics never receive the underlying error, request bytes, identity,
/// or key material. This keeps logging implementations from accidentally
/// exposing protocol content while still distinguishing operational failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionErrorClass {
    Transport,
    Tls,
    Rpc,
    Protocol,
    Storage,
    Internal,
}

pub trait SessionDiagnostics: Send + Sync {
    fn session_failed(&self, listener: Listener, connection_id: u64, class: SessionErrorClass);
}

#[derive(Debug, Default)]
pub struct StderrSessionDiagnostics;

impl SessionDiagnostics for StderrSessionDiagnostics {
    fn session_failed(&self, listener: Listener, connection_id: u64, class: SessionErrorClass) {
        eprintln!(
            "FOKS session failed: listener={listener:?}, connection_id={connection_id}, class={class:?}"
        );
    }
}

pub(crate) fn classify(error: &Error) -> SessionErrorClass {
    match error {
        Error::Io(_) => SessionErrorClass::Transport,
        Error::Tls(_) => SessionErrorClass::Tls,
        Error::Rpc(_) => SessionErrorClass::Rpc,
        Error::Protocol(_)
        | Error::Snowpack(_)
        | Error::Crypto(_)
        | Error::Verify(_)
        | Error::Signup(_)
        | Error::AuthorizationChanged => SessionErrorClass::Protocol,
        Error::Database(_) | Error::Merkle(_) | Error::WriterQueue | Error::ReaderPool => {
            SessionErrorClass::Storage
        }
        Error::Certificate(_)
        | Error::Thread
        | Error::Config(_)
        | Error::Key(_)
        | Error::KeyCrypto => SessionErrorClass::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, SessionErrorClass};

    #[test]
    fn failure_classification_never_formats_error_details() {
        let error = crate::Error::Config("secret-shaped detail");
        assert_eq!(classify(&error), SessionErrorClass::Internal);
    }
}
