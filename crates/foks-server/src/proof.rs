//! Opt-in qualification seam, not a supported production backend.
//!
//! Only `net::start_proof` installs this hook. The implementation must own a
//! bounded, exclusive operation guard until the callback, detached native
//! writer work, authoritative publication and response proof have completed.

use crate::{ReadDatabaseConfig, Result, WriterHandle};

pub struct Projection {
    pub read_database: ReadDatabaseConfig,
    pub writer: WriterHandle,
}

pub trait RpcProof: Send + Sync {
    /// Execute the callback at most once against a disposable projection.
    /// Returned bytes are only a candidate: never return them to the server
    /// before authoritative SQL commit/ownership/durability proof succeeds.
    /// This includes read-only replies and application-error replies.
    fn execute(
        &self,
        route: Option<crate::rpc::RouteId>,
        operation: Box<dyn FnOnce(Projection) -> Result<Vec<u8>> + '_>,
    ) -> Result<Vec<u8>>;
}
