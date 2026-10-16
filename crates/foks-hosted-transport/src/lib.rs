//! Shared native request ownership and bounded FOKS transport helpers.
//! This crate supplies no database, TLS authority, hosted route or readiness proof.

pub mod deadline;
mod error;
pub mod framing;
pub use error::{Error, Result};

/// One allocation owner shared by codecs, retained SQL results and physical work.
///
/// Qualified admission is deliberately absent from a default external build.
#[cfg_attr(feature = "qualification", doc = "```rust")]
#[cfg_attr(not(feature = "qualification"), doc = "```compile_fail,E0599")]
/// use std::sync::Arc;
/// use foks_hosted_transport::memory::{Pools, Request};
/// fn admitted(pools: &Arc<Pools>) -> foks_metadata_core::Result<Arc<Request>> {
///     pools.admitted_request(None)
/// }
/// ```
///
/// Pool diagnostics require the separate test-support feature.
#[cfg_attr(feature = "test-support", doc = "```rust")]
#[cfg_attr(not(feature = "test-support"), doc = "```compile_fail,E0432")]
/// use foks_hosted_transport::memory::{Pools, test_support};
/// fn inspect(pools: &Pools) {
///     let _ = test_support::snapshot(pools);
/// }
/// ```
pub mod memory;
