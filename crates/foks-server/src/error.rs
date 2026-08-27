use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
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
    #[error("server thread panicked")]
    Thread,
    #[error("SQLite writer queue is full or closed")]
    WriterQueue,
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
