use super::*;
use std::sync::Arc;

fn clean() {
    STATE.with(|state| *state.borrow_mut() = State::new());
}
fn retained() -> (usize, usize, u8, Phase) {
    STATE.with(|state| {
        let s = state.borrow();
        (
            s.buffer.as_ref().map_or(0, Vec::len),
            s.buffer.as_ref().map_or(0, Vec::capacity),
            s.depth,
            s.phase,
        )
    })
}
struct Refuse;
impl foks_snowpack::AllocationBudget for Refuse {
    fn reserve(&self, _: usize) -> bool {
        false
    }
}

#[test]
fn large_small_and_error_imports_release_all_response_capacity() {
    clean();
    for length in [MAX_BYTES, 3, MAX_BYTES, 0, 1] {
        let invocation = Invocation::begin(false).unwrap();
        let scope = Scope::begin(Phase::Scalar).unwrap();
        assert!(allocate(length).is_some());
        assert_eq!(retained().0, length);
        let bytes = scope.finish(false).unwrap();
        assert_eq!(bytes, vec![0; length]);
        assert_eq!(retained(), (0, 0, 1, Phase::Idle));
        drop(bytes);
        drop(invocation);
        assert_eq!(retained(), (0, 0, 0, Phase::Idle));
    }
    let invocation = Invocation::begin(false).unwrap();
    let scope = Scope::begin(Phase::Scalar).unwrap();
    assert!(allocate(MAX_BYTES).is_some());
    assert_eq!(scope.finish(true), Err(Error::Storage));
    assert_eq!(retained(), (0, 0, 1, Phase::Idle));
    assert_eq!(checked(Ok(())), Err(Error::Storage));
    drop(invocation);
    assert_eq!(retained(), (0, 0, 0, Phase::Idle));
}

#[test]
fn duplicate_missing_out_of_phase_and_oversize_allocation_fail_closed() {
    for case in 0..4 {
        clean();
        let invocation = Invocation::begin(false).unwrap();
        if case == 0 {
            assert!(allocate(1).is_none());
            assert_eq!(check(), Err(Error::Storage));
        } else {
            let scope = Scope::begin(Phase::Scalar).unwrap();
            let expected = if case == 3 {
                Error::Bounds
            } else {
                Error::Storage
            };
            if case == 1 {
                assert!(allocate(1).is_some());
                assert!(allocate(1).is_none());
            }
            if case == 3 {
                assert!(allocate(MAX_BYTES + 1).is_none());
            }
            assert_eq!(scope.finish(false), Err(expected.clone()));
            assert_eq!(check(), Err(expected));
        }
        drop(invocation);
        assert_eq!(retained(), (0, 0, 0, Phase::Idle));
    }
}

#[test]
fn scalar_failure_cannot_be_ignored_by_enclosing_domain_or_publication() {
    clean();
    let invocation = Invocation::begin(false).unwrap();
    let domain = Scope::begin(Phase::Domain).unwrap();
    {
        let _nested = Invocation::begin(true).unwrap();
        let scalar = Scope::begin(Phase::Scalar).unwrap();
        assert!(allocate(1).is_some());
        assert!(allocate(2).is_none());
        assert_eq!(scalar.finish(false), Err(Error::Storage));
        assert_eq!(checked(Ok("ignored nested error")), Err(Error::Storage));
    }
    assert!(allocate(1).is_none());
    assert_eq!(domain.finish(false), Err(Error::Storage));
    assert_eq!(checked(Ok("forged outer success")), Err(Error::Storage));
    drop(invocation);
    // A normal error unwound all physical ownership; a new invocation can begin.
    let _next = Invocation::begin(false).unwrap();
    assert_eq!(check(), Ok(()));
}

#[test]
fn normal_nested_domain_restores_scope_without_retained_buffers() {
    clean();
    let _invocation = Invocation::begin(false).unwrap();
    let domain = Scope::begin(Phase::Domain).unwrap();
    {
        let _nested = Invocation::begin(true).unwrap();
        for length in [1, 20, 4096] {
            let scope = Scope::begin(Phase::Scalar).unwrap();
            assert!(allocate(length).is_some());
            assert_eq!(scope.finish(false).unwrap().len(), length);
            assert_eq!(retained(), (0, 0, 2, Phase::Domain));
        }
    }
    assert!(allocate(7).is_some());
    assert_eq!(domain.finish(false).unwrap(), vec![0; 7]);
    assert_eq!(retained(), (0, 0, 1, Phase::Idle));
}

#[test]
fn allocation_refusal_and_forbidden_reentry_stay_sticky() {
    clean();
    let invocation = Invocation::begin(false).unwrap();
    let scope = Scope::begin(Phase::Scalar).unwrap();
    assert!(foks_snowpack::with_allocation_budget(Arc::new(Refuse), || allocate(12)).is_none());
    assert_eq!(scope.finish(false), Err(Error::Bounds));
    assert_eq!(checked(Ok(())), Err(Error::Bounds));
    drop(invocation);
    let _next = Invocation::begin(false).unwrap();
    let scope = Scope::begin(Phase::Scalar).unwrap();
    assert!(Invocation::begin(true).is_err());
    assert!(allocate(12).is_none());
    assert_eq!(scope.finish(false), Err(Error::Storage));
}

#[test]
fn unwinding_drops_owned_buffer_and_epoch_never_wraps() {
    clean();
    let caught = std::panic::catch_unwind(|| {
        let _invocation = Invocation::begin(false).unwrap();
        let _scope = Scope::begin(Phase::Scalar).unwrap();
        assert!(allocate(4096).is_some());
        panic!("native unwind only; Wasm traps require instance disposal");
    });
    assert!(caught.is_err());
    assert_eq!(retained(), (0, 0, 0, Phase::Idle));
    STATE.with(|state| state.borrow_mut().epoch = u64::MAX);
    assert!(matches!(Invocation::begin(false), Err(Error::Bounds)));
}

#[test]
fn pure_shared_buffer_helpers_refuse_active_sql_and_domain_allows_only_nested_buffers() {
    clean();
    assert!(idle());
    clear_idle();
    let invocation = Invocation::begin(false).unwrap();
    assert_eq!(retained(), (0, 0, 1, Phase::Idle));
    let scope = Scope::begin(Phase::Scalar).unwrap();
    assert!(allocate(7).is_some());
    assert!(!input_output_allowed());
    clear_idle();
    assert_eq!(retained().0, 7); // rejected clear did not alter the live buffer
    assert_eq!(scope.finish(false), Err(Error::Storage));
    drop(invocation);
    let _next = Invocation::begin(false).unwrap();
    let domain = Scope::begin(Phase::Domain).unwrap();
    assert!(input_output_allowed());
    assert!(!idle());
    assert_eq!(domain.finish(false), Err(Error::Storage));
}

#[test]
fn page_delivery_transfers_capacity_and_error_paths_drop_it() {
    use crate::page::{EnvelopeKind, PageBuffer, PageEncoder, MAX_PAGE_BYTES};
    let spec = crate::page_catalog::PageStatement::ProbeTypes.spec();
    let exact = PageEncoder::new(spec, 1)
        .unwrap()
        .finish()
        .unwrap()
        .into_bytes();
    clean();
    for length in [MAX_PAGE_BYTES, 24, MAX_PAGE_BYTES] {
        let invocation = Invocation::begin(false).unwrap();
        let scope = Scope::begin(Phase::Page).unwrap();
        assert!(allocate_page(length).is_some());
        assert_eq!(retained().0, length);
        assert!(retained().1 >= length);
        if length == 24 {
            STATE.with(|state| {
                state
                    .borrow_mut()
                    .buffer
                    .as_mut()
                    .unwrap()
                    .copy_from_slice(&exact)
            });
            let bytes = scope.finish(false).unwrap();
            let pointer = bytes.as_ptr();
            let capacity = bytes.capacity();
            assert_eq!(retained(), (0, 0, 1, Phase::Idle));
            // Empty TLS is not a released allocation: PageBuffer still owns it.
            let page = PageBuffer::decode(bytes, &spec, EnvelopeKind::Rows).unwrap();
            assert_eq!(page.as_bytes().as_ptr(), pointer);
            let bytes = page.into_bytes();
            assert_eq!(bytes.capacity(), capacity);
            drop(bytes); // Physical Vec owner ends only here.
        } else {
            assert_eq!(scope.finish(true), Err(Error::Storage));
            assert_eq!(retained(), (0, 0, 1, Phase::Idle));
        }
        drop(invocation);
        assert_eq!(retained(), (0, 0, 0, Phase::Idle));
    }
    let invocation = Invocation::begin(false).unwrap();
    let scope = Scope::begin(Phase::Page).unwrap();
    assert!(allocate_page(24).is_some());
    let bytes = scope.finish(false).unwrap(); // zero bytes are an invalid envelope
    assert!(PageBuffer::decode(bytes, &spec, EnvelopeKind::Rows).is_err());
    assert_eq!(retained(), (0, 0, 1, Phase::Idle));
    drop(invocation);
}

#[test]
fn page_wrong_allocator_missing_duplicate_and_length_failures_are_sticky() {
    for phase in [Phase::Idle, Phase::Scalar, Phase::Domain, Phase::Page] {
        clean();
        let invocation = Invocation::begin(false).unwrap();
        let scope = if phase == Phase::Idle {
            None
        } else {
            Some(Scope::begin(phase).unwrap())
        };
        let result = if phase == Phase::Page {
            allocate(24)
        } else {
            allocate_page(24)
        };
        assert!(result.is_none());
        assert_eq!(check(), Err(Error::Storage));
        if let Some(scope) = scope {
            assert_eq!(scope.finish(false), Err(Error::Storage));
        }
        drop(invocation);
        assert_eq!(retained(), (0, 0, 0, Phase::Idle));
    }
    for length in [0, 23, crate::page::MAX_PAGE_BYTES + 1] {
        clean();
        let invocation = Invocation::begin(false).unwrap();
        let scope = Scope::begin(Phase::Page).unwrap();
        assert!(allocate_page(length).is_none());
        assert_eq!(scope.finish(false), Err(Error::Bounds));
        assert_eq!(retained(), (0, 0, 1, Phase::Idle));
        drop(invocation);
    }
    for duplicate in [false, true] {
        clean();
        let invocation = Invocation::begin(false).unwrap();
        let scope = Scope::begin(Phase::Page).unwrap();
        if duplicate {
            assert!(allocate_page(24).is_some());
            assert!(allocate_page(24).is_none());
        }
        assert_eq!(scope.finish(false), Err(Error::Storage));
        drop(invocation);
        assert_eq!(retained(), (0, 0, 0, Phase::Idle));
    }
}

#[test]
fn page_refusal_reentrant_admission_and_nested_domain_obey_same_owner() {
    struct Reenter;
    impl foks_snowpack::AllocationBudget for Reenter {
        fn reserve(&self, _: usize) -> bool {
            assert!(allocate_page(24).is_none());
            true
        }
    }
    for reenter in [false, true] {
        clean();
        let invocation = Invocation::begin(false).unwrap();
        let scope = Scope::begin(Phase::Page).unwrap();
        let budget: Arc<dyn foks_snowpack::AllocationBudget> = if reenter {
            Arc::new(Reenter)
        } else {
            Arc::new(Refuse)
        };
        assert!(foks_snowpack::with_allocation_budget(budget, || allocate_page(24)).is_none());
        assert_eq!(
            scope.finish(false),
            Err(if reenter {
                Error::Storage
            } else {
                Error::Bounds
            })
        );
        drop(invocation);
        assert_eq!(retained(), (0, 0, 0, Phase::Idle));
    }
    for reject in [false, true] {
        clean();
        let invocation = Invocation::begin(false).unwrap();
        let domain = Scope::begin(Phase::Domain).unwrap();
        {
            let nested = Invocation::begin(true).unwrap();
            let page = Scope::begin(Phase::Page).unwrap();
            assert!(allocate_page(24).is_some());
            if reject {
                assert!(allocate(1).is_none());
                assert_eq!(page.finish(false), Err(Error::Storage));
            } else {
                drop(page.finish(false).unwrap());
                assert_eq!(retained(), (0, 0, 2, Phase::Domain));
            }
            drop(nested);
        }
        if reject {
            assert!(allocate(1).is_none());
            assert_eq!(domain.finish(false), Err(Error::Storage));
        } else {
            assert!(allocate(1).is_some());
            drop(domain.finish(false).unwrap());
        }
        drop(invocation);
        assert_eq!(retained(), (0, 0, 0, Phase::Idle));
    }
    clean();
    let invocation = Invocation::begin(false).unwrap();
    let domain = Scope::begin(Phase::Domain).unwrap();
    assert!(Scope::begin(Phase::Page).is_err()); // No nested Invocation at depth1 Domain.
    assert_eq!(domain.finish(false), Err(Error::Storage));
    drop(invocation);
}

#[test]
fn page_native_unwind_releases_capacity_without_claiming_wasm_trap_unwind() {
    clean();
    assert!(std::panic::catch_unwind(|| {
        let _invocation = Invocation::begin(false).unwrap();
        let _scope = Scope::begin(Phase::Page).unwrap();
        assert!(allocate_page(crate::page::MAX_PAGE_BYTES).is_some());
        panic!("native-only drop test");
    })
    .is_err());
    assert_eq!(retained(), (0, 0, 0, Phase::Idle));
}

#[test]
fn refused_admission_is_distinct_from_fallible_allocator_failure() {
    use std::cell::Cell;
    for kind in [DeliveryKind::Scalar, DeliveryKind::Page] {
        for refuse in [false, true] {
            clean();
            let invocation = Invocation::begin(false).unwrap();
            let phase = match kind {
                DeliveryKind::Scalar => Phase::Scalar,
                DeliveryKind::Page => Phase::Page,
            };
            let scope = Scope::begin(phase).unwrap();
            let reserved = Cell::new(false);
            let marked = Cell::new(false);
            let call = || {
                allocate_kind_with(
                    kind,
                    24,
                    |_, _| {
                        reserved.set(true);
                        Err(())
                    },
                    || marked.set(true),
                )
            };
            let result = if refuse {
                foks_snowpack::with_allocation_budget(Arc::new(Refuse), call)
            } else {
                call()
            };
            assert!(result.is_none());
            assert_eq!(reserved.get(), !refuse);
            assert_eq!(marked.get(), !refuse);
            assert_eq!(scope.finish(false), Err(Error::Bounds));
            drop(invocation);
            assert_eq!(retained(), (0, 0, 0, Phase::Idle));
        }
    }
}

#[test]
fn input_buffer_reservation_failure_is_a_refusal_not_an_abort() {
    assert_eq!(zeroed_input(16), Some(vec![0; 16]));
    assert_eq!(zeroed_input(0), Some(Vec::new()));
    // Capacity overflow: `vec![0; usize::MAX]` panics here, and in the Wasm
    // module an allocation failure aborts and traps the instance.
    assert_eq!(zeroed_input(usize::MAX), None);
    assert_eq!(zeroed_input_with(16, |_, _| Err(())), None);
}
