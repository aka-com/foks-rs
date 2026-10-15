//! Process-local rate limit for maintenance scans at session entry.
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Records when a maintenance pass last completed for each key, so a pass
/// runs at most once per `INTERVAL` per key. Nothing is persisted; at most
/// `CAPACITY` keys are retained, and the smallest key is evicted to admit a
/// new one.
pub(crate) struct ScanLimiter<K> {
    last: Mutex<BTreeMap<K, Instant>>,
}

impl<K: Ord> ScanLimiter<K> {
    const INTERVAL: Duration = Duration::from_secs(60);
    const CAPACITY: usize = 128;

    pub(crate) const fn new() -> Self {
        Self {
            last: Mutex::new(BTreeMap::new()),
        }
    }

    fn entries(&self) -> MutexGuard<'_, BTreeMap<K, Instant>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether no pass for `key` has completed within the interval.
    pub(crate) fn due(&self, key: &K) -> bool {
        self.entries()
            .get(key)
            .is_none_or(|last| last.elapsed() >= Self::INTERVAL)
    }

    /// Records a completed pass for `key`.
    pub(crate) fn record(&self, key: K) {
        let mut entries = self.entries();
        if entries.len() >= Self::CAPACITY && !entries.contains_key(&key) {
            entries.pop_first();
        }
        entries.insert(key, Instant::now());
    }

    #[cfg(test)]
    pub(crate) fn forget(&self, key: &K) {
        self.entries().remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_suppress_until_forgotten_and_capacity_evicts_smallest_key() {
        let limiter = ScanLimiter::<u32>::new();
        assert!(limiter.due(&1));
        limiter.record(1);
        assert!(!limiter.due(&1));
        limiter.forget(&1);
        assert!(limiter.due(&1));

        for key in 0..ScanLimiter::<u32>::CAPACITY as u32 {
            limiter.record(key);
        }
        limiter.record(0);
        assert!(!limiter.due(&0));
        limiter.record(1000);
        assert!(limiter.due(&0));
        assert!(!limiter.due(&1));
        assert!(!limiter.due(&1000));
        assert_eq!(limiter.entries().len(), ScanLimiter::<u32>::CAPACITY);
    }
}
