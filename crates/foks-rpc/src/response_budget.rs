//! Optional allocation admission for a server's response encoders.
//!
//! A scope is synchronous and thread local. Async callers must install it for
//! each poll, never keep a thread-local scope alive across an await. The owner
//! of the budget must retain its reservations with the returned bytes.

use std::{cell::RefCell, sync::Arc};

pub trait ResponseAllocationBudget: Send + Sync {
    /// Reserve before allocating. `control` identifies a small status/void reply,
    /// not an exemption from the caller's bounded memory policy.
    fn reserve(&self, bytes: usize, control: bool) -> bool;
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<dyn ResponseAllocationBudget>>> = const { RefCell::new(None) };
}

pub fn with_response_budget<T>(
    budget: Arc<dyn ResponseAllocationBudget>,
    f: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<Arc<dyn ResponseAllocationBudget>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|current| *current.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(CURRENT.with(|current| current.replace(Some(budget))));
    f()
}

pub(crate) fn reserve_response(bytes: usize, control: bool) -> crate::Result<()> {
    let accepted = CURRENT.with(|current| {
        let budget = current.borrow().clone();
        budget.is_none_or(|budget| budget.reserve(bytes, control))
    });
    if accepted {
        Ok(())
    } else {
        Err(crate::Error::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "response memory admission exhausted",
        )))
    }
}

pub(crate) fn reserve_success(length: usize) -> crate::Result<()> {
    if length > crate::DEFAULT_MAX_FRAME_LENGTH {
        return Err(crate::Error::FrameTooLarge {
            received: length,
            maximum: crate::DEFAULT_MAX_FRAME_LENGTH,
        });
    }
    // The envelope and final framed buffer coexist. Encoders reserve an exact
    // conservative capacity so Vec growth cannot escape this allowance.
    reserve_response(2 * (length + 128), false)
}
