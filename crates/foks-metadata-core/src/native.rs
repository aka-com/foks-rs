//! Synchronous host for an already-open native transaction. It does not commit.
use crate::{catalog::Kind, Error, Result, Sql, Statement, Value, MAX_BLOB_BYTES, MAX_SQL_CALLS};
use rusqlite::{
    types::{Value as SqlValue, ValueRef},
    Connection,
};

pub struct NativeSql<'a> {
    connection: &'a Connection,
    calls: usize,
}
impl<'a> NativeSql<'a> {
    pub fn new(connection: &'a Connection) -> Result<Self> {
        if connection.is_autocommit() {
            return Err(Error::Storage);
        }
        Ok(Self {
            connection,
            calls: 0,
        })
    }
    pub fn calls(&self) -> usize {
        self.calls
    }
    fn admit(
        &mut self,
        statement: Statement,
        parameters: &[Value],
        write: bool,
    ) -> Result<Vec<SqlValue>> {
        statement.validate_parameters(parameters)?;
        self.calls += 1;
        if self.calls > MAX_SQL_CALLS || statement.spec().write != write {
            return Err(Error::Bounds);
        }
        Ok(parameters
            .iter()
            .map(|v| match v {
                Value::Null => SqlValue::Null,
                Value::Integer(n) => SqlValue::Text(n.to_string()),
                Value::Text(s) => SqlValue::Text(s.clone()),
                Value::Blob(b) => SqlValue::Blob(b.clone()),
            })
            .collect())
    }
}
impl Sql for NativeSql<'_> {
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
        let mut values = Vec::with_capacity(spec.columns.len());
        for (index, kind) in spec.columns.iter().enumerate() {
            let value = row.get_ref(index).map_err(|_| Error::Storage)?;
            values.push(match (value, kind) {
                (ValueRef::Text(text), Kind::Integer) if text.len() <= 20 => {
                    let text = std::str::from_utf8(text).map_err(|_| Error::Storage)?;
                    let n: i64 = text.parse().map_err(|_| Error::IntegerRange)?;
                    if n.to_string() != text {
                        return Err(Error::Storage);
                    }
                    Value::Integer(n)
                }
                (ValueRef::Blob(bytes), Kind::Blob | Kind::NullableBlob)
                    if bytes.len() <= MAX_BLOB_BYTES =>
                {
                    Value::Blob(bytes.to_vec())
                }
                (ValueRef::Text(bytes), Kind::Text) if bytes.len() <= MAX_BLOB_BYTES => {
                    Value::Text(
                        std::str::from_utf8(bytes)
                            .map_err(|_| Error::Storage)?
                            .to_owned(),
                    )
                }
                (ValueRef::Null, Kind::NullableBlob) => Value::Null,
                _ => return Err(Error::Bounds),
            });
        }
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
