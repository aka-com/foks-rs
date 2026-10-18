//! Exact logical rows, including the hidden rowid. No source SQL is executed.
use super::*;
use foks_schema_contract::storage::{self, Dimension, OperandSource, TableId};
use rusqlite::types::{Value, ValueRef};

pub(super) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn owned(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => Value::Integer(v),
        ValueRef::Real(v) if v.is_finite() => Value::Real(v),
        ValueRef::Real(_) => return Err(unsupported("nonfinite imported REAL")),
        ValueRef::Blob(v) => {
            let mut out = Vec::new();
            out.try_reserve_exact(v.len())
                .map_err(|_| ScanError::Allocation)?;
            out.extend_from_slice(v);
            Value::Blob(out)
        }
        ValueRef::Text(v) => {
            let text = std::str::from_utf8(v).map_err(|_| unsupported("invalid imported UTF8"))?;
            let mut out = String::new();
            out.try_reserve_exact(text.len())
                .map_err(|_| ScanError::Allocation)?;
            out.push_str(text);
            Value::Text(out)
        }
    })
}
fn dimension(value: ValueRef<'_>) -> Dimension {
    match value {
        ValueRef::Null => Dimension::Null,
        ValueRef::Integer(_) => Dimension::Integer,
        ValueRef::Real(_) => Dimension::Real,
        ValueRef::Blob(v) => Dimension::Blob {
            bytes: v.len() as u64,
        },
        ValueRef::Text(v) => Dimension::TextUtf8 {
            bytes: v.len() as u64,
        },
    }
}
fn key_columns(table: TableId) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    if let Some(keys) = table.locator().primary_key() {
        for key in keys {
            let OperandSource::Column { ordinal } = key.source() else {
                return Err(unsupported("import key source"));
            };
            if key.descending() || key.collation() != "BINARY" {
                return Err(unsupported("import key ordering"));
            }
            out.push(usize::from(ordinal));
        }
    }
    Ok(out)
}
struct Row {
    values: Vec<Value>,
    key: Vec<Value>,
}
fn read(db: &scan::Database, table: TableId, previous: &[Value]) -> Result<Option<Row>> {
    let columns = table.columns().map(|c| quote(c.name())).collect::<Vec<_>>();
    let keys = key_columns(table)?;
    let rowid = table.locator().rowid_spelling().map(quote);
    let order = match &rowid {
        Some(v) => v.clone(),
        None => keys
            .iter()
            .map(|i| columns[*i].clone())
            .collect::<Vec<_>>()
            .join(","),
    };
    let predicate = if previous.is_empty() {
        String::new()
    } else {
        let parameters = (1..=previous.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(" WHERE ({order})>({parameters})")
    };
    let mut projection = columns.join(",");
    if let Some(id) = &rowid {
        projection.push(',');
        projection.push_str(id);
    }
    let sql = format!(
        "SELECT {projection} FROM main.{}{predicate} ORDER BY {order} LIMIT 1",
        quote(table.name())
    );
    db.control().statement()?;
    let mut stmt = db.connection().prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(previous))?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let n = columns.len();
    if n > 32 {
        return Err(unsupported("import compiled column bound"));
    }
    let mut dimensions = [Dimension::Null; 32];
    for (i, value) in dimensions[..n].iter_mut().enumerate() {
        *value = dimension(row.get_ref(i)?);
    }
    storage::check_utf8_row_dimensions(table, &dimensions[..n])
        .map_err(|violation| ScanError::Row { table, violation })?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(n + 1)
        .map_err(|_| ScanError::Allocation)?;
    for i in 0..n {
        values.push(owned(row.get_ref(i)?)?);
    }
    let mut key = Vec::new();
    key.try_reserve_exact(keys.len().max(1))
        .map_err(|_| ScanError::Allocation)?;
    if rowid.is_some() {
        key.push(Value::Integer(row.get(n)?));
    } else {
        let mut bytes = 0usize;
        for ordinal in keys {
            let value = row.get_ref(ordinal)?;
            let ValueRef::Blob(v) = value else {
                return Err(unsupported("import WR key type"));
            };
            bytes = bytes.checked_add(v.len()).ok_or(ScanError::Allocation)?;
            if bytes > 2 * 1024 * 1024 {
                return Err(unsupported("import WR key bytes"));
            }
            key.push(owned(value)?);
        }
    }
    if key == previous {
        return Err(unsupported("import key did not advance"));
    }
    db.control().check()?;
    Ok(Some(Row { values, key }))
}
pub(super) fn copy_all(source: &scan::Database, target: &scan::Database) -> Result<u64> {
    let mut count = 0u64;
    for table in HOSTED_SMALL_V1.descriptor(Component::Foks).tables() {
        let mut columns = table.columns().map(|c| quote(c.name())).collect::<Vec<_>>();
        let alias = table.columns().any(|c| c.integer_rowid_alias());
        let extra = table.locator().rowid_spelling().filter(|_| !alias);
        if let Some(name) = extra {
            columns.push(quote(name));
        }
        let parameters = (1..=columns.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "INSERT INTO main.{}({}) VALUES({parameters})",
            quote(table.name()),
            columns.join(",")
        );
        let mut previous = Vec::new();
        while let Some(mut row) = read(source, table, &previous)? {
            if extra.is_some() {
                let Value::Integer(id) = row.key[0] else {
                    return Err(unsupported("import rowid type"));
                };
                row.values.push(Value::Integer(id));
            }
            target.control().statement()?;
            if target
                .connection()
                .execute(&sql, rusqlite::params_from_iter(&row.values))?
                != 1
            {
                return Err(unsupported("import insert cardinality"));
            }
            previous = row.key;
            count = count
                .checked_add(1)
                .ok_or(unsupported("import row count"))?;
        }
    }
    Ok(count)
}
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Real(a), Value::Real(b)) => a.to_bits() == b.to_bits(),
        _ => a == b,
    }
}
pub(super) fn compare(
    source: &scan::Database,
    target: &scan::Database,
    coordinate: Option<&foks_server_db::ReadDependency>,
) -> Result<u64> {
    let mut count = 0u64;
    for table in HOSTED_SMALL_V1.descriptor(Component::Foks).tables() {
        let mut previous = Vec::new();
        loop {
            let a = read(source, table, &previous)?;
            let b = read(target, table, &previous)?;
            match (a, b) {
                (None, None) => break,
                (Some(a), Some(b)) => {
                    if a.key != b.key || a.values.len() != b.values.len() {
                        return Err(unsupported("import row identity mismatch"));
                    }
                    for (i, (a, b)) in a.values.iter().zip(&b.values).enumerate() {
                        if table.name() == "metadata_read_dependency" {
                            if let Some(expected) = coordinate {
                                match table.columns().nth(i).map(|c| c.name()) {
                                    Some("incarnation") => {
                                        if !matches!(b, Value::Blob(value) if value.as_slice() == expected.incarnation)
                                        {
                                            return Err(unsupported(
                                                "fresh import incarnation mismatch",
                                            ));
                                        }
                                        continue;
                                    }
                                    Some("revision") => {
                                        if b != &Value::Integer(1) {
                                            return Err(unsupported(
                                                "fresh import revision mismatch",
                                            ));
                                        }
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if !same(a, b) {
                            return Err(unsupported("import value or storage class mismatch"));
                        }
                    }
                    previous = a.key;
                    count = count
                        .checked_add(1)
                        .ok_or(unsupported("import comparison count"))?;
                }
                _ => return Err(unsupported("import row count mismatch")),
            }
        }
    }
    Ok(count)
}
pub(super) fn sequence(target: &scan::Database, expected: Option<i64>) -> Result<()> {
    target.control().statement()?;
    target
        .connection()
        .execute("DELETE FROM main.sqlite_sequence", [])?;
    if let Some(value) = expected {
        target.control().statement()?;
        target.connection().execute(
            "INSERT INTO main.sqlite_sequence(name,seq) VALUES('admin_audit',?1)",
            [value],
        )?;
    }
    verify_sequence(target, expected)
}
pub(super) fn verify_sequence(target: &scan::Database, expected: Option<i64>) -> Result<()> {
    target.control().statement()?;
    let mut stmt = target
        .connection()
        .prepare("SELECT name,seq FROM main.sqlite_sequence LIMIT 2")?;
    let mut rows = stmt.query([])?;
    match (rows.next()?, expected) {
        (None, None) => {}
        (Some(row), Some(seq))
            if row.get_ref(0)? == ValueRef::Text(b"admin_audit")
                && row.get::<_, i64>(1)? == seq => {}
        _ => return Err(unsupported("import sequence mismatch")),
    }
    if rows.next()?.is_some() {
        return Err(unsupported("extra import sequence"));
    }
    Ok(())
}

#[cfg(test)]
mod numeric_tests {
    use super::*;
    #[test]
    fn sqlite_real_readback_policy_keeps_types_and_rejects_nonfinite() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE r(value REAL)").unwrap();
        db.execute("INSERT INTO r VALUES(?1)", [f64::NAN]).unwrap();
        let mut s = db.prepare("SELECT value FROM r").unwrap();
        let mut rows = s.query([]).unwrap();
        assert!(matches!(
            owned(rows.next().unwrap().unwrap().get_ref(0).unwrap()).unwrap(),
            Value::Null
        ));
        drop(rows);
        drop(s);
        assert!(owned(ValueRef::Real(f64::INFINITY)).is_err());
        assert!(owned(ValueRef::Real(f64::NEG_INFINITY)).is_err());
        assert!(!same(&Value::Real(-0.0), &Value::Real(0.0)));
        assert!(!same(&Value::Integer(1), &Value::Real(1.0)));
        db.execute("DELETE FROM r", []).unwrap();
        db.execute("INSERT INTO r VALUES(?1)", [1.5f64]).unwrap();
        let observed: Value = db
            .query_row("SELECT value FROM r", [], |r| r.get(0))
            .unwrap();
        assert!(same(&Value::Real(1.5), &observed));
    }
}
