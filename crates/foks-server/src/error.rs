use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
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
