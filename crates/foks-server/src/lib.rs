//! Standalone, single-host FOKS v0.1.9 server.
//!
//! The implementation is introduced in independently tested layers. The
//! executable protocol contract and PKI feasibility tests intentionally land
//! before network or persistence authority.

#![forbid(unsafe_code)]

mod config;
mod error;
pub mod keys;
pub mod net;
pub mod pki;
pub mod rpc;

pub use config::{Config, SessionLimits};
pub use error::{Error, Result};
pub use net::{start, RunningServer, ServerAddresses};
