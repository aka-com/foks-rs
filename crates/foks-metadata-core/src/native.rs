//! Synchronous host for an already-open native transaction. It does not commit.
use crate::{Error, Result, Sql, SqlCallBudget, Statement, Value, MAX_SQL_CALLS};

use rusqlite::{
    types::{Value as SqlValue, ValueRef},
    Connection,
};

mod scalar;

pub struct NativeSql<'a> {
    connection: &'a Connection,
    calls: Counter<'a>,
}

enum Counter<'a> {
    Local(usize),
    Borrowed(&'a mut SqlCallBudget),
}

impl Counter<'_> {
    fn before_validation(&mut self) -> Result<()> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Borrowed(budget) => budget.attempt(),
        }
    }

    fn after_validation(&mut self) -> Result<()> {
        match self {
            // Preserve the qualification host's existing local ordering.
            Self::Local(calls) => {
                *calls += 1;
                if *calls > MAX_SQL_CALLS {
                    return Err(Error::Bounds);
                }
                Ok(())
            }
            Self::Borrowed(_) => Ok(()),
        }
    }
}
impl<'a> NativeSql<'a> {
    pub fn new(connection: &'a Connection) -> Result<Self> {
        if connection.is_autocommit() {
            return Err(Error::Storage);
        }
        Ok(Self {
            connection,
            calls: Counter::Local(0),
        })
    }

    /// Borrow the enclosing operation's original budget. Constructing another
    /// adapter for a later interval cannot reset that operation's accounting.
    pub fn new_with_budget(
        connection: &'a Connection,
        budget: &'a mut SqlCallBudget,
    ) -> Result<Self> {
        if connection.is_autocommit() {
            return Err(Error::Storage);
        }
        if budget.failed() {
            return Err(Error::Bounds);
        }
        Ok(Self {
            connection,
            calls: Counter::Borrowed(budget),
        })
    }

    pub fn calls(&self) -> usize {
        match &self.calls {
            Counter::Local(calls) => *calls,
            Counter::Borrowed(budget) => budget.attempts(),
        }
    }
    fn admit(
        &mut self,
        statement: Statement,
        parameters: &[Value],
        write: bool,
    ) -> Result<Vec<SqlValue>> {
        self.calls.before_validation()?;
        statement.validate_parameters(parameters)?;
        self.calls.after_validation()?;
        if statement.spec().write != write {
            return Err(Error::Bounds);
        }
        scalar::bindings(parameters)
    }
}
impl Sql for NativeSql<'_> {
    fn query_page(
        &mut self,
        statement: crate::page_catalog::PageStatement,
        parameters: &[Value],
    ) -> Result<crate::page::PageBuffer> {
        use crate::page::{Cell, PageEncoder, PageKind};
        let spec = statement.spec();
        self.calls.before_validation()?;
        spec.validate_parameters(parameters)?;
        self.calls.after_validation()?;
        let limit = spec.requested_rows(parameters)?;
        // Admit bindings and one bounded cursor payload before cloning parameters
        // or preparing/advancing SQLite. Fixed row/container overhead and every
        // per-row Cell vector are included. The encoded page charges its actual
        // allocations separately; these are cumulative charges, not peak RSS.
        // SQLite's internal query working memory still needs host-level control.
        let parameter_bytes = parameters.iter().try_fold(0usize, |sum, value| {
            let bytes = match value {
                Value::Null => 0,
                Value::Integer(_) => 20,
                Value::Text(s) => s.len(),
                Value::Blob(b) => b.len(),
            };
            sum.checked_add(bytes).ok_or(Error::Bounds)
        })?;
        let containers = parameters.len() * std::mem::size_of::<SqlValue>()
            + spec.columns.len()
                * (512 + (usize::from(limit) + 1) * std::mem::size_of::<Cell<'_>>());
        crate::page::charge(
            parameter_bytes
                .checked_add(containers)
                .and_then(|n| n.checked_add(spec.maximum_row_bytes().ok()?))
                .ok_or(Error::Bounds)?,
        )?;
        let mut bindings = Vec::with_capacity(parameters.len());
        for value in parameters {
            bindings.push(match value {
                Value::Null => SqlValue::Null,
                Value::Integer(n) => SqlValue::Text(
                    String::from_utf8(crate::scalar::Decimal::new(*n).bytes().to_vec())
                        .map_err(|_| Error::Storage)?,
                ),
                Value::Text(s) => SqlValue::Text(s.clone()),
                Value::Blob(b) => SqlValue::Blob(b.clone()),
            });
        }
        let mut query = self
            .connection
            .prepare(spec.sql)
            .map_err(|_| Error::Storage)?;
        if query.column_count() != spec.columns.len() {
            return Err(Error::Bounds);
        }
        let mut rows = query
            .query(rusqlite::params_from_iter(bindings))
            .map_err(|_| Error::Storage)?;
        let mut encoded = PageEncoder::new(spec, limit)?;
        for index in 0..=limit {
            let Some(row) = rows.next().map_err(|_| Error::Storage)? else {
                break;
            };
            if index == limit {
                return Err(Error::Bounds);
            }
            let mut cells = Vec::with_capacity(spec.columns.len());
            for (index, column) in spec.columns.iter().enumerate() {
                let value = row.get_ref(index).map_err(|_| Error::Storage)?;
                cells.push(match (value, column.kind) {
                    (ValueRef::Null, _) if column.nullable => Cell::Null,
                    (ValueRef::Text(text), PageKind::Integer) if text.len() <= 20 => {
                        let text = std::str::from_utf8(text).map_err(|_| Error::Bounds)?;
                        let n: i64 = text.parse().map_err(|_| Error::Bounds)?;
                        if crate::scalar::Decimal::new(n).bytes() != text.as_bytes() {
                            return Err(Error::Bounds);
                        }
                        Cell::Integer(n)
                    }
                    (ValueRef::Blob(bytes), PageKind::Blob)
                        if bytes.len() <= column.maximum_bytes as usize =>
                    {
                        Cell::Blob(bytes)
                    }
                    (ValueRef::Text(bytes), PageKind::Text)
                        if bytes.len() <= column.maximum_bytes as usize =>
                    {
                        Cell::Text(std::str::from_utf8(bytes).map_err(|_| Error::Bounds)?)
                    }
                    _ => return Err(Error::Bounds),
                });
            }
            encoded.push(&cells)?;
        }
        encoded.finish()
    }
    fn query(&mut self, statement: Statement, parameters: &[Value]) -> Result<Option<Vec<Value>>> {
        let bindings = self.admit(statement, parameters, false)?;
        let spec = statement.spec();
        let mut query = self
            .connection
            .prepare(spec.sql)
            .map_err(|_| Error::Storage)?;
        let mut rows = query
            .query(rusqlite::params_from_iter(bindings))
            .map_err(|_| Error::Storage)?;
        let Some(row) = rows.next().map_err(|_| Error::Storage)? else {
            return Ok(None);
        };
        let values = scalar::row(row, spec.columns)?;
        if rows.next().map_err(|_| Error::Storage)?.is_some() {
            return Err(Error::Bounds);
        }
        Ok(Some(values))
    }
    fn execute(&mut self, statement: Statement, parameters: &[Value]) -> Result<u64> {
        let bindings = self.admit(statement, parameters, true)?;
        let count = self
            .connection
            .execute(statement.spec().sql, rusqlite::params_from_iter(bindings))
            .map_err(|_| Error::Storage)?;
        Ok(count as u64)
    }
}

#[cfg(test)]
mod sql_budget_tests;

#[cfg(test)]
mod page_decimal_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Charges(Mutex<Vec<usize>>);
    impl foks_snowpack::AllocationBudget for Charges {
        fn reserve(&self, bytes: usize) -> bool {
            self.0.lock().unwrap().push(bytes);
            true
        }
    }
    #[test]
    fn page_decimal_extrema_preserve_bytes_and_original_charge_sequence() {
        use crate::{page::Cell, page_catalog::PageStatement};
        let mut connection = Connection::open_in_memory().unwrap();
        let transaction = connection.transaction().unwrap();
        let mut sql = NativeSql::new(&transaction).unwrap();
        let spec = PageStatement::ProbeTypes.spec();
        let row = spec.maximum_row_bytes().unwrap();
        let first = 62
            + 6 * std::mem::size_of::<SqlValue>()
            + spec.columns.len() * (512 + 2 * std::mem::size_of::<Cell<'_>>())
            + row;
        for integer in [i64::MIN, -1, 0, 1, i64::MAX] {
            let parameters = [
                Value::Integer(1),
                Value::Integer(integer),
                Value::Blob(vec![]),
                Value::Text("é".into()),
                Value::Null,
                Value::Integer(1),
            ];
            let charges = Arc::new(Charges(Mutex::new(Vec::with_capacity(2))));
            let page = foks_snowpack::with_allocation_budget(charges.clone(), || {
                sql.query_page(PageStatement::ProbeTypes, &parameters)
            })
            .unwrap();
            assert!(
                matches!(page.rows().next().unwrap().cells().nth(1), Some(Cell::Integer(value)) if value == integer)
            );
            assert_eq!(*charges.0.lock().unwrap(), [first, 24 + row]);
        }
        assert_eq!(sql.calls(), 5);
    }
}
