use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};

#[derive(Default)]
struct Parent {
    held: AtomicUsize,
    reservations: AtomicUsize,
    retained: AtomicUsize,
    failed: AtomicBool,
    refuse_workspace: AtomicBool,
}
impl ListRetainedOwner for Parent {
    fn reserve_page_workspace(&self, bytes: usize) -> bool {
        self.reservations.fetch_add(1, Ordering::SeqCst);
        if self.refuse_workspace.load(Ordering::SeqCst) {
            return false;
        }
        self.held.fetch_add(bytes, Ordering::SeqCst);
        true
    }
    fn reserve_retained(&self, bytes: usize) -> bool {
        self.retained.fetch_add(bytes, Ordering::SeqCst);
        true
    }
    fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
    fn fail(&self) {
        self.failed.store(true, Ordering::SeqCst);
    }
}
fn progress() -> PageProgress {
    PageProgress {
        added: 0,
        last: None,
        exhausted: true,
    }
}
#[test]
fn one_held_reservation_reuses_only_local_credits_and_retains_parent() {
    let parent = Arc::new(Parent::default());
    let weak = Arc::downgrade(&parent);
    let budget = Budget::new(parent.clone()).unwrap();
    let total = budget.terms.workspace_bytes + budget.terms.control_bytes;
    assert_eq!(parent.held.load(Ordering::SeqCst), total);
    assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
    for _ in 0..3 {
        budget
            .run(|| {
                foks_snowpack::reserve_allocation(budget.terms.dynamic_credits).unwrap();
                assert!(parent.reserve_retained(17));
                Ok(progress())
            })
            .unwrap();
        assert!(!budget.state.occupied.load(Ordering::SeqCst));
    }
    assert_eq!(budget.state.generation.load(Ordering::SeqCst), 3);
    assert_eq!(budget.state.attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        budget.state.traffic.load(Ordering::SeqCst),
        (3 * budget.terms.dynamic_credits) as u64
    );
    assert_eq!(parent.held.load(Ordering::SeqCst), total);
    assert_eq!(parent.retained.load(Ordering::SeqCst), 51);
    assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
    drop(parent);
    assert!(weak.upgrade().is_some());
    drop(budget);
    assert!(weak.upgrade().is_none());
}
#[test]
fn refusal_cannot_spend_auxiliary_or_reenter_and_hook_restores_before_release() {
    struct Ambient(AtomicUsize);
    impl AllocationBudget for Ambient {
        fn reserve(&self, bytes: usize) -> bool {
            self.0.fetch_add(bytes, Ordering::SeqCst);
            true
        }
    }
    let ambient = Arc::new(Ambient(AtomicUsize::new(0)));
    let parent = Arc::new(Parent::default());
    let budget = Budget::new(parent.clone()).unwrap();
    foks_snowpack::with_allocation_budget(ambient.clone(), || {
        let result = budget.run(|| {
            assert!(budget.reserve(budget.terms.dynamic_credits));
            assert!(!budget.reserve(1));
            // Return a successful Copy progress deliberately: the sticky refusal
            // must still prevent the caller from treating this page as success.
            Ok(progress())
        });
        assert!(matches!(result, Err(PageFault::Bounds)));
        assert!(foks_snowpack::reserve_allocation(9).is_ok());
    });
    assert_eq!(ambient.0.load(Ordering::SeqCst), 9);
    assert!(parent.failed());
    assert_eq!(budget.state.first_refusal.load(Ordering::SeqCst), 2);
    assert!(matches!(
        budget.run(|| panic!("must not enter")),
        Err(PageFault::Bounds)
    ));
    assert_eq!(budget.state.generation.load(Ordering::SeqCst), 1);
}
#[test]
fn nested_entry_unwind_and_counter_overflow_latch_without_reset() {
    let parent = Arc::new(Parent::default());
    let budget = Budget::new(parent.clone()).unwrap();
    let result = budget.run(|| {
        assert!(matches!(
            budget.run(|| panic!("nested")),
            Err(PageFault::Storage)
        ));
        Ok(progress())
    });
    assert!(matches!(result, Err(PageFault::Bounds)));
    assert!(parent.failed());
    let parent = Arc::new(Parent::default());
    let budget = Budget::new(parent.clone()).unwrap();
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = budget.run(|| panic!("page unwind"));
    }));
    assert!(unwind.is_err());
    assert!(parent.failed());
    assert!(!budget.state.occupied.load(Ordering::SeqCst));
    for counter in [0, 1, 2] {
        let parent = Arc::new(Parent::default());
        let budget = Budget::new(parent.clone()).unwrap();
        match counter {
            0 => budget.state.generation.store(u64::MAX, Ordering::SeqCst),
            1 => budget.state.attempts.store(u64::MAX, Ordering::SeqCst),
            _ => budget.state.traffic.store(u64::MAX, Ordering::SeqCst),
        }
        let result = budget.run(|| {
            let _ = budget.reserve(1);
            Ok(progress())
        });
        assert!(matches!(result, Err(PageFault::Bounds)));
        assert!(parent.failed());
    }
}
#[test]
fn workspace_refusal_and_earlier_domain_error_preserve_provenance() {
    let parent = Arc::new(Parent::default());
    parent.refuse_workspace.store(true, Ordering::SeqCst);
    assert!(matches!(Budget::new(parent.clone()), Err(Error::Bounds)));
    assert!(parent.failed());
    assert_eq!(parent.held.load(Ordering::SeqCst), 0);
    let parent = Arc::new(Parent::default());
    let budget = Budget::new(parent.clone()).unwrap();
    assert!(matches!(
        budget.run(|| Err(PageFault::Corrupt)),
        Err(PageFault::Corrupt)
    ));
    assert!(parent.failed());
    assert!(matches!(
        budget.run(|| panic!("failed page")),
        Err(PageFault::Bounds)
    ));
}
#[test]
fn checked_terms_match_valid_schema_formula_and_pinned_layout_assumptions() {
    let terms = target_terms().unwrap();
    assert_eq!(
        terms.workspace_bytes,
        terms.dynamic_credits + terms.auxiliary_live
    );
    assert_eq!(
        terms.auxiliary_live,
        193 * std::mem::size_of::<foks_snowpack::PathSegment>() + 4062
    );
    assert!(
        terms.control_bytes
            >= std::mem::size_of::<Budget>() + 2 * std::mem::size_of::<AtomicUsize>()
    );
    assert!(include_str!("../../../../../../rust-toolchain.toml").contains("channel = \"1.95.0\""));
    #[cfg(feature = "list-allocation-qualification")]
    {
        let previous = crate::allocation_trace::formula::bounds().unwrap();
        assert_eq!(terms.dynamic_credits as u64, previous.full_page_scratch);
        assert_eq!(
            terms.parameter_copies + terms.list_transport + terms.decoded_container,
            previous.page_transport as usize
        );
        assert_eq!(terms.three_role_batches as u64, previous.page_roles);
    }
    let mut l = layout();
    l.path = 1;
    assert!(matches!(terms_for_test(l), Err(Error::Bounds)));
}
fn terms_for_test(l: Layout) -> Result<PageAllowanceTerms> {
    super::terms(l)
}

#[test]
fn fallible_local_capacity_failure_latches_without_a_hook_refusal() {
    let parent = Arc::new(Parent::default());
    let budget = Budget::new(parent.clone()).unwrap();
    let result = budget.run(|| {
        let mut bytes = Vec::<u8>::new();
        // Actual fallible capacity error, deliberately not allocator exhaustion.
        bytes
            .try_reserve_exact(usize::MAX)
            .map_err(|_| PageFault::Bounds)?;
        Ok(progress())
    });
    assert!(matches!(result, Err(PageFault::Bounds)));
    assert!(parent.failed());
    assert_eq!(budget.state.attempts.load(Ordering::SeqCst), 0);
    assert!(matches!(
        budget.run(|| panic!("failed local reserve")),
        Err(PageFault::Bounds)
    ));
}
mod allocations;

#[test]
fn snowpack_allocation_sources_remain_the_audited_bytes() {
    use sha2::{Digest, Sha256};
    for (source, expected) in [
        (
            include_bytes!("../../../../../foks-snowpack/src/decode.rs").as_slice(),
            "842057630a9c2be7eef7c234c5293ea36260c6dd77c35460f7914ace7c566020",
        ),
        (
            include_bytes!("../../../../../foks-snowpack/src/encode.rs").as_slice(),
            "d85781adfdfdd2fb640181a4fb94928ff135f42c7a35922aa53f3243c38ed094",
        ),
        (
            include_bytes!("../../../../../foks-snowpack/src/error.rs").as_slice(),
            "081596e37b4dfbd83e8cb20fa5b7cee821200776db7eeadeb2df5f1387478489",
        ),
        (
            include_bytes!("../../../../../foks-snowpack/src/allocation.rs").as_slice(),
            "5d7d2240f8e1b46bcf4dd8e790926b676e59f7b8c0b95f88914e7d99d5097ab4",
        ),
    ] {
        let digest: [u8; 32] = Sha256::digest(source).into();
        let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            actual, expected,
            "re-audit W2 path/error/charge ownership before updating pins"
        );
    }
}
