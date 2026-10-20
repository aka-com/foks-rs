use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KexMetricsSnapshot {
    pub active: u64,
    pub admitted: u64,
    pub rejected: u64,
    pub delivered: u64,
    pub timed_out: u64,
    pub disconnected: u64,
    /// Receives the client cancelled with a CANCEL control message.
    pub cancelled: u64,
    pub shutdown: u64,
    pub failed: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Completion {
    Delivered,
    TimedOut,
    Disconnected,
    Cancelled,
    Shutdown,
    Failed,
}

#[derive(Default)]
pub(crate) struct KexMetrics(Mutex<KexMetricsSnapshot>);

impl KexMetrics {
    pub(super) fn snapshot(&self) -> KexMetricsSnapshot {
        *self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn rejected(&self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .rejected += 1;
    }
}

/// Admission and its metric share one lifetime, including early exits/unwinding.
/// The permit is retained until this guard drops after response handling.
pub(crate) struct Poll {
    metrics: Arc<super::ServerMetrics>,
    completion: Completion,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl Poll {
    pub(crate) fn new(
        metrics: Arc<super::ServerMetrics>,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Self {
        {
            let mut value = metrics
                .kex
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            value.active += 1;
            value.admitted += 1;
        }
        Self {
            metrics,
            completion: Completion::Failed,
            permit: Some(permit),
        }
    }

    pub(crate) fn complete(&mut self, completion: Completion) {
        self.completion = completion;
    }
}

impl Drop for Poll {
    fn drop(&mut self) {
        let mut value = self
            .metrics
            .kex
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Publish zero active only once the corresponding slot is reusable.
        drop(self.permit.take());
        value.active -= 1;
        match self.completion {
            Completion::Delivered => value.delivered += 1,
            Completion::TimedOut => value.timed_out += 1,
            Completion::Disconnected => value.disconnected += 1,
            Completion::Cancelled => value.cancelled += 1,
            Completion::Shutdown => value.shutdown += 1,
            Completion::Failed => value.failed += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_terminal_reason_and_early_drop_balances_metrics_and_permits() {
        let metrics = Arc::new(super::super::ServerMetrics::default());
        let permits = Arc::new(tokio::sync::Semaphore::new(1));
        for completion in [
            Some(Completion::Delivered),
            Some(Completion::TimedOut),
            Some(Completion::Disconnected),
            Some(Completion::Cancelled),
            Some(Completion::Shutdown),
            None,
        ] {
            let mut poll = Poll::new(
                metrics.clone(),
                permits.clone().try_acquire_owned().unwrap(),
            );
            assert_eq!(metrics.snapshot().kex.active, 1);
            assert!(permits.clone().try_acquire_owned().is_err());
            if let Some(completion) = completion {
                poll.complete(completion);
            }
            drop(poll);
            assert_eq!(metrics.snapshot().kex.active, 0);
            assert_eq!(permits.available_permits(), 1);
        }
        let observed = metrics.snapshot().kex;
        assert_eq!(observed.admitted, 6);
        assert_eq!(
            (
                observed.delivered,
                observed.timed_out,
                observed.disconnected,
                observed.cancelled,
                observed.shutdown,
                observed.failed
            ),
            (1, 1, 1, 1, 1, 1)
        );
    }
}
