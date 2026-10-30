mod accept;
mod listener;
pub(crate) mod memory;
pub(crate) mod session;

pub(crate) use accept::retry_accept;
pub use accept::AcceptErrorKind;
pub(crate) use listener::bind_addresses;
pub use listener::{RunningServer, ServerAddresses};

use crate::{Config, Result};

pub fn start(config: Config) -> Result<RunningServer> {
    RunningServer::start(config)
}

#[cfg(feature = "celld-proof")]
pub fn start_proof(
    config: Config,
    proof: std::sync::Arc<dyn crate::proof::RpcProof>,
) -> Result<RunningServer> {
    RunningServer::start_proof(config, proof)
}
