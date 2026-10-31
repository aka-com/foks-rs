//! Detection of unexpected core-thread exits.
//!
//! The standalone server runs its RPC listeners, SQLite writer, maintenance
//! loop, automatic backups, management endpoint and the optional
//! administration and OIDC HTTP listeners on dedicated threads. None of them
//! restarts in place: a writer that panicked may have abandoned a
//! transaction, and a listener whose accept loop ended no longer serves its
//! socket. The first unexpected exit is recorded here, `/healthz` and
//! `/readyz` report it, and the `foks-server` binary exits non-zero so that the
//! service manager restarts the process.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// A server thread whose unexpected exit leaves the process without a
/// required service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CoreThread {
    /// The probe, public-services and authenticated RPC listeners.
    Listener,
    /// The SQLite writer.
    Writer,
    /// Periodic expiry, upload reclamation and WAL checkpoints.
    Maintenance,
    /// Scheduled automatic backups.
    Backup,
    /// The loopback `/healthz`, `/readyz` and `/metrics` listener.
    Management,
    /// The optional loopback web administration listener.
    Administration,
    /// The optional OIDC browser start and callback listener.
    Oidc,
}

impl CoreThread {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Listener => "listener",
            Self::Writer => "writer",
            Self::Maintenance => "maintenance",
            Self::Backup => "backup",
            Self::Management => "management",
            Self::Administration => "administration",
            Self::Oidc => "oidc",
        }
    }
}

type Observer = Box<dyn FnOnce(CoreThread) + Send>;

#[derive(Default)]
struct FatalState {
    exited: Option<CoreThread>,
    observers: Vec<Observer>,
}

/// Records the first unexpected core-thread exit and runs each registered
/// observer once. Later exits are not recorded; the first one identifies the
/// failure, and the others usually follow from the shutdown it causes.
#[derive(Clone, Default)]
pub(crate) struct FatalSignal(Arc<Mutex<FatalState>>);

impl FatalSignal {
    pub(crate) fn notify(&self, thread: CoreThread) {
        let observers = {
            let mut state = self.lock();
            if state.exited.is_some() {
                return;
            }
            state.exited = Some(thread);
            std::mem::take(&mut state.observers)
        };
        for observer in observers {
            observer(thread);
        }
    }

    pub(crate) fn exited(&self) -> Option<CoreThread> {
        self.lock().exited
    }

    /// Registers `observer` to run on the first exit, or runs it on the
    /// calling thread when an exit is already recorded. Otherwise observers
    /// run on the exiting thread, possibly while it unwinds from a panic, so
    /// they must not block or panic.
    pub(crate) fn observe(&self, observer: Observer) {
        let mut state = self.lock();
        match state.exited {
            Some(thread) => {
                drop(state);
                observer(thread);
            }
            None => state.observers.push(observer),
        }
    }

    /// Returns a guard that reports `thread` when dropped before `end`
    /// marks a requested stop: on an early return, an error, or a panic.
    pub(crate) fn exit_guard(&self, thread: CoreThread) -> ExitGuard {
        ExitGuard {
            fatal: self.clone(),
            thread,
            armed: true,
        }
    }

    fn lock(&self) -> MutexGuard<'_, FatalState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) struct ExitGuard {
    fatal: FatalSignal,
    thread: CoreThread,
    armed: bool,
}

impl ExitGuard {
    /// Ends the guarded thread's monitoring. Reports the exit now unless the
    /// thread is ending because a stop was requested.
    pub(crate) fn end(mut self, requested: bool) {
        self.armed = !requested;
    }
}

impl Drop for ExitGuard {
    fn drop(&mut self) {
        if self.armed {
            self.fatal.notify(self.thread);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_exit_is_recorded_and_observers_run_once() {
        let fatal = FatalSignal::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        let early = sender.clone();
        fatal.observe(Box::new(move |thread| early.send(thread).unwrap()));
        assert_eq!(fatal.exited(), None);
        fatal.notify(CoreThread::Writer);
        fatal.notify(CoreThread::Listener);
        assert_eq!(fatal.exited(), Some(CoreThread::Writer));
        fatal.observe(Box::new(move |thread| sender.send(thread).unwrap()));
        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            [CoreThread::Writer, CoreThread::Writer]
        );
    }

    #[test]
    fn exit_guard_reports_every_end_except_a_requested_stop() {
        let fatal = FatalSignal::default();
        fatal.exit_guard(CoreThread::Maintenance).end(true);
        assert_eq!(fatal.exited(), None);

        let guard = fatal.exit_guard(CoreThread::Backup);
        let panicked = std::thread::spawn(move || {
            let _guard = guard;
            panic!("injected backup thread panic");
        })
        .join();
        assert!(panicked.is_err());
        assert_eq!(fatal.exited(), Some(CoreThread::Backup));

        let fatal = FatalSignal::default();
        drop(fatal.exit_guard(CoreThread::Oidc));
        assert_eq!(fatal.exited(), Some(CoreThread::Oidc));

        let fatal = FatalSignal::default();
        fatal.exit_guard(CoreThread::Management).end(false);
        assert_eq!(fatal.exited(), Some(CoreThread::Management));
    }
}
