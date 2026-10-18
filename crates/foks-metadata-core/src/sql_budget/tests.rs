use super::*;

#[test]
fn successful_intervals_keep_unused_cleanup_charged() {
    let mut budget = SqlCallBudget::new();
    for interval in 1..=2 {
        budget.begin_cleanup_reservation().unwrap();
        // BEGIN, three bounded reads, and COMMIT.
        for _ in 0..5 {
            budget.attempt().unwrap();
        }
        budget.finish_cleanup_reservation().unwrap();
        assert_eq!(budget.charged(), interval * 6);
        assert_eq!(budget.attempts(), interval * 5);
        assert!(!budget.failed());
    }
}

#[test]
fn exhaustion_keeps_exactly_one_prepaid_rollback_available() {
    let mut budget = SqlCallBudget::new();
    budget.begin_cleanup_reservation().unwrap();
    for _ in 1..MAX_SQL_CALLS {
        budget.attempt().unwrap();
    }
    assert_eq!(budget.charged(), MAX_SQL_CALLS);
    assert_eq!(budget.attempts(), MAX_SQL_CALLS - 1);
    for _ in 0..8 {
        assert_eq!(budget.attempt(), Err(Error::Bounds));
    }
    assert!(budget.failed());
    assert_eq!(budget.attempts(), MAX_SQL_CALLS - 1);
    budget.attempt_rollback().unwrap();
    assert_eq!(budget.charged(), MAX_SQL_CALLS);
    assert_eq!(budget.attempts(), MAX_SQL_CALLS);
    assert_eq!(budget.attempt_rollback(), Err(Error::Bounds));
    assert_eq!(budget.finish_cleanup_reservation(), Err(Error::Bounds));
    assert_eq!(budget.begin_cleanup_reservation(), Err(Error::Bounds));
    assert_eq!(budget.attempts(), MAX_SQL_CALLS);
    assert!(budget.failed());
}

#[test]
fn nested_cleanup_misuse_fails_without_consuming_the_original_credit() {
    let mut budget = SqlCallBudget::new();
    budget.begin_cleanup_reservation().unwrap();
    assert_eq!(budget.begin_cleanup_reservation(), Err(Error::Bounds));
    assert_eq!(budget.charged(), 1);
    assert_eq!(budget.attempt(), Err(Error::Bounds));
    budget.attempt_rollback().unwrap();
    assert_eq!(budget.attempts(), 1);
    assert!(budget.failed());
}

#[test]
fn failed_begin_disarms_cleanup_without_refund_or_clearing_failure() {
    let mut budget = SqlCallBudget::new();
    budget.begin_cleanup_reservation().unwrap();
    budget.attempt().unwrap();
    budget.fail();
    budget.finish_cleanup_reservation().unwrap();
    assert_eq!((budget.charged(), budget.attempts()), (2, 1));
    assert_eq!(budget.attempt_rollback(), Err(Error::Bounds));
    assert_eq!(budget.attempt(), Err(Error::Bounds));
    assert!(budget.failed());
}

#[test]
fn exhausted_or_missing_cleanup_cannot_create_an_unpaid_call() {
    let mut budget = SqlCallBudget::default();
    for _ in 0..MAX_SQL_CALLS {
        budget.attempt().unwrap();
    }
    assert_eq!(budget.begin_cleanup_reservation(), Err(Error::Bounds));
    assert_eq!(budget.attempt_rollback(), Err(Error::Bounds));
    assert_eq!(
        (budget.charged(), budget.attempts()),
        (MAX_SQL_CALLS, MAX_SQL_CALLS)
    );
    let mut empty = SqlCallBudget::new();
    assert_eq!(empty.finish_cleanup_reservation(), Err(Error::Bounds));
    assert_eq!(empty.attempt_rollback(), Err(Error::Bounds));
    assert_eq!((empty.charged(), empty.attempts()), (0, 0));
    assert!(empty.failed());
}

#[test]
fn shared_counter_has_the_reviewed_fixed_layout() {
    assert_eq!(std::mem::size_of::<SqlCallBudget>(), 6);
    assert_eq!(std::mem::align_of::<SqlCallBudget>(), 2);
}
