use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, Result};

pub trait Clock: Send + Sync {
    fn now_micros(&self) -> Result<u64>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_micros(&self) -> Result<u64> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Invalid("system time precedes Unix epoch"))?;
        u64::try_from(elapsed.as_micros()).map_err(|_| Error::IntegerRange)
    }
}
