//! Canonical Snowpack encoding for FOKS v0.1.9.
//!
//! Snowpack uses a deliberately small MessagePack subset. Structs are
//! positional arrays and variants are fixed maps with at most one short-string
//! key. This crate owns only that generic wire layer; FOKS protocol schemas and
//! cryptographic verification belong in higher-level crates.

#![forbid(unsafe_code)]

mod decode;
mod encode;
mod error;

pub use decode::{decode, validate};
pub use encode::encode;
pub use error::{Error, ErrorKind, PathSegment};

/// Maximum number of positional or variant steps below the root value.
pub const MAX_DEPTH: usize = 64;

/// A canonical Snowpack value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Unsigned(u64),
    /// A negative integer. Encoding rejects zero and positive values so that
    /// decoding and re-encoding have one canonical representation.
    Negative(i64),
    Binary(Vec<u8>),
    /// MessagePack string bytes. Schema types decide whether UTF-8 is required.
    Text(Vec<u8>),
    /// Positional struct/list representation. Canonical arrays are non-empty.
    Array(Vec<Value>),
    /// Snowpack variant representation. `None` is an empty fixed map; `Some`
    /// is a one-entry fixed map with a short-string tag.
    Variant(Option<(Vec<u8>, Box<Value>)>),
}
