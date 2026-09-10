//! Canonical Snowpack encoding for FOKS v0.1.9.
//!
//! Snowpack uses a restricted MessagePack subset. Structs are represented
//! exclusively as positional arrays (`toarray`), and variants are fixed maps with at most one short-string
//! key. This crate owns only that generic wire layer; FOKS protocol schemas and
//! cryptographic verification belong in higher-level crates.

#![forbid(unsafe_code)]

mod decode;
mod encode;
mod error;

pub use decode::{decode, decode_prefix, decode_sensitive, validate, validate_signable};
pub use encode::{encode, encode_ref};
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

/// A borrowed canonical Snowpack value.
///
/// This form is useful for sensitive plaintexts: binary and text fields can
/// be encoded directly from zeroizing or caller-owned storage without first
/// cloning them into an ordinary [`Vec`].
#[derive(Debug, Eq, PartialEq)]
pub enum ValueRef<'a> {
    /// An owned nonsensitive subtree used alongside borrowed sensitive fields.
    Owned(Value),
    /// An already-owned subtree that can be embedded without cloning it.
    Value(&'a Value),
    Null,
    Bool(bool),
    Unsigned(u64),
    Negative(i64),
    Binary(&'a [u8]),
    Text(&'a [u8]),
    Array(Vec<ValueRef<'a>>),
    Variant(Option<(&'a [u8], Box<ValueRef<'a>>)>),
}

impl<'a> From<&'a Value> for ValueRef<'a> {
    fn from(value: &'a Value) -> Self {
        Self::Value(value)
    }
}

// Opt-in erasure; ordinary Value ownership and destructuring remain unchanged.
impl zeroize::Zeroize for Value {
    fn zeroize(&mut self) {
        match self {
            Self::Binary(bytes) | Self::Text(bytes) => bytes.zeroize(),
            Self::Array(values) => values.zeroize(),
            Self::Variant(Some((tag, value))) => {
                tag.zeroize();
                value.zeroize();
            }
            _ => {}
        }
        *self = Self::Null;
    }
}
