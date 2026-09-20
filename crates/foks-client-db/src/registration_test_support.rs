//! Opt-in, thread-scoped observation of registration work only. No identifiers
//! or secrets are retained, and no counters exist in default production builds.
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RegistrationWork {
    pub database_opens: usize,
    pub batches: usize,
    pub read_transactions: usize,
    pub write_transactions: usize,
}
thread_local! {
    static WORK: Cell<Option<RegistrationWork>> = const { Cell::new(None) };
}

pub fn observe<T>(operation: impl FnOnce() -> T) -> (T, RegistrationWork) {
    struct Restore(Option<RegistrationWork>);
    impl Drop for Restore {
        fn drop(&mut self) {
            WORK.with(|s| s.set(self.0));
        }
    }
    let _restore = Restore(WORK.with(|s| s.replace(Some(RegistrationWork::default()))));
    let result = operation();
    (result, WORK.with(|s| s.get().unwrap()))
}

/// Used by the scheduler immediately before its registration-specific open.
pub fn registration_opened() {
    record(|s| s.database_opens += 1);
}
pub(crate) fn record(update: impl FnOnce(&mut RegistrationWork)) {
    WORK.with(|slot| {
        if let Some(mut work) = slot.get() {
            update(&mut work);
            slot.set(Some(work));
        }
    });
}
