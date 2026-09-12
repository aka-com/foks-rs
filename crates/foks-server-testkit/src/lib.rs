//! Test fixtures and harness for running isolated FOKS test servers.

#![forbid(unsafe_code)]

mod account;
mod binary;
mod certs;
mod client;
mod clock;
mod config;
mod environment;
pub mod oidc;
mod process;
mod scheduling;

pub use account::TestAccountSpec;
pub use binary::{BinaryExit, BinaryServer};
pub use client::TestClient;
pub use environment::{TestEnvironment, TestFault, TestProfile};
pub use foks_server::web_admin::WebAdminConfig;
pub use process::{InProcessServer, IsolatedTestServer, ProbeOverrideServer};
pub use scheduling::WriterQueuePressure;
