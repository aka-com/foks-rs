use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub(crate) struct TestClock {
    now: AtomicU64,
}

impl TestClock {
    pub(crate) fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
        }
    }

    pub(crate) fn set(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }

    pub(crate) fn advance(&self, microseconds: u64) -> u64 {
        self.now
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |now| {
                now.checked_add(microseconds)
            })
            .expect("test clock overflow")
            + microseconds
    }

    pub(crate) fn now(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

impl foks_server_db::Clock for TestClock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        Ok(self.now())
    }
}
