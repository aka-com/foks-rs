//! Private transaction-local connection checks, never a reusable readiness proof.
mod scratch;
#[cfg(test)]
mod tests;

pub(super) use scratch::SchemaScratch;

use super::operation::OperationControl;
use foks_metadata_core::{
    native::NativeSql,
    page::{Cell, PageBuffer, Row},
    page_catalog::{schema_objects_parameters, PageStatement},
    Error, Result, Sql, SqlCallBudget, Statement,
};
use foks_schema_contract::{Contract, MatchPolicy, SchemaObject, SchemaVerifier};
use rusqlite::{types::ValueRef, Connection};

const PAGE_ROWS: u16 = 8;
const MAXIMUM_OBJECTS: usize = 1024;
const MAXIMUM_DDL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FixedExpected {
    Integer(i64),
    Text(&'static str),
}

/// One immutable inventory for execution, profile hashing and policy qualification.
/// These are readbacks, not repair commands or a caller-selected SQL interface.
pub(super) struct FixedCheck {
    pub(super) name: &'static str,
    pub(super) sql: &'static str,
    pub(super) expected: FixedExpected,
    pub(super) native_only: bool,
}

pub(super) const FIXED_CHECKS: &[FixedCheck] = &[
    FixedCheck {
        name: "foreign_keys",
        sql: "PRAGMA foreign_keys",
        expected: FixedExpected::Integer(1),
        native_only: false,
    },
    FixedCheck {
        name: "ignore_check_constraints",
        sql: "PRAGMA ignore_check_constraints",
        expected: FixedExpected::Integer(0),
        native_only: false,
    },
    FixedCheck {
        name: "recursive_triggers",
        sql: "PRAGMA recursive_triggers",
        expected: FixedExpected::Integer(1),
        native_only: false,
    },
    FixedCheck {
        name: "trusted_schema",
        sql: "PRAGMA trusted_schema",
        expected: FixedExpected::Integer(0),
        native_only: true,
    },
    FixedCheck {
        name: "journal_mode",
        sql: "PRAGMA main.journal_mode",
        expected: FixedExpected::Text("wal"),
        native_only: true,
    },
    FixedCheck {
        name: "synchronous",
        sql: "PRAGMA main.synchronous",
        expected: FixedExpected::Integer(2),
        native_only: true,
    },
    FixedCheck {
        name: "empty_temp",
        sql: "SELECT CASE WHEN EXISTS(SELECT 1 FROM temp.sqlite_schema) THEN '1' ELSE '0' END",
        expected: FixedExpected::Text("0"),
        native_only: false,
    },
    FixedCheck {
        name: "no_attachments",
        sql: "SELECT CASE WHEN EXISTS(SELECT 1 FROM pragma_table_list WHERE schema NOT IN ('main','temp')) THEN '1' ELSE '0' END AS value",
        expected: FixedExpected::Text("0"),
        native_only: false,
    },
];

/// Locally observed coordinates, not authority to use a connection later.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Observation {
    cookie: u64,
    incarnation: [u8; 32],
    revision: u64,
}

#[cfg(test)]
impl Observation {
    pub(super) fn cookie(self) -> u64 {
        self.cookie
    }
    pub(super) fn incarnation(self) -> [u8; 32] {
        self.incarnation
    }
    pub(super) fn revision(self) -> u64 {
        self.revision
    }
}

fn transaction(connection: &Connection, control: &OperationControl) -> Result<()> {
    control.check()?;
    if connection.is_autocommit() {
        return Err(Error::Storage);
    }
    Ok(())
}

fn fixed_checks(
    connection: &Connection,
    budget: &mut SqlCallBudget,
    control: &OperationControl,
) -> Result<()> {
    for check in FIXED_CHECKS {
        control.check()?;
        budget.attempt()?;
        let mut statement = connection.prepare(check.sql).map_err(|_| Error::Storage)?;
        if statement.column_count() != 1 {
            return Err(Error::Bounds);
        }
        let mut rows = statement.query([]).map_err(|_| Error::Storage)?;
        let row = rows
            .next()
            .map_err(|_| Error::Storage)?
            .ok_or(Error::Storage)?;
        let actual = row.get_ref(0).map_err(|_| Error::Storage)?;
        let matches = match (actual, check.expected) {
            (ValueRef::Integer(actual), FixedExpected::Integer(expected)) => actual == expected,
            (ValueRef::Text(actual), FixedExpected::Text(expected)) => {
                actual == expected.as_bytes()
            }
            _ => false,
        };
        if !matches || rows.next().map_err(|_| Error::Storage)?.is_some() {
            return Err(Error::Storage);
        }
        control.check()?;
    }
    Ok(())
}

/// Canonical nonnegative i64 decimal, consumed without allocating a String.
fn nonnegative(value: ValueRef<'_>) -> Result<u64> {
    let ValueRef::Text(bytes) = value else {
        return Err(Error::Storage);
    };
    if bytes.is_empty()
        || bytes.len() > 19
        || bytes.len() > 1 && bytes[0] == b'0'
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return Err(Error::Storage);
    }
    let value = bytes.iter().try_fold(0u64, |value, byte| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(*byte - b'0')))
            .ok_or(Error::Storage)
    })?;
    if value > i64::MAX as u64 {
        return Err(Error::Storage);
    }
    Ok(value)
}

fn cookie(
    connection: &Connection,
    budget: &mut SqlCallBudget,
    control: &OperationControl,
) -> Result<u64> {
    control.check()?;
    budget.attempt()?;
    let mut statement = connection
        .prepare(Statement::ReadSchemaCookie.spec().sql)
        .map_err(|_| Error::Storage)?;
    if statement.column_count() != 1 {
        return Err(Error::Bounds);
    }
    let mut rows = statement.query([]).map_err(|_| Error::Storage)?;
    let row = rows
        .next()
        .map_err(|_| Error::Storage)?
        .ok_or(Error::Storage)?;
    let result = nonnegative(row.get_ref(0).map_err(|_| Error::Storage)?)?;
    if rows.next().map_err(|_| Error::Storage)?.is_some() {
        return Err(Error::Storage);
    }
    control.check()?;
    Ok(result)
}

fn dependency(
    connection: &Connection,
    budget: &mut SqlCallBudget,
    control: &OperationControl,
) -> Result<([u8; 32], u64)> {
    control.check()?;
    budget.attempt()?;
    let mut statement = connection
        .prepare(Statement::ReadDependency.spec().sql)
        .map_err(|_| Error::Storage)?;
    if statement.column_count() != 5 {
        return Err(Error::Bounds);
    }
    let mut rows = statement.query([]).map_err(|_| Error::Storage)?;
    let row = rows
        .next()
        .map_err(|_| Error::Storage)?
        .ok_or(Error::Storage)?;
    if row.get_ref(0).map_err(|_| Error::Storage)? != ValueRef::Text(b"blob")
        || row.get_ref(1).map_err(|_| Error::Storage)? != ValueRef::Text(b"32")
        || row.get_ref(3).map_err(|_| Error::Storage)? != ValueRef::Text(b"integer")
    {
        return Err(Error::Storage);
    }
    let ValueRef::Blob(incarnation) = row.get_ref(2).map_err(|_| Error::Storage)? else {
        return Err(Error::Storage);
    };
    let incarnation: [u8; 32] = incarnation.try_into().map_err(|_| Error::Storage)?;
    let revision = nonnegative(row.get_ref(4).map_err(|_| Error::Storage)?)?;
    if incarnation == [0; 32] || revision == 0 || rows.next().map_err(|_| Error::Storage)?.is_some()
    {
        return Err(Error::Storage);
    }
    control.check()?;
    Ok((incarnation, revision))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Cursor {
    kind: [u8; 8],
    kind_length: u8,
    name: [u8; 256],
    name_length: u16,
}

impl Cursor {
    fn from_object(object: SchemaObject<'_>) -> Result<Self> {
        let mut result = Self {
            kind: [0; 8],
            kind_length: u8::try_from(object.kind.len()).map_err(|_| Error::Bounds)?,
            name: [0; 256],
            name_length: u16::try_from(object.name.len()).map_err(|_| Error::Bounds)?,
        };
        result
            .kind
            .get_mut(..object.kind.len())
            .ok_or(Error::Bounds)?
            .copy_from_slice(object.kind.as_bytes());
        result
            .name
            .get_mut(..object.name.len())
            .ok_or(Error::Bounds)?
            .copy_from_slice(object.name.as_bytes());
        Ok(result)
    }
    fn after(&self) -> Result<(&str, &str)> {
        Ok((
            std::str::from_utf8(&self.kind[..usize::from(self.kind_length)])
                .map_err(|_| Error::Bounds)?,
            std::str::from_utf8(&self.name[..usize::from(self.name_length)])
                .map_err(|_| Error::Bounds)?,
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PageOutcome {
    last: Option<Cursor>,
    objects: usize,
    ddl_bytes: usize,
}

/// Target-sized diagnostics, not an additive charge or an ownership capability.
/// Cursor/outcome/verifier are inline scan state. The scratch Arc is already
/// charged exactly once by SchemaScratch::new, apart from its inline handle.
#[derive(Clone, Copy, Debug)]
pub(super) struct ControlLayouts {
    pub(super) cursor: std::alloc::Layout,
    pub(super) page_outcome: std::alloc::Layout,
    pub(super) verifier: std::alloc::Layout,
    pub(super) scratch_budget_arc: std::alloc::Layout,
}

pub(super) fn control_layouts() -> Result<ControlLayouts> {
    Ok(ControlLayouts {
        cursor: std::alloc::Layout::new::<Cursor>(),
        page_outcome: std::alloc::Layout::new::<PageOutcome>(),
        verifier: std::alloc::Layout::new::<SchemaVerifier<'static, 'static>>(),
        scratch_budget_arc: scratch::budget_arc_layout()?,
    })
}

fn object(row: Row<'_>) -> Result<SchemaObject<'_>> {
    let mut cells = row.cells();
    let mut fields = [""; 4];
    for (field, maximum) in fields.iter_mut().zip([8, 256, 256, 65536]) {
        let (Some(Cell::Text(kind)), Some(Cell::Integer(length)), Some(Cell::Text(value))) =
            (cells.next(), cells.next(), cells.next())
        else {
            return Err(Error::Storage);
        };
        if kind != "text"
            || usize::try_from(length).ok() != Some(value.len())
            || value.len() > maximum
        {
            return Err(Error::Bounds);
        }
        *field = value;
    }
    if cells.next().is_some()
        || !matches!(fields[0], "table" | "index" | "trigger" | "view")
        || fields[1].is_empty()
    {
        return Err(Error::Storage);
    }
    Ok(SchemaObject {
        kind: fields[0],
        name: fields[1],
        table: fields[2],
        sql: fields[3],
    })
}

fn observe_page(
    page: &PageBuffer,
    objects: usize,
    ddl_bytes: usize,
    verifier: &mut SchemaVerifier<'_, '_>,
    control: &OperationControl,
) -> Result<PageOutcome> {
    if page.len() > usize::from(PAGE_ROWS) {
        return Err(Error::Bounds);
    }
    let mut outcome = PageOutcome {
        last: None,
        objects,
        ddl_bytes,
    };
    // Validate the complete page before allowing even its first object to alter
    // verifier progress. These are borrowed cells, not a decoded schema Vec.
    for row in page.rows() {
        control.check()?;
        let object = object(row)?;
        outcome.objects = outcome.objects.checked_add(1).ok_or(Error::Bounds)?;
        outcome.ddl_bytes = outcome
            .ddl_bytes
            .checked_add(object.sql.len())
            .ok_or(Error::Bounds)?;
        if outcome.objects > MAXIMUM_OBJECTS || outcome.ddl_bytes > MAXIMUM_DDL_BYTES {
            return Err(Error::Bounds);
        }
        outcome.last = Some(Cursor::from_object(object)?);
    }
    for row in page.rows() {
        control.check()?;
        verifier.observe(object(row)?).map_err(|_| Error::Storage)?;
    }
    control.check()?;
    Ok(outcome)
}

fn schema(
    connection: &Connection,
    budget: &mut SqlCallBudget,
    contract: &Contract<'static>,
    scratch: &SchemaScratch,
    control: &OperationControl,
) -> Result<()> {
    scratch.reserve_verifier()?;
    let mut verifier = contract.verifier(MatchPolicy::ExactUnion);
    let mut progress = PageOutcome {
        last: None,
        objects: 0,
        ddl_bytes: 0,
    };
    // The final empty page is required even after a short nonempty page. At the
    // 1,024-object boundary the 129th attempt either proves EOF or refuses.
    for _ in 0..=MAXIMUM_OBJECTS / usize::from(PAGE_ROWS) {
        control.check()?;
        let next = scratch.run(|| {
            let after = progress.last.as_ref().map(Cursor::after).transpose()?;
            let parameters = schema_objects_parameters(after, PAGE_ROWS)?;
            let mut sql = NativeSql::new_with_budget(connection, budget)?;
            let page = sql.query_page(PageStatement::SchemaObjects, &parameters)?;
            observe_page(
                &page,
                progress.objects,
                progress.ddl_bytes,
                &mut verifier,
                control,
            )
            // Page, parameters and NativeSql all drop before the scratch hook
            // restores. Only fixed Copy progress can leave this closure.
        })?;
        if next.last.is_none() {
            return verifier.finish().map_err(|_| Error::Storage);
        }
        progress = next;
    }
    Err(Error::Bounds)
}

pub(super) fn entry(
    connection: &Connection,
    budget: &mut SqlCallBudget,
    contract: &Contract<'static>,
    scratch: &SchemaScratch,
    control: &OperationControl,
) -> Result<Observation> {
    let result = (|| {
        transaction(connection, control)?;
        fixed_checks(connection, budget, control)?;
        let before = cookie(connection, budget, control)?;
        schema(connection, budget, contract, scratch, control)?;
        let after = cookie(connection, budget, control)?;
        if before != after {
            return Err(Error::Storage);
        }
        let (incarnation, revision) = dependency(connection, budget, control)?;
        Ok(Observation {
            cookie: after,
            incarnation,
            revision,
        })
    })();
    if result.is_err() {
        budget.fail();
        scratch.fail();
    }
    result
}

pub(super) fn exit(
    connection: &Connection,
    budget: &mut SqlCallBudget,
    _contract: &Contract<'static>,
    scratch: &SchemaScratch,
    control: &OperationControl,
    expected: Observation,
) -> Result<()> {
    let result = (|| {
        transaction(connection, control)?;
        fixed_checks(connection, budget, control)?;
        let cookie = cookie(connection, budget, control)?;
        let (incarnation, revision) = dependency(connection, budget, control)?;
        if (Observation {
            cookie,
            incarnation,
            revision,
        }) != expected
        {
            return Err(Error::Storage);
        }
        Ok(())
    })();
    if result.is_err() {
        budget.fail();
        scratch.fail();
    }
    result
}
