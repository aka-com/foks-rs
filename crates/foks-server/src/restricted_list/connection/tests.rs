use super::super::operation::test_memory;
use super::*;
use foks_metadata_core::{owned_list::ListRetainedOwner, page::PageEncoder};
use foks_schema_contract::ContractObject;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[derive(Default)]
pub(super) struct Parent {
    pub(super) held: AtomicUsize,
    pub(super) reservations: AtomicUsize,
    pub(super) retained: AtomicUsize,
    pub(super) failed: AtomicBool,
    pub(super) refuse_workspace: AtomicBool,
    pub(super) refuse_retained: AtomicBool,
}
impl foks_snowpack::AllocationBudget for Parent {
    fn reserve(&self, _: usize) -> bool {
        true
    }
}
impl foks_rpc::ResponseAllocationBudget for Parent {
    fn reserve(&self, _: usize, _: bool) -> bool {
        true
    }
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
        if self.refuse_retained.load(Ordering::SeqCst) {
            return false;
        }
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

fn control() -> Arc<OperationControl> {
    OperationControl::test(std::time::Duration::from_secs(60), test_memory())
}

struct Fixture {
    connection: Connection,
    _file: tempfile::NamedTempFile,
    contract: Contract<'static>,
}
fn contract(connection: &Connection) -> Contract<'static> {
    let mut query = connection.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE substr(name,1,7)!='sqlite_' COLLATE NOCASE ORDER BY type COLLATE BINARY,name COLLATE BINARY").unwrap();
    let objects = query
        .query_map([], |row| {
            Ok(ContractObject::owned(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    Contract::new(objects).unwrap()
}
impl Fixture {
    fn new(objects: usize) -> Self {
        assert!(objects >= 1);
        let file = tempfile::NamedTempFile::new().unwrap();
        let connection = Connection::open(file.path()).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA ignore_check_constraints=OFF; PRAGMA recursive_triggers=ON; PRAGMA trusted_schema=OFF; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE metadata_read_dependency(singleton INTEGER PRIMARY KEY,incarnation BLOB NOT NULL,revision INTEGER NOT NULL)").unwrap();
        connection
            .execute(
                "INSERT INTO metadata_read_dependency VALUES(1,?1,1)",
                [[1u8; 32].as_slice()],
            )
            .unwrap();
        connection.execute_batch("BEGIN").unwrap();
        for index in 1..objects {
            connection
                .execute_batch(&format!("CREATE TABLE t_{index:04}(v)"))
                .unwrap();
        }
        connection.execute_batch("COMMIT").unwrap();
        let contract = contract(&connection);
        Self {
            connection,
            _file: file,
            contract,
        }
    }
}

#[test]
fn two_full_intervals_count_every_page_eof_exit_and_unused_abort_credit() {
    for objects in [1, 7, 8, 9, 1024] {
        let fixture = Fixture::new(objects);
        let control = control();
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let mut budget = SqlCallBudget::new();
        let pages = objects.div_ceil(8) + 1;
        for interval in 1..=2 {
            budget.begin_cleanup_reservation().unwrap();
            budget.attempt().unwrap();
            fixture.connection.execute_batch("BEGIN").unwrap();
            let start = budget.attempts();
            let observed = entry(
                &fixture.connection,
                &mut budget,
                &fixture.contract,
                &scratch,
                &control,
            )
            .unwrap();
            assert_eq!(budget.attempts() - start, 11 + pages);
            assert_eq!(observed.incarnation(), [1; 32]);
            assert_eq!(observed.revision(), 1);
            assert!(observed.cookie() > 0);
            let start = budget.attempts();
            exit(
                &fixture.connection,
                &mut budget,
                &fixture.contract,
                &scratch,
                &control,
                observed,
            )
            .unwrap();
            assert_eq!(budget.attempts() - start, 10);
            budget.attempt().unwrap();
            fixture.connection.execute_batch("COMMIT").unwrap();
            budget.finish_cleanup_reservation().unwrap();
            assert_eq!(budget.charged(), interval * (24 + pages));
            assert_eq!(budget.attempts(), budget.charged() - interval);
        }
        if objects == 1024 {
            assert_eq!((budget.charged(), budget.attempts()), (306, 304));
        }
        assert!(!budget.failed());
        assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
        assert_eq!(
            parent.retained.load(Ordering::SeqCst),
            2 * scratch::VERIFIER_BYTES
        );
        assert!(!parent.failed());
    }
}

#[test]
fn incorrect_native_settings_temp_and_attached_schemas_are_not_repaired() {
    for (mutation, attempted) in [
        ("PRAGMA foreign_keys=OFF", 1),
        ("PRAGMA ignore_check_constraints=ON", 2),
        ("PRAGMA recursive_triggers=OFF", 3),
        ("PRAGMA trusted_schema=ON", 4),
        ("PRAGMA journal_mode=DELETE", 5),
        ("PRAGMA synchronous=NORMAL", 6),
        ("CREATE TEMP TABLE unexpected(v)", 7),
        ("ATTACH ':memory:' AS unexpected", 8),
        (
            "ATTACH ':memory:' AS unexpected; CREATE TABLE unexpected.t(v)",
            8,
        ),
    ] {
        let fixture = Fixture::new(1);
        fixture.connection.execute_batch(mutation).unwrap();
        fixture.connection.execute_batch("BEGIN").unwrap();
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let mut budget = SqlCallBudget::new();
        assert_eq!(
            entry(
                &fixture.connection,
                &mut budget,
                &fixture.contract,
                &scratch,
                &control()
            ),
            Err(Error::Storage),
            "{mutation}"
        );
        assert_eq!(budget.attempts(), attempted, "{mutation}");
        assert!(budget.failed());
        assert!(parent.failed());
        assert_eq!(parent.retained.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn missing_changed_and_extra_schema_objects_refuse_the_exact_union() {
    for mutation in [
        "DROP TABLE t_0001",
        "ALTER TABLE t_0001 ADD COLUMN extra",
        "CREATE TABLE unexpected(v)",
    ] {
        let fixture = Fixture::new(2);
        fixture.connection.execute_batch(mutation).unwrap();
        fixture.connection.execute_batch("BEGIN").unwrap();
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let mut budget = SqlCallBudget::new();
        assert_eq!(
            entry(
                &fixture.connection,
                &mut budget,
                &fixture.contract,
                &scratch,
                &control()
            ),
            Err(Error::Storage),
            "{mutation}"
        );
        assert!(budget.attempts() <= 11);
        assert!(budget.failed());
        assert!(parent.failed());
    }
}

#[test]
fn final_extra_object_is_refused_on_the_129th_schema_attempt() {
    let fixture = Fixture::new(1025);
    fixture.connection.execute_batch("BEGIN").unwrap();
    let parent = Arc::new(Parent::default());
    let scratch = SchemaScratch::new(parent.clone()).unwrap();
    let mut budget = SqlCallBudget::new();
    assert_eq!(
        entry(
            &fixture.connection,
            &mut budget,
            &fixture.contract,
            &scratch,
            &control()
        ),
        Err(Error::Bounds)
    );
    assert_eq!(budget.attempts(), 8 + 1 + 129);
    assert!(budget.failed());
    assert!(parent.failed());
}

#[test]
fn exit_refuses_schema_cookie_or_dependency_changes_without_another_scan() {
    for mutation in [
        "CREATE TABLE unexpected(v)",
        "UPDATE metadata_read_dependency SET revision=2",
        "UPDATE metadata_read_dependency SET incarnation=zeroblob(32)",
    ] {
        let fixture = Fixture::new(1);
        fixture.connection.execute_batch("BEGIN").unwrap();
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let mut budget = SqlCallBudget::new();
        let control = control();
        let observed = entry(
            &fixture.connection,
            &mut budget,
            &fixture.contract,
            &scratch,
            &control,
        )
        .unwrap();
        fixture.connection.execute_batch(mutation).unwrap();
        let before = budget.attempts();
        assert_eq!(
            exit(
                &fixture.connection,
                &mut budget,
                &fixture.contract,
                &scratch,
                &control,
                observed
            ),
            Err(Error::Storage)
        );
        assert_eq!(budget.attempts() - before, 10);
        assert_eq!(
            parent.retained.load(Ordering::SeqCst),
            scratch::VERIFIER_BYTES
        );
        assert!(budget.failed());
    }
}

#[test]
fn absent_transaction_cancelled_operation_and_verifier_refusal_fail_closed() {
    for case in 0..3 {
        let fixture = Fixture::new(1);
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let control = control();
        let mut budget = SqlCallBudget::new();
        if case != 0 {
            fixture.connection.execute_batch("BEGIN").unwrap();
        }
        if case == 1 {
            control.cancel();
        }
        if case == 2 {
            parent.refuse_retained.store(true, Ordering::SeqCst);
        }
        let expected = [Error::Storage, Error::Expired, Error::Bounds][case].clone();
        assert_eq!(
            entry(
                &fixture.connection,
                &mut budget,
                &fixture.contract,
                &scratch,
                &control
            ),
            Err(expected)
        );
        assert_eq!(budget.attempts(), if case == 2 { 9 } else { 0 });
        assert!(budget.failed());
        assert!(parent.failed());
    }
}

#[test]
fn fixed_decimal_validation_never_coerces_types_or_noncanonical_values() {
    for bytes in [
        b"".as_slice(),
        b"00",
        b"01",
        b"+1",
        b"-1",
        b" 1",
        b"1 ",
        b"1.0",
        b"9223372036854775808",
        b"18446744073709551616",
    ] {
        assert_eq!(nonnegative(ValueRef::Text(bytes)), Err(Error::Storage));
    }
    assert_eq!(nonnegative(ValueRef::Integer(1)), Err(Error::Storage));
    assert_eq!(nonnegative(ValueRef::Blob(b"1")), Err(Error::Storage));
    assert_eq!(nonnegative(ValueRef::Text(b"0")), Ok(0));
    assert_eq!(
        nonnegative(ValueRef::Text(b"9223372036854775807")),
        Ok(i64::MAX as u64)
    );
}

#[test]
fn dependency_guard_rejects_suppressed_missing_and_duplicate_rows() {
    for mutation in [
        "DELETE FROM metadata_read_dependency",
        "UPDATE metadata_read_dependency SET incarnation=zeroblob(31)",
        "UPDATE metadata_read_dependency SET incarnation=zeroblob(33)",
        "UPDATE metadata_read_dependency SET incarnation='not a blob'",
        "UPDATE metadata_read_dependency SET incarnation=zeroblob(32)",
        "UPDATE metadata_read_dependency SET revision=0",
        "UPDATE metadata_read_dependency SET revision=-1",
        "UPDATE metadata_read_dependency SET revision=1.5",
        "UPDATE metadata_read_dependency SET revision='1x'",
        "UPDATE metadata_read_dependency SET revision=X'01'",
        "ALTER TABLE metadata_read_dependency RENAME TO original; CREATE TABLE metadata_read_dependency AS SELECT * FROM original; INSERT INTO metadata_read_dependency SELECT * FROM original",
    ] {
        let fixture = Fixture::new(1);
        fixture.connection.execute_batch(mutation).unwrap();
        fixture.connection.execute_batch("BEGIN").unwrap();
        let mut budget = SqlCallBudget::new();
        assert_eq!(dependency(&fixture.connection, &mut budget, &control()), Err(Error::Storage), "{mutation}");
        assert_eq!(budget.attempts(), 1);
    }
    let fixture = Fixture::new(1);
    fixture
        .connection
        .execute_batch("UPDATE metadata_read_dependency SET revision=9223372036854775807; BEGIN")
        .unwrap();
    let mut budget = SqlCallBudget::new();
    assert_eq!(
        dependency(&fixture.connection, &mut budget, &control()),
        Ok(([1; 32], i64::MAX as u64))
    );
}

fn cells(object: SchemaObject<'_>) -> [Cell<'_>; 12] {
    [
        Cell::Text("text"),
        Cell::Integer(object.kind.len() as i64),
        Cell::Text(object.kind),
        Cell::Text("text"),
        Cell::Integer(object.name.len() as i64),
        Cell::Text(object.name),
        Cell::Text("text"),
        Cell::Integer(object.table.len() as i64),
        Cell::Text(object.table),
        Cell::Text("text"),
        Cell::Integer(object.sql.len() as i64),
        Cell::Text(object.sql),
    ]
}

#[test]
fn complete_page_semantic_and_aggregate_validation_precedes_verifier_progress() {
    let valid = SchemaObject {
        kind: "table",
        name: "t",
        table: "t",
        sql: "CREATE TABLE t(v)",
    };
    let contract = Contract::new(vec![ContractObject::borrowed(valid)]).unwrap();
    let control = control();
    for failure in 0..5 {
        let mut encoder = PageEncoder::new(PageStatement::SchemaObjects.spec(), PAGE_ROWS).unwrap();
        encoder.push(&cells(valid)).unwrap();
        if failure < 3 {
            let mut invalid = cells(valid);
            match failure {
                0 => invalid[0] = Cell::Text("blob"),
                1 => invalid[4] = Cell::Integer(9),
                _ => invalid[11] = Cell::Null,
            }
            encoder.push(&invalid).unwrap();
        }
        let page = encoder.finish().unwrap();
        let mut verifier = contract.verifier(MatchPolicy::ExactUnion);
        let (objects, bytes) = match failure {
            3 => (MAXIMUM_OBJECTS, 0),
            4 => (0, MAXIMUM_DDL_BYTES),
            _ => (0, 0),
        };
        assert!(observe_page(&page, objects, bytes, &mut verifier, &control).is_err());
        // If the first valid row had been observed before the later bad row,
        // this would fail as a duplicate (or with a poisoned verifier).
        verifier.observe(valid).unwrap();
        verifier.finish().unwrap();
    }
}

#[test]
fn unicode_cursor_is_an_owned_fixed_copy_and_field_caps_fail_before_observation() {
    let name = "é".repeat(128);
    let valid = SchemaObject {
        kind: "table",
        name: &name,
        table: &name,
        sql: "CREATE TABLE t(v)",
    };
    let cursor = Cursor::from_object(valid).unwrap();
    assert_eq!(cursor.after().unwrap(), ("table", name.as_str()));
    drop(name);
    assert_eq!(cursor.after().unwrap().1.len(), 256);
    for (name_bytes, sql_bytes, accepted) in [
        (256, 0, true),
        (257, 0, false),
        (0, 65536, true),
        (0, 65537, false),
    ] {
        let fixture = Fixture::new(1);
        let mutation = if name_bytes != 0 {
            format!("CREATE TABLE \"{}\"(v)", "x".repeat(name_bytes))
        } else {
            let prefix = "CREATE VIEW extra AS SELECT '";
            let suffix = "' AS v";
            format!(
                "{prefix}{}{suffix}",
                "x".repeat(sql_bytes - prefix.len() - suffix.len())
            )
        };
        fixture.connection.execute_batch(&mutation).unwrap();
        let expected = contract(&fixture.connection);
        fixture.connection.execute_batch("BEGIN").unwrap();
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let mut budget = SqlCallBudget::new();
        assert_eq!(
            entry(
                &fixture.connection,
                &mut budget,
                &expected,
                &scratch,
                &control()
            )
            .is_ok(),
            accepted
        );
        assert_eq!(parent.failed(), !accepted);
        assert_eq!(budget.attempts(), if accepted { 13 } else { 10 });
    }
}

#[test]
fn aggregate_ddl_limit_is_checked_across_full_sized_real_pages() {
    for (views, accepted) in [(31, true), (32, false)] {
        let fixture = Fixture::new(1);
        fixture.connection.execute_batch("BEGIN").unwrap();
        for index in 0..views {
            let prefix = format!("CREATE VIEW v_{index:02} AS SELECT '");
            let suffix = "' AS v";
            let sql = format!(
                "{prefix}{}{suffix}",
                "x".repeat(65536 - prefix.len() - suffix.len())
            );
            fixture.connection.execute_batch(&sql).unwrap();
        }
        fixture.connection.execute_batch("COMMIT").unwrap();
        let expected = contract(&fixture.connection);
        fixture.connection.execute_batch("BEGIN").unwrap();
        let parent = Arc::new(Parent::default());
        let scratch = SchemaScratch::new(parent.clone()).unwrap();
        let mut budget = SqlCallBudget::new();
        assert_eq!(
            entry(
                &fixture.connection,
                &mut budget,
                &expected,
                &scratch,
                &control()
            )
            .is_ok(),
            accepted
        );
        assert_eq!(parent.failed(), !accepted);
        assert_eq!(budget.attempts(), if accepted { 16 } else { 14 });
        assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
        assert_eq!(parent.retained.load(Ordering::SeqCst), 2080);
    }
}

#[test]
fn control_layouts_bound_inline_scan_state_and_match_the_actual_arc_charge() {
    let layouts = control_layouts().unwrap();
    for (layout, maximum) in [
        (layouts.cursor, 288),
        (layouts.page_outcome, 320),
        (layouts.verifier, 128),
        (layouts.scratch_budget_arc, 128),
    ] {
        assert!(layout.size() <= maximum);
        assert!(layout.align() <= 8);
    }
    assert_eq!(layouts.cursor, std::alloc::Layout::new::<Cursor>());
    assert_eq!(
        layouts.page_outcome,
        std::alloc::Layout::new::<PageOutcome>()
    );
    assert_eq!(
        layouts.verifier,
        std::alloc::Layout::new::<SchemaVerifier<'static, 'static>>()
    );
    let parent = Arc::new(Parent::default());
    let scratch = SchemaScratch::new(parent.clone()).unwrap();
    let terms = SchemaScratch::terms().unwrap();
    assert_eq!(layouts.scratch_budget_arc.size(), terms.control_bytes);
    assert_eq!(
        parent.held.load(Ordering::SeqCst),
        terms.workspace_bytes + layouts.scratch_budget_arc.size()
    );
    assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
    // The inline scan layouts above are diagnostics only. They must not add
    // another retained allocation or duplicate the single scratch Arc charge.
    assert_eq!(parent.retained.load(Ordering::SeqCst), 0);
    drop(scratch);
}
