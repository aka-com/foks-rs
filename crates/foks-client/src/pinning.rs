//! One verify-and-accept span per host database at a time within a process.
//!
//! Every pin the client writes is accepted against the Merkle head the same
//! operation advanced: the head is fetched, verified and accepted, then the
//! user, team or generic chain verified under it is accepted with a rollback
//! check against the stored head. Two operations on one host interleaving
//! those steps would make the later acceptance of an earlier head read as a
//! rollback of the newer one. The profile operation lock kept them apart while
//! every session held it exclusively; sessions that share it are kept apart
//! here instead, for the span of one advance-and-accept, so the listing or
//! page read that follows can overlap while the acceptances cannot.
//!
//! A span is reentrant on its thread: an acceptance nested inside a guarded
//! load reuses the outer hold rather than waiting on it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};

struct Span {
    held: Mutex<bool>,
    released: Condvar,
}

fn spans() -> &'static Mutex<HashMap<PathBuf, Arc<Span>>> {
    static SPANS: OnceLock<Mutex<HashMap<PathBuf, Arc<Span>>>> = OnceLock::new();
    SPANS.get_or_init(Mutex::default)
}

thread_local! {
    static HELD: RefCell<HashMap<PathBuf, usize>> = RefCell::new(HashMap::new());
}

/// The hold on one host database's span. Dropping it ends the span, or one
/// level of a nested hold.
pub(crate) struct PinningSpan {
    key: PathBuf,
    span: Arc<Span>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

/// Enters the span for the host whose hard state lives at `database_path`,
/// waiting for another thread's span on the same database to end.
pub(crate) fn span(database_path: &Path) -> PinningSpan {
    let key = std::fs::canonicalize(database_path).unwrap_or_else(|_| database_path.to_path_buf());
    let nested = HELD.with(|held| {
        let mut held = held.borrow_mut();
        let depth = held.entry(key.clone()).or_insert(0);
        *depth += 1;
        *depth > 1
    });
    let span = Arc::clone(
        spans()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key.clone())
            .or_insert_with(|| {
                Arc::new(Span {
                    held: Mutex::new(false),
                    released: Condvar::new(),
                })
            }),
    );
    if !nested {
        let mut held = span.held.lock().unwrap_or_else(PoisonError::into_inner);
        while *held {
            held = span
                .released
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *held = true;
    }
    PinningSpan {
        key,
        span,
        _thread: std::marker::PhantomData,
    }
}

impl Drop for PinningSpan {
    fn drop(&mut self) {
        let last = HELD.with(|held| {
            let mut held = held.borrow_mut();
            if let Some(depth) = held.get_mut(&self.key) {
                *depth -= 1;
                if *depth == 0 {
                    held.remove(&self.key);
                    return true;
                }
            }
            false
        });
        if last {
            *self
                .span
                .held
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = false;
            self.span.released.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::span;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_span_is_reentrant_on_its_thread() {
        let path = std::path::Path::new("/pinning-test/reentrant.db");
        let outer = span(path);
        let inner = span(path);
        drop(inner);
        // The outer hold is still in place: another thread waits for it.
        let (entered, observed) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _held = span(std::path::Path::new("/pinning-test/reentrant.db"));
            entered.send(()).unwrap();
        });
        assert!(observed.recv_timeout(Duration::from_millis(100)).is_err());
        drop(outer);
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn dropping_the_outer_guard_keeps_a_nested_span_locked() {
        let path = std::path::Path::new("/pinning-test/drop-order.db");
        let outer = span(path);
        let inner = span(path);
        drop(outer);
        let (entered, observed) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _held = span(path);
            entered.send(()).unwrap();
        });
        assert!(observed.recv_timeout(Duration::from_millis(100)).is_err());
        drop(inner);
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn spans_on_one_database_serialize_across_threads() {
        let path = std::path::Path::new("/pinning-test/serial.db");
        let held = span(path);
        let (entered, observed) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _held = span(std::path::Path::new("/pinning-test/serial.db"));
            entered.send(()).unwrap();
        });
        assert!(observed.recv_timeout(Duration::from_millis(100)).is_err());
        drop(held);
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn spans_on_different_databases_do_not_wait() {
        let _held = span(std::path::Path::new("/pinning-test/one.db"));
        let (entered, observed) = mpsc::channel();
        let other = std::thread::spawn(move || {
            let _held = span(std::path::Path::new("/pinning-test/two.db"));
            entered.send(()).unwrap();
        });
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        other.join().unwrap();
    }
}
