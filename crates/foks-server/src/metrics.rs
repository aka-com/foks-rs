use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerMetricsSnapshot {
    pub requests_started: u64,
    pub responses_completed: u64,
}

#[derive(Default)]
pub struct ServerMetrics {
    requests_started: AtomicU64,
    responses_completed: AtomicU64,
}

impl ServerMetrics {
    pub fn snapshot(&self) -> ServerMetricsSnapshot {
        ServerMetricsSnapshot {
            requests_started: self.requests_started.load(Ordering::Relaxed),
            responses_completed: self.responses_completed.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn request_started(&self) {
        self.requests_started.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn response_completed(&self) {
        self.responses_completed.fetch_add(1, Ordering::Relaxed);
    }
}
