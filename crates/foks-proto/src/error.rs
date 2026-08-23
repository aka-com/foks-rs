//! Protocol schema errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid canonical Snowpack: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("expected {expected}, found {found}")]
    Type {
        expected: &'static str,
        found: &'static str,
    },
    #[error("expected {expected} fields, found {found}")]
    FieldCount { expected: usize, found: usize },
    #[error("expected variant tag {expected:?}, found {found:?}")]
    VariantTag { expected: String, found: Vec<u8> },
    #[error("unknown {kind} value {value}")]
    UnknownEnum { kind: &'static str, value: u64 },
    #[error("{kind} has length {found}, expected {expected}")]
    Length {
        kind: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("invalid entity type {0}")]
    EntityType(u8),
    #[error("expected entity type {expected}, found {found}")]
    WrongEntityType { expected: u8, found: u8 },
    #[error("integer does not fit {0}")]
    IntegerRange(&'static str),
    #[error("text is not UTF-8")]
    Utf8,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
