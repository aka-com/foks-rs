//! One original-request reservation, reused only after each physical page exits.
use super::{PageOutcome, PAGE_ROWS};
use foks_metadata_core::{
    owned_list::ListRetainedOwner, page::Cell, page_catalog::PageStatement, Error, Result,
};
use foks_snowpack::AllocationBudget;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc,
};

pub(super) const VERIFIER_BYTES: usize = 4 * (8 + 256 + 256);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct ScratchTerms {
    pub(in super::super) workspace_bytes: usize,
    pub(in super::super) control_bytes: usize,
    pub(in super::super) verifier_bytes: usize,
}

struct State {
    occupied: AtomicBool,
    failed: AtomicBool,
    generation: AtomicU64,
    charged: AtomicUsize,
    attempts: AtomicU64,
    traffic: AtomicU64,
    refusals: AtomicU64,
    first_refusal: AtomicU64,
}

struct Budget {
    parent: Arc<dyn ListRetainedOwner>,
    state: State,
    terms: ScratchTerms,
}

pub(in super::super) struct SchemaScratch {
    budget: Arc<Budget>,
}

pub(super) fn budget_arc_layout() -> Result<std::alloc::Layout> {
    // Pinned alloc::sync::ArcInner repr(C): two AtomicUsize fields then Budget.
    let counts = std::alloc::Layout::array::<AtomicUsize>(2).map_err(|_| Error::Bounds)?;
    let (layout, _) = counts
        .extend(std::alloc::Layout::new::<Budget>())
        .map_err(|_| Error::Bounds)?;
    Ok(layout.pad_to_align())
}

pub(in super::super) fn target_terms() -> Result<ScratchTerms> {
    let spec = PageStatement::SchemaObjects.spec();
    let row = spec.maximum_row_bytes()?;
    let page = 24usize
        .checked_add(
            usize::from(PAGE_ROWS)
                .checked_mul(row)
                .ok_or(Error::Bounds)?,
        )
        .ok_or(Error::Bounds)?;
    if row != 66160 || page != 529304 || spec.columns.len() != 12 || spec.parameters.len() != 4 {
        return Err(Error::Bounds);
    }
    // Exactly the existing schema parameter/NativeSql/PageEncoder charge sites.
    // Engine allocations and allocator rounding are separate qualification work.
    let workspace_bytes = 264
        + 4 * std::mem::size_of::<rusqlite::types::Value>()
        + 304
        + 12 * (512 + 9 * std::mem::size_of::<Cell<'static>>())
        + row
        + page;
    Ok(ScratchTerms {
        workspace_bytes,
        control_bytes: budget_arc_layout()?.size(),
        verifier_bytes: VERIFIER_BYTES,
    })
}

impl State {
    fn new() -> Self {
        Self {
            occupied: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            charged: AtomicUsize::new(0),
            attempts: AtomicU64::new(0),
            traffic: AtomicU64::new(0),
            refusals: AtomicU64::new(0),
            first_refusal: AtomicU64::new(0),
        }
    }
}

impl SchemaScratch {
    pub(in super::super) fn terms() -> Result<ScratchTerms> {
        target_terms()
    }

    pub(in super::super) fn new(parent: Arc<dyn ListRetainedOwner>) -> Result<Self> {
        let terms = target_terms().inspect_err(|_| parent.fail())?;
        let Some(total) = terms.workspace_bytes.checked_add(terms.control_bytes) else {
            parent.fail();
            return Err(Error::Bounds);
        };
        if parent.failed() || !parent.reserve_page_workspace(total) || parent.failed() {
            parent.fail();
            return Err(Error::Bounds);
        }
        Ok(Self {
            budget: Arc::new(Budget {
                parent,
                state: State::new(),
                terms,
            }),
        })
    }

    pub(super) fn reserve_verifier(&self) -> Result<()> {
        if self.budget.failed()
            || !self.budget.parent.reserve_retained(VERIFIER_BYTES)
            || self.budget.failed()
        {
            self.fail();
            return Err(Error::Bounds);
        }
        Ok(())
    }

    pub(in super::super) fn fail(&self) {
        self.budget.fail();
    }

    pub(super) fn run(&self, f: impl FnOnce() -> Result<PageOutcome>) -> Result<PageOutcome> {
        self.budget.run(f)
    }
}

impl Budget {
    fn failed(&self) -> bool {
        self.state.failed.load(Ordering::SeqCst) || self.parent.failed()
    }
    fn fail(&self) {
        self.state.failed.store(true, Ordering::SeqCst);
        self.parent.fail();
    }
    fn run(self: &Arc<Self>, f: impl FnOnce() -> Result<PageOutcome>) -> Result<PageOutcome> {
        if self.failed() {
            self.fail();
            return Err(Error::Bounds);
        }
        if self
            .state
            .occupied
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.fail();
            return Err(Error::Storage);
        }
        struct Scope<'a> {
            budget: &'a Budget,
            complete: bool,
        }
        impl Drop for Scope<'_> {
            fn drop(&mut self) {
                if !self.complete {
                    self.budget.fail();
                }
                self.budget.state.occupied.store(false, Ordering::SeqCst);
            }
        }
        let mut scope = Scope {
            budget: self,
            complete: false,
        };
        if self
            .state
            .generation
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
            .is_err()
        {
            return Err(Error::Bounds);
        }
        // Occupancy proves the previous lexical page/hook has exited. No sticky
        // failure, cumulative traffic, original charge or SQL credit is reset.
        self.state.charged.store(0, Ordering::SeqCst);
        let result = foks_snowpack::with_allocation_budget(self.clone(), f);
        if result.is_err() {
            self.fail();
        }
        let result = match result {
            Ok(_) if self.failed() => Err(Error::Bounds),
            other => other,
        };
        scope.complete = true;
        result
    }
}

impl AllocationBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        let ordinal = self
            .state
            .attempts
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1));
        let next = self.state.charged.load(Ordering::SeqCst).checked_add(bytes);
        let mut accepted = ordinal.is_ok()
            && !self.failed()
            && self.state.occupied.load(Ordering::SeqCst)
            && next.is_some_and(|n| n <= self.terms.workspace_bytes);
        let traffic = u64::try_from(bytes).ok().and_then(|bytes| {
            self.state
                .traffic
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(bytes))
                .ok()
        });
        accepted &= traffic.is_some() && !self.failed();
        if accepted {
            self.state
                .charged
                .store(next.expect("accepted checked sum"), Ordering::SeqCst);
            true
        } else {
            let _ = self
                .state
                .refusals
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1));
            if let Ok(previous) = ordinal {
                let _ = self.state.first_refusal.compare_exchange(
                    0,
                    previous + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            }
            self.fail();
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Parent;
    use super::*;

    fn empty() -> PageOutcome {
        PageOutcome {
            last: None,
            objects: 0,
            ddl_bytes: 0,
        }
    }

    struct Ambient(AtomicUsize);
    impl AllocationBudget for Ambient {
        fn reserve(&self, bytes: usize) -> bool {
            self.0.fetch_add(bytes, Ordering::SeqCst);
            true
        }
    }

    #[test]
    fn one_original_reservation_survives_reuse_and_retains_its_parent() {
        let parent = Arc::new(Parent::default());
        let weak = Arc::downgrade(&parent);
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let terms = SchemaScratch::terms().unwrap();
        assert_eq!(
            parent.held.load(Ordering::SeqCst),
            terms.workspace_bytes + terms.control_bytes
        );
        assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
        for _ in 0..3 {
            scratch
                .run(|| {
                    foks_snowpack::reserve_allocation(terms.workspace_bytes).unwrap();
                    assert!(parent.reserve_retained(17));
                    Ok(empty())
                })
                .unwrap();
            assert!(!scratch.budget.state.occupied.load(Ordering::SeqCst));
        }
        assert_eq!(scratch.budget.state.generation.load(Ordering::SeqCst), 3);
        assert_eq!(scratch.budget.state.attempts.load(Ordering::SeqCst), 3);
        assert_eq!(
            scratch.budget.state.traffic.load(Ordering::SeqCst),
            (3 * terms.workspace_bytes) as u64
        );
        assert_eq!(
            parent.held.load(Ordering::SeqCst),
            terms.workspace_bytes + terms.control_bytes
        );
        assert_eq!(parent.retained.load(Ordering::SeqCst), 51);
        assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
        drop(parent);
        assert!(weak.upgrade().is_some());
        drop(scratch);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn hook_restores_after_normal_error_and_unwind_exit() {
        for outcome in 0..3 {
            let parent = Arc::new(Parent::default());
            let scratch = SchemaScratch::new(parent.clone()).unwrap();
            let ambient = Arc::new(Ambient(AtomicUsize::new(0)));
            foks_snowpack::with_allocation_budget(ambient.clone(), || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    scratch.run(|| {
                        foks_snowpack::reserve_allocation(7).unwrap();
                        match outcome {
                            0 => Ok(empty()),
                            1 => Err(Error::Storage),
                            _ => panic!("schema page unwind"),
                        }
                    })
                }));
                match outcome {
                    0 => assert_eq!(result.unwrap(), Ok(empty())),
                    1 => assert_eq!(result.unwrap(), Err(Error::Storage)),
                    _ => assert!(result.is_err()),
                }
                assert!(!scratch.budget.state.occupied.load(Ordering::SeqCst));
                foks_snowpack::reserve_allocation(9).unwrap();
            });
            assert_eq!(ambient.0.load(Ordering::SeqCst), 9);
            assert_eq!(parent.failed(), outcome != 0);
            assert_eq!(scratch.budget.state.traffic.load(Ordering::SeqCst), 7);
        }
    }

    #[test]
    fn refused_page_cannot_report_success_or_reuse_failed_credits() {
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let result = scratch.run(|| {
            assert!(scratch.budget.reserve(scratch.budget.terms.workspace_bytes));
            assert!(!scratch.budget.reserve(1));
            Ok(empty())
        });
        assert_eq!(result, Err(Error::Bounds));
        assert!(parent.failed());
        assert_eq!(scratch.budget.state.first_refusal.load(Ordering::SeqCst), 2);
        assert_eq!(
            scratch.run(|| panic!("failed page must not enter")),
            Err(Error::Bounds)
        );
        assert_eq!(scratch.budget.state.generation.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reentry_and_all_monotonic_counter_overflows_fail_closed() {
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        assert_eq!(
            scratch.run(|| {
                assert_eq!(
                    scratch.run(|| panic!("nested schema page")),
                    Err(Error::Storage)
                );
                Ok(empty())
            }),
            Err(Error::Bounds)
        );
        assert!(parent.failed());
        for which in 0..3 {
            let parent = Arc::new(Parent::default());
            let scratch = SchemaScratch::new(parent.clone()).unwrap();
            match which {
                0 => scratch
                    .budget
                    .state
                    .generation
                    .store(u64::MAX, Ordering::SeqCst),
                1 => scratch
                    .budget
                    .state
                    .attempts
                    .store(u64::MAX, Ordering::SeqCst),
                _ => scratch
                    .budget
                    .state
                    .traffic
                    .store(u64::MAX, Ordering::SeqCst),
            }
            assert_eq!(
                scratch.run(|| {
                    let _ = scratch.budget.reserve(1);
                    Ok(empty())
                }),
                Err(Error::Bounds)
            );
            assert!(parent.failed());
            assert!(!scratch.budget.state.occupied.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn initial_refusal_allocates_no_scratch_and_prevents_any_schema_call() {
        let parent = Arc::new(Parent::default());
        parent.refuse_workspace.store(true, Ordering::SeqCst);
        assert!(matches!(
            SchemaScratch::new(parent.clone()),
            Err(Error::Bounds)
        ));
        assert_eq!(parent.held.load(Ordering::SeqCst), 0);
        assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
        assert!(parent.failed());
        // Construction owns no connection and therefore cannot prepare a page
        // before this refusal; the production caller cannot obtain a scratch.
    }

    #[test]
    fn exact_target_formula_and_std_growth_source_pins_are_retained() {
        let terms = SchemaScratch::terms().unwrap();
        assert_eq!(
            terms.workspace_bytes,
            264 + 4 * std::mem::size_of::<rusqlite::types::Value>()
                + 304
                + 12 * (512 + 9 * std::mem::size_of::<Cell<'static>>())
                + 66160
                + 529304
        );
        assert_eq!(terms.verifier_bytes, 2080);
        #[repr(C)]
        struct ArcInner {
            strong: AtomicUsize,
            weak: AtomicUsize,
            value: Budget,
        }
        assert_eq!(terms.control_bytes, std::mem::size_of::<ArcInner>());
        assert!(include_str!("../../../../../rust-toolchain.toml").contains("channel = \"1.95.0\""));
        // The independent integration allocator test covers the pure verifier's
        // old/new String realloc traffic under this pinned standard library.
        // Scratch's 2080-byte retained credit is never spendable by page hooks.
    }
}
