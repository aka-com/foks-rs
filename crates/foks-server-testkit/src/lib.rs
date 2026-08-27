//! Sealed test-only composition for a process-equivalent isolated server.

#![forbid(unsafe_code)]

mod account;
mod binary;
mod certs;
mod client;
mod clock;
mod config;
mod environment;
mod process;
mod scheduling;

pub use account::TestAccountSpec;
pub use binary::{BinaryExit, BinaryServer};
pub use client::TestClient;
pub use environment::{TestEnvironment, TestFault, TestProfile};
pub use process::{InProcessServer, IsolatedTestServer, ProbeOverrideServer};
pub use scheduling::WriterQueuePressure;
