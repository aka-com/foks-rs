use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("encrypted chunk storage failed: {0}")]
    Chunk(&'static str),
    #[error("SSO flow failed: {0}")]
    Sso(&'static str),
    #[error(transparent)]
    Oidc(#[from] foks_oidc::Error),
    #[error("server I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("RPC failed: {0}")]
    Rpc(#[from] foks_rpc::Error),
    #[error("protocol construction failed: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("canonical encoding failed: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("cryptographic construction failed: {0}")]
    Crypto(#[from] foks_crypto::Error),
    #[error("constructed host state failed verification: {0}")]
    Verify(#[from] foks_verify::Error),
    #[error("SQLite server storage failed: {0}")]
    Database(#[from] foks_server_db::Error),
    #[error("Merkle construction failed: {0}")]
    Merkle(#[from] foks_merkle_store::Error),
    #[error("certificate construction failed: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("installation configuration decoding failed: {0}")]
    TomlDecode(#[from] toml::de::Error),
    #[error("installation configuration encoding failed: {0}")]
    TomlEncode(#[from] toml::ser::Error),
    #[error("server thread panicked")]
    Thread,
    #[error("SQLite writer queue is full or closed")]
    WriterQueue,
    #[error("SQLite reader pool is exhausted")]
    ReaderPool,
    #[error("request authorization changed before the queued write committed")]
    AuthorizationChanged,
    #[error("invalid server configuration: {0}")]
    Config(&'static str),
    #[error("key storage failed: {0}")]
    Key(&'static str),
    #[error("key encryption or authentication failed")]
    KeyCrypto,
    #[error("invalid software signup: {0}")]
    Signup(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<foks_hosted_transport::Error> for Error {
    fn from(error: foks_hosted_transport::Error) -> Self {
        match error {
            foks_hosted_transport::Error::Config(message) => Self::Config(message),
            foks_hosted_transport::Error::Io(error) => Self::Io(error),
        }
    }
}

/// Preserve authentication and Merkle failures across queued mutation boundaries.
pub(crate) fn mutation_failure_status(error: &Error) -> Option<foks_rpc::RpcStatus> {
    match error {
        Error::Sso(message) => Some(foks_rpc::RpcStatus::OAuth2Auth(Box::new(
            foks_rpc::RpcStatus::OAuth2((*message).into()),
        ))),
        Error::AuthorizationChanged => Some(foks_rpc::RpcStatus::PermissionDenied(
            "credential authorization changed".into(),
        )),
        Error::Merkle(foks_merkle_store::Error::EpochLimitExceeded { .. }) => {
            Some(foks_rpc::RpcStatus::MerkleVerify(error.to_string()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_transport_conversion_keeps_existing_variants_and_io_details() {
        let config: super::Error =
            foks_hosted_transport::Error::Config("request deadline overflows monotonic clock")
                .into();
        assert!(matches!(
            config,
            super::Error::Config("request deadline overflows monotonic clock")
        ));
        assert_eq!(
            config.to_string(),
            "invalid server configuration: request deadline overflows monotonic clock"
        );
        let io: super::Error = foks_hosted_transport::Error::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "original socket failure",
        ))
        .into();
        assert_eq!(io.to_string(), "server I/O failed: original socket failure");
        assert!(
            matches!(io, super::Error::Io(error) if error.kind() == std::io::ErrorKind::BrokenPipe && error.to_string() == "original socket failure")
        );
        let deadline = crate::deadline::Deadline::after(std::time::Duration::ZERO).unwrap();
        crate::deadline::scope(Some(deadline), || {
            assert!(
                matches!(crate::deadline::check(), Err(super::Error::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut)
            );
        });
        assert!(crate::deadline::check().is_ok());
    }
    #[test]
    fn epoch_limit_maps_to_explicit_verification_status() {
        let error = super::Error::Merkle(foks_merkle_store::Error::EpochLimitExceeded {
            epoch: 65_536,
            pointer_count: 16,
        });
        assert!(matches!(
            super::mutation_failure_status(&error),
            Some(foks_rpc::RpcStatus::MerkleVerify(detail))
                if detail.contains("epoch 65536") && detail.contains("back-pointer count 16")
        ));
    }
}
