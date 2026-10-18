//! Requested Rust allocations for the native scalar host only. SQLite's own
//! allocations and the caller's input values are outside these cumulative charges.
use crate::{catalog::Kind, Error, Result, Value, MAX_BLOB_BYTES};
use rusqlite::{
    types::{Value as SqlValue, ValueRef},
    Row,
};
use std::mem::size_of;

use crate::scalar::Decimal;

fn add(total: usize, bytes: usize) -> Result<usize> {
    total.checked_add(bytes).ok_or(Error::Bounds)
}
fn container<T>(count: usize) -> Result<usize> {
    count.checked_mul(size_of::<T>()).ok_or(Error::Bounds)
}
fn charge(bytes: usize) -> Result<()> {
    if bytes != 0 {
        crate::page::charge(bytes)?;
    }
    Ok(())
}
fn vector<T>(count: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| Error::Bounds)?;
    Ok(values)
}
fn copy(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut owned = vector(bytes.len())?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}
fn text(bytes: &[u8]) -> Result<String> {
    // Callers have already validated UTF-8 (or produced ASCII decimal bytes).
    // This conversion transfers the exact buffer; it does not allocate again.
    String::from_utf8(copy(bytes)?).map_err(|_| Error::Storage)
}

pub(super) fn bindings(parameters: &[Value]) -> Result<Vec<SqlValue>> {
    let mut bytes = container::<SqlValue>(parameters.len())?;
    for value in parameters {
        bytes = add(
            bytes,
            match value {
                Value::Null => 0,
                Value::Integer(n) => Decimal::new(*n).bytes().len(),
                Value::Text(s) => s.len(),
                Value::Blob(b) => b.len(),
            },
        )?;
    }
    charge(bytes)?;
    let mut bindings = vector(parameters.len())?;
    for value in parameters {
        bindings.push(match value {
            Value::Null => SqlValue::Null,
            Value::Integer(n) => SqlValue::Text(text(Decimal::new(*n).bytes())?),
            Value::Text(s) => SqlValue::Text(text(s.as_bytes())?),
            Value::Blob(b) => SqlValue::Blob(copy(b)?),
        });
    }
    Ok(bindings)
}

enum Borrowed<'a> {
    Null,
    Integer(i64),
    Text(&'a str),
    Blob(&'a [u8]),
}
impl Borrowed<'_> {
    fn bytes(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Blob(b) => b.len(),
            Self::Null | Self::Integer(_) => 0,
        }
    }
    fn copy(self) -> Result<Value> {
        Ok(match self {
            Self::Null => Value::Null,
            Self::Integer(n) => Value::Integer(n),
            Self::Text(s) => Value::Text(text(s.as_bytes())?),
            Self::Blob(b) => Value::Blob(copy(b)?),
        })
    }
}
fn borrowed(value: ValueRef<'_>, kind: Kind) -> Result<Borrowed<'_>> {
    Ok(match (value, kind) {
        (ValueRef::Text(bytes), Kind::Integer) if bytes.len() <= 20 => {
            let text = std::str::from_utf8(bytes).map_err(|_| Error::Storage)?;
            let n: i64 = text.parse().map_err(|_| Error::IntegerRange)?;
            if Decimal::new(n).bytes() != bytes {
                return Err(Error::Storage);
            }
            Borrowed::Integer(n)
        }
        (ValueRef::Blob(b), Kind::Blob | Kind::NullableBlob) if b.len() <= MAX_BLOB_BYTES => {
            Borrowed::Blob(b)
        }
        (ValueRef::Text(b), Kind::Text) if b.len() <= MAX_BLOB_BYTES => {
            Borrowed::Text(std::str::from_utf8(b).map_err(|_| Error::Storage)?)
        }
        (ValueRef::Null, Kind::NullableBlob) => Borrowed::Null,
        _ => return Err(Error::Bounds),
    })
}

pub(super) fn row(row: &Row<'_>, columns: &[Kind]) -> Result<Vec<Value>> {
    // Validate in the original column order and admit all requested storage while
    // SQLite still owns the borrowed row. No intervening SQL or descriptor Vec.
    let mut bytes = container::<Value>(columns.len())?;
    for (index, kind) in columns.iter().enumerate() {
        let value = row.get_ref(index).map_err(|_| Error::Storage)?;
        bytes = add(bytes, borrowed(value, *kind)?.bytes())?;
    }
    charge(bytes)?;
    let mut values = vector(columns.len())?;
    for (index, kind) in columns.iter().enumerate() {
        let value = row.get_ref(index).map_err(|_| Error::Storage)?;
        values.push(borrowed(value, *kind)?.copy()?);
    }
    Ok(values)
}

#[cfg(test)]
mod tests;
