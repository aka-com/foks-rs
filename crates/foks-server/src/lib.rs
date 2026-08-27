//! Standalone, single-host FOKS v0.1.9 server.
//!
//! The implementation is introduced in independently tested layers. The
//! executable protocol contract and PKI feasibility tests intentionally land
//! before network or persistence authority.

#![forbid(unsafe_code)]

mod auth;
mod config;
mod diagnostics;
mod entropy;
mod error;
mod fault;
pub mod host;
mod identity;
pub mod keys;
mod maintenance;
mod metrics;
pub mod net;
mod operations;
pub mod pki;
mod rate_limit;
mod read_pool;
pub mod rpc;
mod services;
mod standalone;
mod writer;

pub use config::{Config, ReadDatabaseConfig, SessionLimits};
pub use diagnostics::{SessionDiagnostics, SessionErrorClass, StderrSessionDiagnostics};
pub use entropy::{Entropy, OsEntropy};
pub use error::{Error, Result};
#[doc(hidden)]
pub use fault::{SessionFaultPoint, SessionFaults};
pub use metrics::{ServerMetrics, ServerMetricsSnapshot};
pub use net::{start, RunningServer, ServerAddresses};
pub use rate_limit::RateLimitConfig;
pub use standalone::{
    backup_standalone_installation, restore_backup, start_standalone, BackupArtifacts,
    BackupSchedule, RunningStandaloneServer, StandaloneConfig,
};
pub use writer::{Writer, WriterHandle, WriterMetrics};
