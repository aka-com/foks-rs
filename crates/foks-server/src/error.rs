use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("server I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("RPC failed: {0}")]
    Rpc(#[from] foks_rpc::Error),
    #[error("server thread panicked")]
    Thread,
    #[error("invalid server configuration: {0}")]
    Config(&'static str),
    #[error("key storage failed: {0}")]
    Key(&'static str),
    #[error("key encryption or authentication failed")]
    KeyCrypto,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
