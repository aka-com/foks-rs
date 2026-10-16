//! Qualification kernel, not an enabled server backend. The host owns one SQL
//! transaction, must roll it back on *any* error/trap, and proves durability only
//! after COMMIT. The private authority fixture adds owner/principal-binding and
//! durable-outcome state machines; it is not real FOKS authentication, a production
//! backend, a distributed deadline guarantee, or a qualified recovery deployment.

pub mod authority;
pub mod catalog;
mod dirents;
#[cfg(feature = "native")]
pub mod native;
pub mod page;
pub mod page_catalog;
pub mod personal;
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
    Expired,
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
            Self::Expired => "expired",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Only pinned catalog statements may execute. Queries return at most one row.
/// Hosts validate type, row count and cell length *before copying payloads*.
pub trait Sql {
    fn query(&mut self, statement: Statement, parameters: &[Value]) -> Result<Option<Vec<Value>>>;
    fn execute(&mut self, statement: Statement, parameters: &[Value]) -> Result<u64>;

    /// Separate private page ABI; unsupported hosts never fall back to scalar SQL.
    fn query_page(
        &mut self,
        _statement: page_catalog::PageStatement,
        _parameters: &[Value],
    ) -> Result<page::PageBuffer> {
        Err(Error::Bounds)
    }

    /// Run only the shared domain kernel in a nested rollback scope. Hosts with
    /// callback-only transaction APIs override this operation, never the kernel.
    /// All non-domain errors must escape the host-owned outer transaction.
    fn domain(&mut self, input: &[u8]) -> Result<wire::Reply>
    where
        Self: Sized,
    {
        self.execute(Statement::AuthoritySavepoint, &[])?;
        let result = wire::run(self, input);
        match &result {
            Ok(_) => {}
            Err(Error::Conflict | Error::Permission | Error::Quota) => {
                self.execute(Statement::AuthorityRollback, &[])?;
            }
            Err(_) => return result,
        }
        self.execute(Statement::AuthorityRelease, &[])?;
        result
    }
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
