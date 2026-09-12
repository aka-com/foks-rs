//! Epoch-local, suspend-aware deadlines; UTC remains authoritative for certificates.
use crate::{Entropy, Error, Result};
use foks_server_db::AdminMoment;
use std::sync::{Arc, Mutex};
pub trait AdminElapsedClock: Send + Sync {
    fn now_us(&self) -> Result<u64>;
}
pub struct SuspendClock;
impl AdminElapsedClock for SuspendClock {
    fn now_us(&self) -> Result<u64> {
        #[cfg(target_os = "linux")]
        let id = nix::time::ClockId::CLOCK_BOOTTIME;
        #[cfg(target_os = "macos")]
        let id = nix::time::ClockId::from_raw(libc::CLOCK_MONOTONIC_RAW);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let value = nix::time::clock_gettime(id).map_err(std::io::Error::from)?;
            let seconds = u64::try_from(value.tv_sec())
                .map_err(|_| Error::Config("invalid elapsed clock"))?;
            let nanos = u64::try_from(value.tv_nsec())
                .map_err(|_| Error::Config("invalid elapsed clock"))?;
            seconds
                .checked_mul(1_000_000)
                .and_then(|v| v.checked_add(nanos / 1000))
                .ok_or(Error::Config("elapsed clock overflow"))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(Error::Config("administration needs a suspend-aware clock"))
        }
    }
}
struct State {
    epoch: [u8; 16],
    utc: u64,
    elapsed: u64,
    fenced: bool,
}
pub struct AdminClock {
    utc: Arc<dyn foks_server_db::Clock>,
    elapsed: Arc<dyn AdminElapsedClock>,
    entropy: Arc<dyn Entropy>,
    state: Mutex<State>,
}
impl AdminClock {
    pub fn new(
        utc: Arc<dyn foks_server_db::Clock>,
        elapsed: Arc<dyn AdminElapsedClock>,
        entropy: Arc<dyn Entropy>,
    ) -> Result<Self> {
        let mut epoch = [0; 16];
        entropy.fill(&mut epoch)?;
        let wall = utc.now_micros()?;
        let monotonic = elapsed.now_us()?;
        Ok(Self {
            utc,
            elapsed,
            entropy,
            state: Mutex::new(State {
                epoch,
                utc: wall,
                elapsed: monotonic,
                fenced: false,
            }),
        })
    }
    pub fn sample(&self) -> Result<AdminMoment> {
        let mut state = self.state.lock().map_err(|_| Error::Thread)?;
        let utc = self.utc.now_micros()?;
        let elapsed = self.elapsed.now_us()?;
        if utc < state.utc || elapsed < state.elapsed {
            state.fenced = true;
            return Err(Error::Config("admin clock is fenced"));
        }
        if state.fenced {
            self.entropy.fill(&mut state.epoch)?;
            state.fenced = false;
        }
        state.utc = utc;
        state.elapsed = elapsed;
        Ok(AdminMoment {
            epoch: state.epoch,
            utc_us: utc,
            elapsed_us: elapsed,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Clock(AtomicU64);
    impl foks_server_db::Clock for Clock {
        fn now_micros(&self) -> foks_server_db::Result<u64> {
            Ok(self.0.load(Ordering::SeqCst))
        }
    }
    impl AdminElapsedClock for Clock {
        fn now_us(&self) -> Result<u64> {
            Ok(self.0.load(Ordering::SeqCst))
        }
    }
    struct Ent;
    impl Entropy for Ent {
        fn fill(&self, bytes: &mut [u8]) -> Result<()> {
            getrandom::fill(bytes).map_err(|_| Error::Config("entropy"))
        }
    }
    #[test]
    fn wall_rollback_fences_old_epoch_and_stabilization_never_revives_it() {
        let wall = Arc::new(Clock(AtomicU64::new(100)));
        let elapsed = Arc::new(Clock(AtomicU64::new(10)));
        let c = AdminClock::new(wall.clone(), elapsed.clone(), Arc::new(Ent)).unwrap();
        let first = c.sample().unwrap();
        wall.0.store(99, Ordering::SeqCst);
        assert!(c.sample().is_err());
        elapsed.0.store(500, Ordering::SeqCst);
        assert!(c.sample().is_err());
        wall.0.store(100, Ordering::SeqCst);
        let next = c.sample().unwrap();
        assert_ne!(first.epoch, next.epoch);
        assert_eq!(next.elapsed_us, 500);
    }
    #[test]
    fn platform_clock_advances() {
        let c = SuspendClock;
        let a = c.now_us().unwrap();
        assert!(c.now_us().unwrap() >= a);
    }
}
