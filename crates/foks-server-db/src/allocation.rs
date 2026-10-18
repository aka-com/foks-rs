//! Charge before SQLite values are copied into request-owned Rust results.
use rusqlite::{
    types::{FromSql, ValueRef},
    Row, RowIndex,
};

pub(crate) trait RowAllocationExt {
    fn budgeted_get<I: RowIndex, T: FromSql>(&self, index: I) -> rusqlite::Result<T>;
}
impl RowAllocationExt for Row<'_> {
    fn budgeted_get<I: RowIndex, T: FromSql>(&self, index: I) -> rusqlite::Result<T> {
        let index = index.idx(self.as_ref())?;
        let value = self.get_ref(index)?;
        let length = match value {
            ValueRef::Blob(v) | ValueRef::Text(v) => v.len(),
            _ => 0,
        };
        // Eight overlapping copies cover query results, selected/cloned records,
        // schema values and collection growth. Scalar columns also charge record
        // and Vec/BTreeMap overhead. Codec allocations are charged separately.
        foks_snowpack::reserve_allocation(length.saturating_mul(8).saturating_add(512)).map_err(
            |error| {
                rusqlite::Error::FromSqlConversionFailure(index, value.data_type(), Box::new(error))
            },
        )?;
        self.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Reject;
    impl foks_snowpack::AllocationBudget for Reject {
        fn reserve(&self, _: usize) -> bool {
            false
        }
    }
    struct MustNotMaterialize;
    impl FromSql for MustNotMaterialize {
        fn column_result(_: ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
            panic!("FromSql ran before admission")
        }
    }
    #[test]
    fn rejects_blob_before_from_sql_runs() {
        let database = rusqlite::Connection::open_in_memory().unwrap();
        let result = foks_snowpack::with_allocation_budget(std::sync::Arc::new(Reject), || {
            database.query_row("SELECT zeroblob(1048576)", [], |row| {
                row.budgeted_get::<_, MustNotMaterialize>(0)
            })
        });
        assert!(result.is_err());
        let bytes: Vec<u8> = database
            .query_row("SELECT zeroblob(16)", [], |row| row.budgeted_get(0))
            .unwrap();
        assert_eq!(bytes.len(), 16);
    }
}
