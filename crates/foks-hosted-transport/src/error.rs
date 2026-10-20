use thiserror::Error;

/// Preserve the ordinary server's error kinds, display text and source chain.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid server configuration: {0}")]
    Config(&'static str),
    #[error("server I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
