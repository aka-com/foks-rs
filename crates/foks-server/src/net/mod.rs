mod listener;
pub(crate) mod session;

pub(crate) use listener::bind_addresses;
pub use listener::{RunningServer, ServerAddresses};

use crate::{Config, Result};

pub fn start(config: Config) -> Result<RunningServer> {
    RunningServer::start(config)
}
