use super::*;
use crate::{native::NativeSql, Sql, Statement};
use foks_snowpack::{with_allocation_budget, AllocationBudget};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Budget {
    charges: Mutex<Vec<usize>>,
    reject: Option<usize>,
}
impl Budget {
    fn rejecting(index: usize) -> Arc<Self> {
        Arc::new(Self {
            reject: Some(index),
            ..Self::default()
        })
    }
    fn charges(&self) -> Vec<usize> {
        self.charges.lock().unwrap().clone()
    }
}
impl AllocationBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        let mut charges = self.charges.lock().unwrap();
        let index = charges.len();
        charges.push(bytes);
        self.reject != Some(index)
    }
}

fn database() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scalar.sqlite");
    drop(foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap());
    let connection = Connection::open(path).unwrap();
    connection.execute_batch(crate::authority::SCHEMA).unwrap();
    connection.execute("INSERT INTO metadata_authority(singleton,identity,activation,owner,epoch,revision,high_water,max_rows,max_bytes,clock_floor) VALUES(1,'identity','activation',?1,1,1,0,10,46080,1)", [vec![7u8; 16]]).unwrap();
    (directory, connection)
}

#[test]
fn stack_decimals_match_canonical_i64_and_bind_as_sql_text() {
    let connection = Connection::open_in_memory().unwrap();
    for n in [
        i64::MIN,
        -1000,
        -100,
        -10,
        -1,
        0,
        1,
        9,
        10,
        99,
        100,
        999,
        1000,
        i64::MAX,
    ] {
        assert_eq!(Decimal::new(n).bytes(), n.to_string().as_bytes());
        let budget = Arc::new(Budget::default());
        let values =
            with_allocation_budget(budget.clone(), || bindings(&[Value::Integer(n)])).unwrap();
        assert_eq!(
            budget.charges(),
            [size_of::<SqlValue>() + n.to_string().len()]
        );
        let (kind, actual): (String, String) = connection
            .query_row(
                "SELECT typeof(?1),?1",
                rusqlite::params_from_iter(values),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "text");
        assert_eq!(actual, n.to_string());
    }
}

#[test]
fn integer_and_cell_validation_preserve_error_categories_without_admission() {
    let budget = Budget::rejecting(0);
    with_allocation_budget(budget.clone(), || {
        for bytes in [b"+1".as_slice(), b"01", b"-0", b"00"] {
            assert!(matches!(
                borrowed(ValueRef::Text(bytes), Kind::Integer),
                Err(Error::Storage)
            ));
        }
        // Text that does not parse as an i64 is stored data at fault, like
        // non-canonical text, and the page decoder classifies it the same way.
        for bytes in [
            b"".as_slice(),
            b"1.0",
            b"9223372036854775808",
            b"-9223372036854775809",
            b"+1",
            b"01",
            &[0xff],
        ] {
            assert!(matches!(
                borrowed(ValueRef::Text(bytes), Kind::Integer),
                Err(Error::Storage)
            ));
            assert_eq!(integer_text(bytes), Err(Error::Storage));
        }
        for n in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(integer_text(n.to_string().as_bytes()), Ok(n));
        }
        assert!(matches!(
            borrowed(ValueRef::Text(&[0xff]), Kind::Integer),
            Err(Error::Storage)
        ));
        assert!(matches!(
            borrowed(ValueRef::Text(b"000000000000000000000"), Kind::Integer),
            Err(Error::Bounds)
        ));
        assert!(matches!(
            borrowed(ValueRef::Integer(1), Kind::Integer),
            Err(Error::Bounds)
        ));
        assert!(matches!(
            borrowed(ValueRef::Text(&[0xff]), Kind::Text),
            Err(Error::Storage)
        ));
        assert!(matches!(
            borrowed(ValueRef::Null, Kind::Blob),
            Err(Error::Bounds)
        ));
    });
    assert!(budget.charges().is_empty());
}

#[test]
fn probe_types_charges_binding_and_row_containers_and_payloads_once() {
    let (_directory, connection) = database();
    let transaction = connection.unchecked_transaction().unwrap();
    let parameters = [
        Value::Integer(i64::MIN),
        Value::Blob(vec![9; 4096]),
        Value::Null,
        Value::Text("é\0z".into()),
    ];
    let budget = Arc::new(Budget::default());
    let result = with_allocation_budget(budget.clone(), || {
        NativeSql::new(&transaction)
            .unwrap()
            .query(Statement::ProbeTypes, &parameters)
    })
    .unwrap()
    .unwrap();
    assert_eq!(result, parameters);
    assert_eq!(
        budget.charges(),
        [
            4 * size_of::<SqlValue>() + 20 + 4096 + 4,
            4 * size_of::<Value>() + 4096 + 4
        ]
    );
    transaction.rollback().unwrap();
}

#[test]
fn actual_activation_rows_have_exact_requested_charges_and_refuse_before_copy() {
    let (_directory, connection) = database();
    let transaction = connection.unchecked_transaction().unwrap();
    for statement in [
        Statement::ReadDependency,
        Statement::ReadSchemaCookie,
        Statement::AuthorityState,
    ] {
        let budget = Arc::new(Budget::default());
        let values = with_allocation_budget(budget.clone(), || {
            NativeSql::new(&transaction).unwrap().query(statement, &[])
        })
        .unwrap()
        .unwrap();
        let payload: usize = values
            .iter()
            .map(|v| match v {
                Value::Text(s) => s.len(),
                Value::Blob(b) => b.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(
            budget.charges(),
            [values.len() * size_of::<Value>() + payload]
        );
        let refuse = Budget::rejecting(0);
        assert_eq!(
            with_allocation_budget(refuse.clone(), || NativeSql::new(&transaction)
                .unwrap()
                .query(statement, &[])),
            Err(Error::Bounds)
        );
        assert_eq!(refuse.charges(), budget.charges());
    }
    transaction.rollback().unwrap();
}

#[test]
fn row_refusal_no_row_and_empty_buffers_keep_exact_admission() {
    let (_directory, connection) = database();
    let transaction = connection.unchecked_transaction().unwrap();
    let parameters = [
        Value::Integer(0),
        Value::Blob(vec![]),
        Value::Null,
        Value::Text(String::new()),
    ];
    let budget = Budget::rejecting(1);
    assert_eq!(
        with_allocation_budget(budget.clone(), || NativeSql::new(&transaction)
            .unwrap()
            .query(Statement::ProbeTypes, &parameters)),
        Err(Error::Bounds)
    );
    assert_eq!(
        budget.charges(),
        [4 * size_of::<SqlValue>() + 1, 4 * size_of::<Value>()]
    );
    let budget = Arc::new(Budget::default());
    assert_eq!(
        with_allocation_budget(budget.clone(), || NativeSql::new(&transaction)
            .unwrap()
            .query(
                Statement::PersonalCurrentCredential,
                &[
                    Value::Blob(vec![1; 33]),
                    Value::Blob(vec![2; 33]),
                    Value::Blob(vec![3; 33])
                ]
            )),
        Ok(None)
    );
    assert_eq!(budget.charges(), [3 * size_of::<SqlValue>() + 99]);
    let empty = Arc::new(Budget::default());
    assert!(with_allocation_budget(empty.clone(), || bindings(&[]))
        .unwrap()
        .is_empty());
    assert!(empty.charges().is_empty());
    transaction.rollback().unwrap();
}

#[test]
fn refused_binding_precedes_sql_prepare_and_authority_clock_write() {
    let (_directory, connection) = database();
    let transaction = connection.unchecked_transaction().unwrap();
    let refuse = Budget::rejecting(0);
    assert_eq!(
        with_allocation_budget(refuse.clone(), || NativeSql::new(&transaction)
            .unwrap()
            .execute(Statement::AuthorityClock, &[Value::Integer(999)])),
        Err(Error::Bounds)
    );
    assert_eq!(refuse.charges(), [size_of::<SqlValue>() + 3]);
    assert_eq!(
        transaction
            .query_row("SELECT clock_floor FROM metadata_authority", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
    transaction
        .execute_batch("DROP TABLE metadata_principals")
        .unwrap();
    let parameters = [Value::Blob(vec![1; 32])];
    assert_eq!(
        with_allocation_budget(Budget::rejecting(0), || NativeSql::new(&transaction)
            .unwrap()
            .query(Statement::AuthorityPrincipal, &parameters)),
        Err(Error::Bounds)
    );
    assert_eq!(
        NativeSql::new(&transaction)
            .unwrap()
            .query(Statement::AuthorityPrincipal, &parameters),
        Err(Error::Storage)
    );
    transaction.rollback().unwrap();
}

#[test]
fn parameter_mode_and_call_limit_fail_before_binding_charge() {
    let connection = Connection::open_in_memory().unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let mut host = NativeSql::new(&transaction).unwrap();
    let budget = Budget::rejecting(0);
    with_allocation_budget(budget.clone(), || {
        assert_eq!(
            host.execute(Statement::AuthorityClock, &[Value::Null]),
            Err(Error::Bounds)
        );
        assert_eq!(host.calls(), 0);
        assert_eq!(
            host.query(Statement::AuthorityClock, &[Value::Integer(1)]),
            Err(Error::Bounds)
        );
        assert_eq!(host.calls(), 1);
        host.calls = super::super::Counter::Local(crate::MAX_SQL_CALLS);
        assert_eq!(
            host.execute(Statement::AuthorityClock, &[Value::Integer(1)]),
            Err(Error::Bounds)
        );
    });
    assert!(budget.charges().is_empty());
}

#[test]
fn malformed_first_row_is_checked_before_row_allocation_and_extra_row_still_fails() {
    // Deliberately not a valid authority schema: exercise the host's rejection
    // boundary independently of the genuine-schema tests above.
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE metadata_authority(singleton,identity,activation,owner,epoch,revision,high_water,retained_rows,reserved_bytes,max_rows,max_bytes,clock_floor); INSERT INTO metadata_authority VALUES(1,'i','a',X'01',1,1,0,0,0,10,46080,0)").unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    for (sql, error) in [
        (
            "UPDATE metadata_authority SET identity=CAST(X'ff' AS TEXT)",
            Error::Storage,
        ),
        (
            "UPDATE metadata_authority SET identity=zeroblob(1)",
            Error::Bounds,
        ),
        (
            "UPDATE metadata_authority SET identity='i',owner=zeroblob(4097)",
            Error::Bounds,
        ),
        (
            "UPDATE metadata_authority SET identity='i',owner=X'01',epoch='01'",
            Error::Storage,
        ),
    ] {
        transaction.execute_batch(sql).unwrap();
        let budget = Budget::rejecting(0);
        assert_eq!(
            with_allocation_budget(budget.clone(), || NativeSql::new(&transaction)
                .unwrap()
                .query(Statement::AuthorityState, &[])),
            Err(error)
        );
        assert!(budget.charges().is_empty());
    }
    transaction.execute_batch("UPDATE metadata_authority SET identity='i',activation='a',owner=X'01',epoch=1; INSERT INTO metadata_authority SELECT * FROM metadata_authority").unwrap();
    let budget = Arc::new(Budget::default());
    assert_eq!(
        with_allocation_budget(budget.clone(), || NativeSql::new(&transaction)
            .unwrap()
            .query(Statement::AuthorityState, &[])),
        Err(Error::Bounds)
    );
    assert_eq!(budget.charges(), [11 * size_of::<Value>() + 3]);
}

#[test]
fn checked_size_overflow_is_bounds() {
    assert_eq!(container::<Value>(usize::MAX), Err(Error::Bounds));
    assert_eq!(add(usize::MAX, 1), Err(Error::Bounds));
}
