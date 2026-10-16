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
    assert!(page_response_allowed());
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
