//! Qualification kernel, not an enabled server backend. The host owns one SQL
//! transaction, must roll it back on *any* error/trap, and proves durability only
//! after COMMIT. No authentication, ownership or durable-outcome claim is made
//! by this first boundary spike.

pub mod catalog;
mod dirents;
#[cfg(feature = "native")]
pub mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;
pub mod wire;

pub use catalog::{Statement, Value};
pub use dirents::{normalize_creation_times, put_dirents, Limits, Mutation};

pub const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BLOB_BYTES: usize = 4096;
pub const MAX_SQL_CALLS: usize = 2048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid(&'static str),
    IntegerRange,
    Conflict,
    Permission,
    Quota,
    Bounds,
    Storage,
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid",
            Self::IntegerRange => "integer_range",
            Self::Conflict => "conflict",
            Self::Permission => "permission",
            Self::Quota => "quota",
            Self::Bounds => "bounds",
            Self::Storage => "storage",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Only pinned catalog statements may execute. Queries return at most one row.
/// Hosts validate type, row count and cell length *before copying payloads*.
pub trait Sql {
    fn query(&mut self, statement: Statement, parameters: &[Value]) -> Result<Option<Vec<Value>>>;
    fn execute(&mut self, statement: Statement, parameters: &[Value]) -> Result<u64>;
}

pub(crate) fn integer(value: u64) -> Result<Value> {
    i64::try_from(value)
        .map(Value::Integer)
        .map_err(|_| Error::IntegerRange)
}
pub(crate) fn unsigned(value: &Value) -> Result<u64> {
    match value {
        Value::Integer(n) => u64::try_from(*n).map_err(|_| Error::IntegerRange),
        _ => Err(Error::Storage),
    }
}
