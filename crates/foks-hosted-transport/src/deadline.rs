//! One monotonic ordinary-request budget. Expiration stops new work; it cannot
//! undo a mutation or prove termination of an already admitted storage attempt.
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use crate::{Error, Result};

#[derive(Clone, Copy, Debug)]
pub struct Deadline(Instant);

thread_local! {
    static CURRENT: Cell<Option<Deadline>> = const { Cell::new(None) };
}

impl Deadline {
    /// Preserve an existing monotonic deadline, including one already expired.
    /// This does not grant a new budget when an operation changes owners.
    pub fn at(instant: Instant) -> Self {
        Self(instant)
    }

    pub fn after(budget: Duration) -> Result<Self> {
        Instant::now()
            .checked_add(budget)
            .map(Self)
            .ok_or(Error::Config("request deadline overflows monotonic clock"))
    }

    pub fn instant(self) -> Instant {
        self.0
    }

    pub fn remaining(self) -> Result<Duration> {
        self.0
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(expired)
    }
}

pub fn expired() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "request deadline expired",
    ))
}

pub fn current() -> Option<Deadline> {
    CURRENT.with(Cell::get)
}

pub fn check() -> Result<()> {
    if let Some(deadline) = current() {
        deadline.remaining()?;
    }
    Ok(())
}

/// Synchronous scope only: never hold a thread-local context across an await.
pub fn scope<T>(deadline: Option<Deadline>, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<Deadline>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|current| current.set(self.0));
        }
    }
    let _restore = Restore(CURRENT.with(|current| current.replace(deadline)));
    operation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_deadline_preserves_original_instant_and_expiry() {
        let original = Instant::now() + Duration::from_secs(60);
        assert_eq!(Deadline::at(original).instant(), original);
        let expired = Instant::now() - Duration::from_secs(1);
        let deadline = Deadline::at(expired);
        assert_eq!(deadline.instant(), expired);
        assert!(deadline.remaining().is_err());
        scope(Some(deadline), || assert!(check().is_err()));
        assert!(current().is_none());
    }

    #[test]
    fn expired_scope_rejects_new_work_and_nested_cleanup_restores_it() {
        let deadline = Deadline(Instant::now() - Duration::from_secs(1));
        assert!(current().is_none());
        scope(Some(deadline), || {
            assert!(check().is_err());
            scope(None, || assert!(check().is_ok()));
            assert!(check().is_err());
            assert!(deadline.remaining().is_err());
        });
        assert!(current().is_none());
    }

    #[test]
    fn scope_restores_context_after_unwind_and_never_extends_budget() {
        let deadline = Deadline::after(Duration::from_secs(15)).unwrap();
        assert!(deadline.remaining().unwrap() <= Duration::from_secs(15));
        let _ = std::panic::catch_unwind(|| scope(Some(deadline), || panic!("injected")));
        assert!(current().is_none());
    }
}
