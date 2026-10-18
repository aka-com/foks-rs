#![cfg(feature = "native")]
//! Compare the bounded batch with the existing scalar semantics and SQL work.
use foks_metadata_core::{
    native::NativeSql,
    page::{Cell, PageBuffer, PageEncoder},
    page_catalog::{role_generation_parameters, PageStatement, ROLE_GENERATIONS_SQL},
    personal::{current_key_generation, current_key_generations, PersonalError},
    Error, Sql, Statement, Value,
};
use foks_proto::Role;
use rusqlite::{params, types::Value as SqlValue, Connection, StatementStatus};

const UID: [u8; 33] = [1; 33];

fn database(strict: bool) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    if strict {
        // Exact source DDL, with foreign keys disabled only for this point-query
        // fixture so no unrelated user bootstrap is needed.
        let identity = include_str!("../../foks-server-db/src/schema/identity.sql");
        let ddl = identity.split("CREATE TABLE shared_keys (").nth(1).unwrap();
        let ddl = ddl.split(';').next().unwrap();
        c.execute_batch(&format!("CREATE TABLE shared_keys ({ddl};"))
            .unwrap();
    } else {
        c.execute_batch("CREATE TABLE shared_keys(uid BLOB,role_type INTEGER,visibility INTEGER,generation); CREATE INDEX key_lookup ON shared_keys(uid,role_type,visibility,generation)").unwrap();
    }
    c
}
fn put(c: &Connection, uid: &[u8], role: Role, generation: i64) {
    c.execute("INSERT INTO shared_keys(uid,role_type,visibility,generation,verify_key,exact_hepk) VALUES(?1,?2,?3,?4,zeroblob(33),X'01')",params![uid,role.protocol_value() as i64,role.visibility().unwrap_or(0),generation]).unwrap();
}
fn batch(c: &Connection, roles: &[Role]) -> Result<Vec<Option<u64>>, PersonalError> {
    let tx = c.unchecked_transaction().unwrap();
    let mut host = NativeSql::new(&tx).unwrap();
    let result = current_key_generations(&mut host, &UID, roles);
    assert_eq!(host.calls(), 1);
    tx.rollback().unwrap();
    result
}

#[test]
fn exact_descriptor_and_typed_parameter_bounds() {
    let spec = PageStatement::RoleGenerations.spec();
    assert_eq!(spec.id, 10);
    assert_eq!(spec.sql, ROLE_GENERATIONS_SQL);
    assert_eq!(spec.parameters.len(), 16);
    assert_eq!(spec.columns.len(), 4);
    assert_eq!(spec.maximum_rows, 7);
    assert_eq!(spec.effective_rows().unwrap(), 7);
    assert_eq!(spec.maximum_bytes, 512);
    assert_eq!(spec.maximum_row_bytes().unwrap(), 39);
    assert!(spec.limit_parameter.is_none());
    for count in 1..=7 {
        let p = role_generation_parameters(&UID, &vec![Role::OWNER; count]).unwrap();
        assert_eq!(p[0], Value::Blob(UID.to_vec()));
        assert_eq!(p[1], Value::Integer(count as i64));
        assert!(p[2 + count * 2..].iter().all(|v| v == &Value::Integer(0)));
    }
    for roles in [vec![], vec![Role::NONE], vec![Role::OWNER; 8]] {
        assert!(role_generation_parameters(&UID, &roles).is_err());
    }
    for uid in [vec![], vec![1; 32], vec![2; 33], vec![1; 34]] {
        assert!(role_generation_parameters(&uid, &[Role::OWNER]).is_err());
    }
    for count in [0, 8, i64::MAX] {
        let mut p = role_generation_parameters(&UID, &[Role::OWNER]).unwrap();
        p[1] = Value::Integer(count);
        assert!(spec.validate_parameters(&p).is_err());
    }
}

#[test]
fn batch_matches_scalar_for_presence_duplicates_extremes_and_every_count() {
    let c = database(true);
    let roles = [
        Role::OWNER,
        Role::member(i16::MIN),
        Role::ADMIN,
        Role::member(i16::MAX),
        Role::member(0),
        Role::OWNER,
        Role::member(-1),
    ];
    put(&c, &UID, Role::OWNER, 1);
    put(&c, &UID, Role::OWNER, 9_007_199_254_740_993);
    put(&c, &UID, Role::member(i16::MIN), i64::MAX);
    put(&c, &UID, Role::member(0), 2);
    // Keep existing scalar zero semantics even though admitted schema forbids
    // creating zero generations in ordinary writes.
    c.execute_batch("PRAGMA ignore_check_constraints=ON")
        .unwrap();
    put(&c, &UID, Role::ADMIN, 0);
    for count in 1..=7 {
        let observed = batch(&c, &roles[..count]).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let mut host = NativeSql::new(&tx).unwrap();
        let expected = roles[..count]
            .iter()
            .map(|r| current_key_generation(&mut host, &UID, *r).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(observed, expected);
        assert_eq!(host.calls(), count);
        tx.rollback().unwrap();
    }
    assert_eq!(
        batch(&c, &roles).unwrap(),
        vec![
            Some(9_007_199_254_740_993),
            Some(i64::MAX as u64),
            Some(0),
            None,
            Some(2),
            Some(9_007_199_254_740_993),
            None
        ]
    );
}

#[test]
fn malformed_stored_generation_is_not_missing_or_copied() {
    for generation in [
        SqlValue::Null,
        SqlValue::Text("1".into()),
        SqlValue::Blob(vec![0; 1024 * 1024]),
        SqlValue::Real(1.5),
        SqlValue::Integer(-1),
    ] {
        let c = database(false);
        c.execute(
            "INSERT INTO shared_keys VALUES(?1,3,0,?2)",
            params![UID, generation],
        )
        .unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let p = role_generation_parameters(&UID, &[Role::OWNER]).unwrap();
        let page = NativeSql::new(&tx)
            .unwrap()
            .query_page(PageStatement::RoleGenerations, &p)
            .unwrap();
        assert!(page.as_bytes().len() <= 63);
        assert_eq!(
            page.rows().next().unwrap().cells().nth(1),
            Some(Cell::Integer(1))
        );
        tx.rollback().unwrap();
        assert!(matches!(
            batch(&c, &[Role::OWNER]),
            Err(PersonalError::Corrupt)
        ));
    }
    assert_eq!(batch(&database(false), &[Role::OWNER]).unwrap(), vec![None]);
}

struct Fake(Option<PageBuffer>);
impl Sql for Fake {
    fn query(
        &mut self,
        _: Statement,
        _: &[Value],
    ) -> foks_metadata_core::Result<Option<Vec<Value>>> {
        panic!("unexpected scalar query")
    }
    fn execute(&mut self, _: Statement, _: &[Value]) -> foks_metadata_core::Result<u64> {
        panic!("unexpected mutation")
    }
    fn query_page(
        &mut self,
        statement: PageStatement,
        _: &[Value],
    ) -> foks_metadata_core::Result<PageBuffer> {
        assert_eq!(statement, PageStatement::RoleGenerations);
        self.0.take().ok_or(Error::Storage)
    }
}
fn fake(rows: &[(i64, i64, &str, i64)]) -> Fake {
    let mut encoder = PageEncoder::new(PageStatement::RoleGenerations.spec(), 7).unwrap();
    for (ordinal, present, kind, generation) in rows {
        encoder
            .push(&[
                Cell::Integer(*ordinal),
                Cell::Integer(*present),
                Cell::Text(kind),
                Cell::Integer(*generation),
            ])
            .unwrap();
    }
    Fake(Some(encoder.finish().unwrap()))
}
#[test]
fn malformed_batch_shape_fails_without_partial_success() {
    for rows in [
        vec![],
        vec![(1, 1, "integer", 1)],
        vec![(0, 1, "integer", 1), (0, 1, "integer", 1)],
        vec![(0, 2, "integer", 1)],
        vec![(0, 0, "integer", 0)],
        vec![(0, 0, "null", 1)],
        vec![(0, 1, "null", 0)],
        vec![(0, 1, "integer", -1)],
    ] {
        assert!(matches!(
            current_key_generations(&mut fake(&rows), &UID, &[Role::OWNER]),
            Err(PersonalError::Corrupt)
        ));
    }
    let mut host = fake(&[(0, 1, "integer", 1)]);
    assert!(matches!(
        current_key_generations(&mut host, &UID, &[Role::OWNER, Role::ADMIN]),
        Err(PersonalError::Corrupt)
    ));
    let mut host = fake(&[(0, 1, "integer", 1)]);
    assert!(matches!(
        current_key_generations(&mut host, &UID, &[Role::NONE]),
        Err(PersonalError::BadArguments)
    ));
    assert!(host.0.is_some(), "bad arguments must not query SQL");
}

#[test]
fn indexed_work_is_independent_of_unrelated_and_selected_role_history() {
    let mut baseline = None;
    for history in [1, 100, 10000] {
        let c = database(true);
        let tx = c.unchecked_transaction().unwrap();
        for n in 1..=history {
            put(&tx, &UID, Role::OWNER, n);
            put(&tx, &[2; 33], Role::OWNER, n);
            put(&tx, &UID, Role::member(-1), n);
        }
        tx.commit().unwrap();
        let p = role_generation_parameters(
            &UID,
            &[
                Role::OWNER,
                Role::ADMIN,
                Role::member(i16::MIN),
                Role::OWNER,
                Role::member(i16::MAX),
                Role::member(0),
                Role::OWNER,
            ],
        )
        .unwrap();
        let bindings: Vec<_> = p
            .into_iter()
            .map(|v| match v {
                Value::Blob(b) => SqlValue::Blob(b),
                Value::Integer(i) => SqlValue::Text(i.to_string()),
                _ => unreachable!(),
            })
            .collect();
        let mut query = c.prepare(ROLE_GENERATIONS_SQL).unwrap();
        let mut rows = query.query(rusqlite::params_from_iter(bindings)).unwrap();
        let mut count = 0;
        while rows.next().unwrap().is_some() {
            count += 1;
        }
        drop(rows);
        assert_eq!(count, 7);
        let steps = query.get_status(StatementStatus::VmStep);
        eprintln!("role batch history={history} vm_steps={steps}");
        if let Some(prior) = baseline {
            assert_eq!(steps, prior);
        } else {
            baseline = Some(steps);
        }
    }
}
