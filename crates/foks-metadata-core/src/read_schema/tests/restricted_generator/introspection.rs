use super::{descriptor::*, hash, policy};
use rusqlite::{Connection, Row};
use std::collections::BTreeSet;

fn db<T>(value: rusqlite::Result<T>) -> Result<T> {
    value.map_err(|e| e.to_string())
}
fn text(row: &Row<'_>, index: usize, maximum: usize) -> Result<String> {
    let value = db(row.get_ref(index))?;
    let value = value.as_str().map_err(|e| e.to_string())?;
    if value.len() > maximum {
        return Err("metadata text bound".into());
    }
    Ok(value.to_owned())
}
fn integer(row: &Row<'_>, index: usize) -> Result<i64> {
    db(row.get(index))
}
fn bit(row: &Row<'_>, index: usize) -> Result<bool> {
    match integer(row, index)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err("metadata boolean".into()),
    }
}
fn ordinal(value: i64) -> Result<usize> {
    usize::try_from(value).map_err(|_| "negative metadata ordinal".into())
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn ddl(connection: &Connection, kind: &str, name: &str) -> Result<Option<String>> {
    let mut statement =
        db(connection.prepare("SELECT sql FROM main.sqlite_schema WHERE type=?1 AND name=?2"))?;
    let mut rows = db(statement.query([kind, name]))?;
    let row = db(rows.next())?.ok_or("missing schema object")?;
    let sql = if matches!(db(row.get_ref(0))?, rusqlite::types::ValueRef::Null) {
        None
    } else {
        Some(text(row, 0, 65_536)?)
    };
    if db(rows.next())?.is_some() {
        return Err("duplicate schema object".into());
    }
    Ok(sql)
}
struct RawIndex {
    name: String,
    unique: bool,
    origin: String,
    partial: bool,
}
fn raw_indexes(connection: &Connection, table: &str) -> Result<Vec<RawIndex>> {
    let mut statement =
        db(connection.prepare(&format!("PRAGMA main.index_list({})", quote(table))))?;
    let mut rows = db(statement.query([]))?;
    let mut output = Vec::new();
    while let Some(row) = db(rows.next())? {
        if output.len() >= MAX_RECORDS {
            return Err("index count bound".into());
        }
        let origin = text(row, 3, 2)?;
        if !matches!(origin.as_str(), "c" | "pk" | "u") {
            return Err("unknown index origin".into());
        }
        output.push(RawIndex {
            name: text(row, 1, 256)?,
            unique: bit(row, 2)?,
            origin,
            partial: bit(row, 4)?,
        });
    }
    output.sort_by(|a, b| a.name.cmp(&b.name));
    if output.windows(2).any(|w| w[0].name == w[1].name) {
        return Err("duplicate index".into());
    }
    Ok(output)
}
fn columns(connection: &Connection, component: &str, table: &str) -> Result<Vec<Column>> {
    let mut statement =
        db(connection.prepare(&format!("PRAGMA main.table_xinfo({})", quote(table))))?;
    let mut rows = db(statement.query([]))?;
    let mut output = Vec::new();
    while let Some(row) = db(rows.next())? {
        if output.len() >= MAX_FIELDS {
            return Err("column count bound".into());
        }
        let cid = ordinal(integer(row, 0)?)?;
        if cid != output.len() || integer(row, 6)? != 0 {
            return Err("hidden/generated/nonordinal column".into());
        }
        let name = text(row, 1, 256)?;
        let declared_type = text(row, 2, 16)?.to_ascii_uppercase();
        let storage_type = policy::declared_type(&declared_type)?;
        let explicit_not_null = bit(row, 3)?;
        let bytes = policy::byte_policy(component, table, &name, &storage_type)?;
        output.push(Column {
            ordinal: cid,
            name,
            declared_type,
            storage_type,
            nullable: !explicit_not_null,
            explicit_not_null,
            primary_key_ordinal: ordinal(integer(row, 5)?)?,
            integer_rowid_alias: false,
            bytes,
        });
    }
    if output.is_empty() {
        return Err("empty table".into());
    }
    Ok(output)
}
fn operands(
    connection: &Connection,
    component: &str,
    table: &str,
    index: &RawIndex,
    sql: Option<&str>,
    columns: &[Column],
    without_rowid: bool,
) -> Result<Vec<Operand>> {
    let mut statement =
        db(connection.prepare(&format!("PRAGMA main.index_xinfo({})", quote(&index.name))))?;
    let mut rows = db(statement.query([]))?;
    let mut output = Vec::new();
    while let Some(row) = db(rows.next())? {
        if output.len() >= MAX_FIELDS || ordinal(integer(row, 0)?)? != output.len() {
            return Err("index field bound/order".into());
        }
        let slot = output.len();
        let cid = integer(row, 1)?;
        let collation = text(row, 4, 256)?;
        if collation != "BINARY" {
            return Err("unsupported index collation".into());
        }
        let key = bit(row, 5)?;
        let source = match cid {
            -1 if !without_rowid && !key => Source::Rowid,
            -2 if key => policy::expression(
                component,
                table,
                &index.name,
                slot,
                sql.ok_or("expression has no DDL")?,
                columns,
            )?,
            n if n >= 0 => {
                let ordinal = ordinal(n)?;
                let column = columns.get(ordinal).ok_or("index column out of range")?;
                if text(row, 2, 256)? != column.name {
                    return Err("index column identity differs".into());
                }
                Source::Column { ordinal }
            }
            _ => return Err("unexplained index operand".into()),
        };
        output.push(Operand {
            source,
            descending: bit(row, 3)?,
            collation,
            key,
        });
    }
    if output.is_empty() {
        return Err("empty index".into());
    }
    Ok(output)
}
fn table(
    connection: &Connection,
    component: &str,
    name: String,
    strict: bool,
    without_rowid: bool,
) -> Result<(Table, Vec<Record>)> {
    let sql = ddl(connection, "table", &name)?.ok_or("table DDL absent")?;
    let mut columns = columns(connection, component, &name)?;
    let raw = raw_indexes(connection, &name)?;
    let primary_indexes = raw.iter().filter(|i| i.origin == "pk").count();
    let primary_columns: Vec<_> = columns
        .iter()
        .filter(|c| c.primary_key_ordinal > 0)
        .map(|c| c.ordinal)
        .collect();
    if primary_indexes > 1 || (without_rowid && primary_indexes != 1) {
        return Err("primary index shape".into());
    }
    let alias = if !without_rowid
        && primary_indexes == 0
        && primary_columns.len() == 1
        && columns[primary_columns[0]].declared_type == "INTEGER"
    {
        Some(primary_columns[0])
    } else {
        None
    };
    for column in &mut columns {
        column.integer_rowid_alias = alias == Some(column.ordinal);
        column.nullable = !(column.explicit_not_null
            || column.integer_rowid_alias
            || ((strict || without_rowid) && column.primary_key_ordinal > 0));
    }
    let record_id = format!("{component}/table/{name}");
    let mut table_operands: Option<Vec<Operand>> = None;
    let mut indexes = Vec::new();
    let mut records = Vec::new();
    for index in raw {
        let table_btree_alias = without_rowid && index.origin == "pk";
        // WITHOUT ROWID primary-key entries are virtual index_list aliases:
        // unlike ordinary automatic indexes, they have no sqlite_schema row.
        let sql = if table_btree_alias {
            let count: i64 = db(connection.query_row(
                "SELECT count(*) FROM main.sqlite_schema WHERE name=?1",
                [&index.name],
                |row| row.get(0),
            ))?;
            if count != 0 {
                return Err("WITHOUT ROWID PK alias unexpectedly has a schema row".into());
            }
            None
        } else {
            ddl(connection, "index", &index.name)?
        };
        if (index.origin == "c") != sql.is_some() {
            return Err("index DDL origin differs".into());
        }
        let fields = operands(
            connection,
            component,
            &name,
            &index,
            sql.as_deref(),
            &columns,
            without_rowid,
        )?;
        let id = if table_btree_alias {
            record_id.clone()
        } else {
            format!("{component}/index/{}", index.name)
        };
        if table_btree_alias {
            if !index.unique || index.partial {
                return Err("table PK index flags".into());
            }
            let ids: BTreeSet<_> = fields
                .iter()
                .filter_map(|o| match o.source {
                    Source::Column { ordinal } => Some(ordinal),
                    _ => None,
                })
                .collect();
            if ids.len() != columns.len()
                || fields.len() != columns.len()
                || !ids.iter().copied().eq(0..columns.len())
            {
                return Err("WITHOUT ROWID physical columns differ".into());
            }
            table_operands = Some(fields);
        } else {
            records.push(Record {
                id: id.clone(),
                table: name.clone(),
                kind: "index",
                operands: fields,
            });
        }
        indexes.push(Index {
            name: index.name,
            origin: index.origin,
            unique: index.unique,
            partial: index.partial,
            partial_policy: "check_record_bound_for_every_row",
            sql_sha256: sql.as_deref().map(|s| hash(s.as_bytes())),
            record_id: id,
            table_btree_alias,
        });
    }
    let (fields, locator) = if without_rowid {
        let fields = table_operands.ok_or("WITHOUT ROWID table record absent")?;
        let keys: Vec<_> = fields.iter().filter(|o| o.key).cloned().collect();
        if keys.len() != primary_columns.len() {
            return Err("primary locator shape differs".into());
        }
        (fields, Locator::PrimaryKey { operands: keys })
    } else {
        let spelling = ["_rowid_", "rowid", "oid"]
            .into_iter()
            .find(|candidate| {
                !columns
                    .iter()
                    .any(|c| c.name.eq_ignore_ascii_case(candidate))
            })
            .ok_or("all hidden rowid spellings shadowed")?;
        let fields = columns
            .iter()
            .map(|c| Operand {
                source: Source::Column { ordinal: c.ordinal },
                descending: false,
                collation: "BINARY".into(),
                key: false,
            })
            .collect();
        (
            fields,
            Locator::Rowid {
                spelling: spelling.into(),
            },
        )
    };
    records.push(Record {
        id: record_id.clone(),
        table: name.clone(),
        kind: "table",
        operands: fields,
    });
    Ok((
        Table {
            name,
            sql_sha256: hash(sql.as_bytes()),
            strict,
            without_rowid,
            columns,
            record_id,
            locator,
            indexes,
        },
        records,
    ))
}
pub(super) fn ensure_namespaces(connection: &Connection) -> Result<()> {
    let mut statement = db(connection.prepare("PRAGMA database_list"))?;
    let mut rows = db(statement.query([]))?;
    let mut main = false;
    let mut count = 0;
    while let Some(row) = db(rows.next())? {
        count += 1;
        if count > 2 {
            return Err("unexpected attached namespace".into());
        }
        match text(row, 1, 256)?.as_str() {
            "main" if !main => main = true,
            "temp" => {}
            _ => return Err("unexpected attached namespace".into()),
        }
    }
    if !main {
        return Err("main namespace absent".into());
    }
    let temp: i64 = db(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema)",
        [],
        |row| row.get(0),
    ))?;
    if temp != 0 {
        return Err("unexpected TEMP objects".into());
    }
    Ok(())
}
pub(super) fn component(connection: &Connection, name: &str) -> Result<Component> {
    ensure_namespaces(connection)?;
    let mut statement = db(connection.prepare("PRAGMA main.table_list"))?;
    let mut rows = db(statement.query([]))?;
    let mut raw = Vec::new();
    while let Some(row) = db(rows.next())? {
        if text(row, 0, 256)? != "main" {
            return Err("wrong schema".into());
        }
        let name = text(row, 1, 256)?;
        if matches!(name.as_str(), "sqlite_schema" | "sqlite_sequence") {
            continue;
        }
        if name.to_ascii_lowercase().starts_with("sqlite_") {
            return Err("unclassified SQLite-owned table".into());
        }
        if raw.len() >= MAX_TABLES || text(row, 2, 16)? != "table" {
            return Err("unsupported table kind/count".into());
        }
        if ordinal(integer(row, 3)?)? > MAX_FIELDS {
            return Err("table width bound".into());
        }
        raw.push((name, bit(row, 5)?, bit(row, 4)?));
    }
    drop(rows);
    drop(statement);
    raw.sort_by(|a, b| a.0.cmp(&b.0));
    if raw.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err("duplicate table".into());
    }
    let mut tables = Vec::new();
    let mut records = Vec::new();
    for (table_name, strict, wr) in raw {
        let (table, mut physical) = table(connection, name, table_name, strict, wr)?;
        if records
            .len()
            .checked_add(physical.len())
            .ok_or("record count overflow")?
            > MAX_RECORDS
        {
            return Err("record count bound".into());
        }
        tables.push(table);
        records.append(&mut physical);
    }
    records.sort_by(|a, b| a.id.cmp(&b.id));
    if records.windows(2).any(|w| w[0].id == w[1].id) {
        return Err("duplicate record".into());
    }
    Ok(Component {
        format_version: 1,
        component: name.into(),
        scope: "trusted_descriptors_only",
        value_limit: VALUE_LIMIT,
        record_limit: RECORD_LIMIT,
        record_formula: "9 + 9 * operand_count + sum(payload_upper_bounds)",
        tables,
        records,
    })
}
