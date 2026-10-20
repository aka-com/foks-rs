use super::*;
use crate::page_catalog::PageStatement;
use std::sync::{Arc, Mutex};

struct NoAllocation(Mutex<usize>);

#[test]
fn native_wrapper_layout_includes_the_borrowed_counter_discriminant() {
    use std::mem::{align_of, size_of};
    // The mutable budget is borrowed, not embedded or allocated here. Counter
    // needs a full pointer-sized discriminant beside Local(usize)/Borrowed(ptr).
    // NativeSql adds its connection pointer. These are stack/control terms, not
    // additional heap allocations or the size of SQLite's connection itself.
    let word = size_of::<usize>();
    assert_eq!(size_of::<Counter<'static>>(), 2 * word);
    assert_eq!(align_of::<Counter<'static>>(), align_of::<usize>());
    assert_eq!(size_of::<NativeSql<'static>>(), 3 * word);
    assert_eq!(align_of::<NativeSql<'static>>(), align_of::<usize>());
    assert_eq!(size_of::<SqlCallBudget>(), 6);
    assert_eq!(align_of::<SqlCallBudget>(), 2);
}

impl foks_snowpack::AllocationBudget for NoAllocation {
    fn reserve(&self, _bytes: usize) -> bool {
        *self.0.lock().unwrap() += 1;
        false
    }
}

#[test]
fn borrowed_budget_counts_invalid_scalar_and_page_attempts_before_allocation() {
    let connection = Connection::open_in_memory().unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let mut budget = SqlCallBudget::new();
    let allocation = Arc::new(NoAllocation(Mutex::new(0)));
    foks_snowpack::with_allocation_budget(allocation.clone(), || {
        let mut sql = NativeSql::new_with_budget(&transaction, &mut budget).unwrap();
        assert_eq!(
            sql.execute(Statement::AuthorityClock, &[Value::Null]),
            Err(Error::Bounds)
        );
        assert_eq!(sql.calls(), 1);
        assert_eq!(
            sql.query(Statement::AuthorityClock, &[Value::Integer(1)]),
            Err(Error::Bounds)
        );
        assert_eq!(sql.calls(), 2);
        assert!(matches!(
            sql.query_page(PageStatement::ProbeTypes, &[]),
            Err(Error::Bounds)
        ));
        assert_eq!(sql.calls(), 3);
    });
    assert_eq!(*allocation.0.lock().unwrap(), 0);
    assert_eq!((budget.charged(), budget.attempts()), (3, 3));
}

#[test]
fn new_adapters_and_transactions_cannot_reset_the_original_budget() {
    let connection = Connection::open_in_memory().unwrap();
    let mut budget = SqlCallBudget::new();
    for interval in 1..=2 {
        budget.begin_cleanup_reservation().unwrap();
        budget.attempt().unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        {
            let mut sql = NativeSql::new_with_budget(&transaction, &mut budget).unwrap();
            assert_eq!(
                sql.query(Statement::ReadSchemaCookie, &[]).unwrap(),
                Some(vec![Value::Integer(0)])
            );
            assert_eq!(sql.calls(), (interval - 1) * 3 + 2);
        }
        budget.attempt().unwrap();
        transaction.commit().unwrap();
        budget.finish_cleanup_reservation().unwrap();
        assert_eq!(
            (budget.charged(), budget.attempts()),
            (interval * 4, interval * 3)
        );
    }
}

#[test]
fn prepare_failure_is_charged_and_cannot_be_hidden_by_replacing_the_adapter() {
    let connection = Connection::open_in_memory().unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let mut budget = SqlCallBudget::new();
    for attempts in 1..=2 {
        let mut sql = NativeSql::new_with_budget(&transaction, &mut budget).unwrap();
        // This connection has no authority table. The failure reaches prepare.
        assert_eq!(
            sql.query(Statement::AuthorityState, &[]),
            Err(Error::Storage)
        );
        assert_eq!(sql.calls(), attempts);
    }
    assert_eq!(budget.charged(), 2);
}

#[test]
fn exhausted_borrowed_host_refuses_sql_and_preserves_cleanup() {
    let connection = Connection::open_in_memory().unwrap();
    let mut budget = SqlCallBudget::new();
    budget.begin_cleanup_reservation().unwrap();
    budget.attempt().unwrap();
    connection.execute_batch("BEGIN").unwrap();
    for _ in 2..MAX_SQL_CALLS {
        budget.attempt().unwrap();
    }
    let allocation = Arc::new(NoAllocation(Mutex::new(0)));
    foks_snowpack::with_allocation_budget(allocation.clone(), || {
        let mut sql = NativeSql::new_with_budget(&connection, &mut budget).unwrap();
        // Invalid parameters would also fail, but exhaustion must latch first.
        assert_eq!(
            sql.query(Statement::AuthorityState, &[Value::Null]),
            Err(Error::Bounds)
        );
        assert!(matches!(
            sql.query_page(PageStatement::ProbeTypes, &[]),
            Err(Error::Bounds)
        ));
    });
    assert_eq!(*allocation.0.lock().unwrap(), 0);
    assert!(budget.failed());
    assert!(matches!(
        NativeSql::new_with_budget(&connection, &mut budget),
        Err(Error::Bounds)
    ));
    budget.attempt_rollback().unwrap();
    connection.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        (budget.charged(), budget.attempts()),
        (MAX_SQL_CALLS, MAX_SQL_CALLS)
    );
    assert!(connection.is_autocommit());
}

#[test]
fn failed_adapter_construction_executes_and_charges_nothing() {
    let connection = Connection::open_in_memory().unwrap();
    let mut budget = SqlCallBudget::new();
    assert!(matches!(
        NativeSql::new_with_budget(&connection, &mut budget),
        Err(Error::Storage)
    ));
    assert_eq!((budget.charged(), budget.attempts()), (0, 0));
    connection.execute_batch("BEGIN").unwrap();
    budget.fail();
    assert!(matches!(
        NativeSql::new_with_budget(&connection, &mut budget),
        Err(Error::Bounds)
    ));
    assert_eq!((budget.charged(), budget.attempts()), (0, 0));
    connection.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn local_page_validation_keeps_its_existing_count_order() {
    let connection = Connection::open_in_memory().unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&transaction).unwrap();
    assert!(matches!(
        sql.query_page(PageStatement::ProbeTypes, &[]),
        Err(Error::Bounds)
    ));
    assert_eq!(sql.calls(), 0);
}
