//! Test fixtures and harness for running isolated FOKS test servers.

#![forbid(unsafe_code)]

mod account;
mod binary;
#[cfg(feature = "celld-proof")]
pub mod celld_proof;
mod certs;
mod client;
mod clock;
mod config;
mod environment;
pub mod oidc;
mod process;
mod recovery;
mod scheduling;

pub use account::TestAccountSpec;
pub use binary::{BinaryExit, BinaryServer};
pub use client::TestClient;
pub use environment::{
    TestEnvironment, TestFault, TestProfile, FEW_REQUESTS_PER_CONNECTION, SHORT_REQUEST_TIMEOUT,
    TIGHT_IO_IDLE_TIMEOUT, WAITLIST_TEST_ENTRIES,
};
pub use foks_server::web_admin::WebAdminConfig;
pub use process::{InProcessServer, IsolatedTestServer, ProbeOverrideServer};
pub use recovery::RecoveryLedger;
pub use scheduling::WriterQueuePressure;
