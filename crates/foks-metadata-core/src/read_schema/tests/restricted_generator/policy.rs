use super::descriptor::*;

pub(super) fn declared_type(name: &str) -> Result<StorageType> {
    match name {
        "INTEGER" | "INT" => Ok(StorageType::Integer),
        "REAL" => Ok(StorageType::Real),
        "BLOB" => Ok(StorageType::Blob),
        "TEXT" => Ok(StorageType::Text),
        _ => Err(format!("unsupported declared type {name:?}")),
    }
}
// KV exact_node covers small-file and symlink nodes; directory bytes live in
// kv_directories and follow their separate profile policy.
const OVERRIDES: &[(&str, &str, usize)] = &[
    ("kv_nodes", "exact_node", 65_536),
    ("kv_dirents", "exact_dirent", 4_096),
    ("team_members", "scoped_host_id", 33),
];
pub(super) fn byte_policy(
    component: &str,
    table: &str,
    column: &str,
    kind: &StorageType,
) -> Result<BytePolicy> {
    if component == "foks" {
        if let Some((_, _, maximum)) = OVERRIDES
            .iter()
            .find(|(t, c, _)| *t == table && *c == column)
        {
            if *kind != StorageType::Blob {
                return Err("field override changed type".into());
            }
            return Ok(BytePolicy {
                maximum_bytes: Some(*maximum),
                source: "reviewed_exact_foks_table_column",
            });
        }
    }
    Ok(match kind {
        StorageType::Blob | StorageType::Text => BytePolicy {
            maximum_bytes: Some(VALUE_LIMIT),
            source: "restricted_global_value_limit",
        },
        StorageType::Integer | StorageType::Real => BytePolicy {
            maximum_bytes: None,
            source: "fixed_numeric_payload_at_most_8",
        },
    })
}
pub(super) fn require_overrides(component: &Component) -> Result<()> {
    for (table, column, maximum) in OVERRIDES {
        let matches: Vec<_> = component
            .tables
            .iter()
            .filter(|t| t.name == *table)
            .flat_map(|t| &t.columns)
            .filter(|c| c.name == *column)
            .collect();
        if matches.len() != 1 || matches[0].bytes.maximum_bytes != Some(*maximum) {
            return Err(format!("missing or changed override {table}.{column}"));
        }
    }
    Ok(())
}

// Exact sqlite_schema strings from the reviewed canonical schema 60. Changes in
// spelling/operands/partial predicate require an explicit registry review.
const FILE_SQL: &str = "CREATE INDEX kv_dirent_file_reference\n    ON kv_dirents(uid, substr(node_id, 2, 16))\n    WHERE substr(node_id, 1, 1) = X'02'";
const HOST_SQL: &str = "CREATE UNIQUE INDEX team_members_identity ON team_members(\n    team_id, party_id, coalesce(scoped_host_id, X''), source_role_type, source_visibility\n)";
const ONE_SQL: &str = "CREATE UNIQUE INDEX sso_single_host_policy ON sso_policy((1))";

pub(super) fn expression(
    component: &str,
    table: &str,
    index: &str,
    slot: usize,
    sql: &str,
    columns: &[Column],
) -> Result<Source> {
    if component != "foks" {
        return Err("unregistered expression component".into());
    }
    let column = |name: &str| -> Result<usize> {
        let value = columns
            .iter()
            .find(|c| c.name == name)
            .ok_or("expression source absent")?;
        if value.storage_type != StorageType::Blob {
            return Err("expression source is not BLOB".into());
        }
        Ok(value.ordinal)
    };
    match (table, index, slot, sql) {
        ("kv_dirents", "kv_dirent_file_reference", 1, FILE_SQL) => Ok(Source::NodeFileSlice {
            ordinal: column("node_id")?,
            maximum_bytes: 16,
        }),
        ("team_members", "team_members_identity", 2, HOST_SQL) => {
            let ordinal = column("scoped_host_id")?;
            if columns[ordinal].bytes.maximum_bytes != Some(33) {
                return Err("coalesce source policy differs".into());
            }
            Ok(Source::ScopedHostOrEmpty {
                ordinal,
                maximum_bytes: 33,
            })
        }
        ("sso_policy", "sso_single_host_policy", 0, ONE_SQL) => {
            Ok(Source::IntegerOne { maximum_bytes: 8 })
        }
        _ => Err(format!("unregistered expression {table}.{index}[{slot}]")),
    }
}
pub(super) fn require_expressions(component: &Component) -> Result<()> {
    let found: Vec<_> = component
        .records
        .iter()
        .flat_map(|r| &r.operands)
        .filter_map(|o| match o.source {
            Source::NodeFileSlice { .. } => Some("file"),
            Source::ScopedHostOrEmpty { .. } => Some("host"),
            Source::IntegerOne { .. } => Some("one"),
            _ => None,
        })
        .collect();
    for expected in ["file", "host", "one"] {
        if found.iter().filter(|v| **v == expected).count() != 1 {
            return Err("expression registry coverage differs".into());
        }
    }
    Ok(())
}

// Index operands compare under BINARY unless registered here. A collation
// changes only the order of index entries, not their stored bytes, so the
// record bounds are unaffected. Locators and other row identities stay BINARY.
const COLLATIONS: &[(&str, &str, &str, usize, &str)] = &[(
    "foks",
    "waitlist_entries",
    "waitlist_entries_email",
    0,
    "NOCASE",
)];
pub(super) fn collation(
    component: &str,
    table: &str,
    index: &str,
    slot: usize,
    collation: &str,
    key: bool,
) -> Result<()> {
    if collation == "BINARY"
        || (key && COLLATIONS.contains(&(component, table, index, slot, collation)))
    {
        Ok(())
    } else {
        Err("unsupported index collation".into())
    }
}
pub(super) fn require_collations(component: &Component) -> Result<()> {
    for (registered_component, table, index, slot, collation) in COLLATIONS {
        if *registered_component != component.component {
            continue;
        }
        let found = component
            .tables
            .iter()
            .filter(|t| t.name == *table)
            .flat_map(|t| &t.indexes)
            .filter(|i| i.name == *index)
            .filter_map(|i| component.records.iter().find(|r| r.id == i.record_id))
            .filter(|r| {
                r.operands
                    .get(*slot)
                    .is_some_and(|o| o.collation == *collation)
            })
            .count();
        if found != 1 {
            return Err(format!(
                "collation registry coverage differs for {table}.{index}"
            ));
        }
    }
    Ok(())
}
