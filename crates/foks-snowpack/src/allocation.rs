//! Optional synchronous allocation admission. Callers retain the allowance until
//! all derived values are dropped, and reinstall scopes on each async poll.
use std::{cell::RefCell, sync::Arc};

pub trait AllocationBudget: Send + Sync {
    fn reserve(&self, bytes: usize) -> bool;
}
thread_local! {
    static CURRENT: RefCell<Option<Arc<dyn AllocationBudget>>> = const { RefCell::new(None) };
}
pub fn with_allocation_budget<T>(budget: Arc<dyn AllocationBudget>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<dyn AllocationBudget>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|current| *current.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(CURRENT.with(|current| current.replace(Some(budget))));
    f()
}
/// Also used by database readers before copying borrowed SQLite values.
pub fn reserve_allocation(bytes: usize) -> Result<(), crate::Error> {
    let accepted = CURRENT.with(|current| {
        let budget = current.borrow().clone();
        budget.is_none_or(|budget| budget.reserve(bytes))
    });
    if accepted {
        Ok(())
    } else {
        Err(crate::Error::new(crate::ErrorKind::AllocationLimit, 0, &[]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Limit(std::sync::atomic::AtomicUsize);
    impl AllocationBudget for Limit {
        fn reserve(&self, bytes: usize) -> bool {
            self.0
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |left| left.checked_sub(bytes),
                )
                .is_ok()
        }
    }
    #[test]
    fn expansion_and_encode_are_admitted_and_scope_restores_on_error() {
        let input = vec![0x91, 0xc0];
        let budget = Arc::new(Limit(std::sync::atomic::AtomicUsize::new(1)));
        let error = with_allocation_budget(budget.clone(), || crate::decode(&input)).unwrap_err();
        assert_eq!(error.kind, crate::ErrorKind::AllocationLimit);
        let error = with_allocation_budget(budget, || {
            crate::encode(&crate::Value::Binary(vec![0; 1024]))
        })
        .unwrap_err();
        assert_eq!(error.kind, crate::ErrorKind::AllocationLimit);
        assert!(crate::decode(&input).is_ok());
    }
}
