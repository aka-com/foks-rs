//! Sealed test-only composition for a process-equivalent isolated server.

#![forbid(unsafe_code)]

mod certs;
mod config;
mod process;

pub use process::IsolatedTestServer;
