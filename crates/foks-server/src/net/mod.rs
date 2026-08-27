mod listener;
mod session;

pub use listener::{RunningServer, ServerAddresses};

use crate::{Config, Result};

pub fn start(config: Config) -> Result<RunningServer> {
    RunningServer::start(config)
}
