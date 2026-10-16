//! Explicit test-only pool geometry and pressure, without raw semaphore access.
use super::{maximum_reservable, Pools};
use crate::{Error, Result};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capacities {
    pub working: usize,
    pub small: usize,
    pub output: usize,
    pub control: usize,
    pub bulk: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub working: usize,
    pub small: usize,
    pub output: usize,
    pub control: usize,
    pub bulk: usize,
}

pub fn pools(capacities: Capacities) -> Result<Arc<Pools>> {
    for capacity in [
        capacities.working,
        capacities.small,
        capacities.output,
        capacities.control,
        capacities.bulk,
    ] {
        checked_count(capacity)?;
    }
    Ok(Arc::new(Pools {
        output: Arc::new(Semaphore::new(capacities.output)),
        working: Arc::new(Semaphore::new(capacities.working)),
        control: Arc::new(Semaphore::new(capacities.control)),
        small: Arc::new(Semaphore::new(capacities.small)),
        bulk: Arc::new(Semaphore::new(capacities.bulk)),
    }))
}

pub fn snapshot(pools: &Pools) -> Snapshot {
    Snapshot {
        working: pools.working.available_permits(),
        small: pools.small.available_permits(),
        output: pools.output.available_permits(),
        control: pools.control.available_permits(),
        bulk: pools.bulk.available_permits(),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PressurePool {
    Working,
    Output,
}

pub struct Pressure {
    _permit: OwnedSemaphorePermit,
}

fn checked_count(count: usize) -> Result<u32> {
    if count > maximum_reservable() {
        return Err(Error::Config("invalid test memory capacity"));
    }
    u32::try_from(count).map_err(|_| Error::Config("invalid test memory capacity"))
}

pub fn try_hold(pools: &Arc<Pools>, pool: PressurePool, count: usize) -> Result<Pressure> {
    let count = checked_count(count)?;
    let semaphore = match pool {
        PressurePool::Working => &pools.working,
        PressurePool::Output => &pools.output,
    };
    let permit = semaphore
        .clone()
        .try_acquire_many_owned(count)
        .map_err(|_| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "test memory pool unavailable",
            ))
        })?;
    Ok(Pressure { _permit: permit })
}
