//! Standalone, single-host FOKS v0.1.9 server.
//!
//! The implementation is introduced in independently tested layers. The
//! executable protocol contract and PKI feasibility tests intentionally land
//! before network or persistence authority.

#![forbid(unsafe_code)]

mod auth;
mod config;
mod entropy;
mod error;
pub mod host;
mod identity;
pub mod keys;
pub mod net;
pub mod pki;
pub mod rpc;
mod services;
mod standalone;
mod writer;

pub use config::{Config, ReadDatabaseConfig, SessionLimits};
pub use entropy::{Entropy, OsEntropy};
pub use error::{Error, Result};
pub use net::{start, RunningServer, ServerAddresses};
pub use standalone::{start_standalone, RunningStandaloneServer, StandaloneConfig};
pub use writer::{Writer, WriterHandle};
