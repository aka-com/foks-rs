use super::{RealtimeNotifier, RealtimeWakeTarget, RpcStatus, WakeKey};
use crate::metrics::realtime::{FanoutOperation, FanoutWork};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, Weak},
    time::Instant,
};
use tokio::sync::Notify;

const CLEANUP_BUDGET: usize = 8;
struct Registration {
    generation: u64,
    listener: Weak<Notify>,
}
#[derive(Default)]
struct Listeners {
    by_key: HashMap<WakeKey, Registration>,
    cleanup: VecDeque<(WakeKey, u64)>,
    generation: u64,
}
impl Listeners {
    fn clean(&mut self, work: &mut FanoutWork) {
        // Visit each queued registration at most once per call, even when small.
        for _ in 0..CLEANUP_BUDGET.min(self.cleanup.len()) {
            let (key, generation) = self.cleanup.pop_front().unwrap();
            work.entries_examined += 1;
            let Some(entry) = self.by_key.get(&key) else {
                continue;
            };
            if entry.generation != generation {
                continue;
            }
            if entry.listener.strong_count() == 0 {
                self.by_key.remove(&key);
                work.expired_entries_removed += 1;
            } else {
                self.cleanup.push_back((key, generation));
            }
        }
    }
}
pub(super) struct InboxHub {
    waiters: Mutex<Listeners>,
    metrics: Arc<crate::ServerMetrics>,
}
impl InboxHub {
    pub(super) fn new(metrics: Arc<crate::ServerMetrics>) -> Self {
        Self {
            waiters: Mutex::new(Listeners::default()),
            metrics,
        }
    }
    pub(super) fn listener(&self, target: &RealtimeWakeTarget) -> Result<Arc<Notify>, RpcStatus> {
        let key = WakeKey::from(target);
        let mut work = FanoutWork::default();
        let start = Instant::now();
        let mut waiters = self
            .waiters
            .lock()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let locked = Instant::now();
        work.mutex_wait = locked.duration_since(start);
        waiters.clean(&mut work);
        work.entries_examined += 1;
        let listener =
            if let Some(listener) = waiters.by_key.get(&key).and_then(|e| e.listener.upgrade()) {
                listener
            } else {
                let generation = waiters
                    .generation
                    .checked_add(1)
                    .ok_or(RpcStatus::TransactionRetry)?;
                waiters.generation = generation;
                let listener = Arc::new(Notify::new());
                if waiters
                    .by_key
                    .insert(
                        key.clone(),
                        Registration {
                            generation,
                            listener: Arc::downgrade(&listener),
                        },
                    )
                    .is_some()
                {
                    work.expired_entries_removed += 1;
                }
                waiters.cleanup.push_back((key, generation));
                listener
            };
        work.mutex_hold = locked.elapsed();
        drop(waiters);
        self.metrics
            .realtime
            .fanout(FanoutOperation::Registration, work);
        Ok(listener)
    }
    pub(super) fn notify_all(&self) {
        let mut work = FanoutWork::default();
        let start = Instant::now();
        let Ok(mut waiters) = self.waiters.lock() else {
            return;
        };
        let locked = Instant::now();
        work.mutex_wait = locked.duration_since(start);
        let mut listeners = Vec::new();
        waiters.by_key.retain(|_, entry| {
            work.entries_examined += 1;
            if let Some(listener) = entry.listener.upgrade() {
                listeners.push(listener);
                true
            } else {
                work.expired_entries_removed += 1;
                false
            }
        });
        work.mutex_hold = locked.elapsed();
        drop(waiters);
        self.wake(FanoutOperation::Membership, listeners, work);
    }
    fn wake(&self, operation: FanoutOperation, listeners: Vec<Arc<Notify>>, mut work: FanoutWork) {
        let start = Instant::now();
        work.listeners_woken = listeners.len() as u64;
        for listener in listeners {
            listener.notify_waiters();
        }
        work.wake = start.elapsed();
        self.metrics.realtime.fanout(operation, work);
    }
}
impl RealtimeNotifier for InboxHub {
    fn notify(&self, targets: &[RealtimeWakeTarget]) {
        let targets = targets.iter().map(WakeKey::from).collect::<HashSet<_>>();
        let mut work = FanoutWork {
            recipient_keys: targets.len() as u64,
            ..Default::default()
        };
        let mut listeners = Vec::with_capacity(targets.len());
        let start = Instant::now();
        let Ok(mut waiters) = self.waiters.lock() else {
            return;
        };
        let locked = Instant::now();
        work.mutex_wait = locked.duration_since(start);
        for key in targets {
            work.entries_examined += 1;
            if let Some(entry) = waiters.by_key.get(&key) {
                if let Some(listener) = entry.listener.upgrade() {
                    listeners.push(listener);
                } else {
                    waiters.by_key.remove(&key);
                    work.expired_entries_removed += 1;
                }
            }
        }
        work.mutex_hold = locked.elapsed();
        drop(waiters);
        self.wake(FanoutOperation::Recipients, listeners, work);
    }
}

#[cfg(test)]
mod tests;
