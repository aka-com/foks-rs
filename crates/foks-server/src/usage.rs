//! The `foks-server usage` report: stored usage against the permanent and
//! storage limits, the largest KV namespaces and the teams with the most
//! realtime messages.
//!
//! The report reads one snapshot through a read-only connection, so the
//! server may be running. It scans every KV and realtime message row; its
//! cost grows with the stored data, which is why `/metrics` exports only the
//! limits.

use std::path::Path;

use foks_server_db::CapacityReport;

use crate::installation::hex;
use crate::Result;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageReport {
    pub capacity: CapacityReport,
    /// Size of the write-ahead log file, which the database limit excludes.
    pub wal_bytes: u64,
}

/// Reads the report at `now`, in microseconds.
pub fn usage_report(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
    now: u64,
) -> Result<UsageReport> {
    let database_path = database_path.as_ref();
    if !database_path.exists() {
        return Err(crate::Error::Config(
            "database does not exist; the server creates it on first start",
        ));
    }
    // The pooled open skips the KV tree capacity check, which would read the
    // same rows as the report a second time.
    let database = foks_server_db::ReadDatabase::open_pooled(database_path, database_config)?;
    let capacity = database.snapshot()?.capacity_report(now)?;
    let wal_bytes = crate::installation::wal_bytes(database_path)?;
    Ok(UsageReport {
        capacity,
        wal_bytes,
    })
}

/// One `key=value` line per limit, then the database, KV namespace and
/// realtime team lines.
pub fn report_text(report: &UsageReport) -> String {
    use std::fmt::Write as _;
    let mut output = String::new();
    for usage in &report.capacity.usage {
        let _ = writeln!(
            output,
            "resource={} used={} limit={} remaining={}",
            usage.resource.label(),
            usage.used,
            usage.limit,
            usage.remaining()
        );
    }
    let _ = writeln!(
        output,
        "database_free_bytes={} wal_bytes={}",
        report.capacity.database_free_bytes, report.wal_bytes
    );
    for namespace in &report.capacity.kv_namespaces {
        let _ = writeln!(
            output,
            "kv_namespace id={} bytes={} objects={}",
            hex(&namespace.namespace_id),
            namespace.bytes,
            namespace.objects
        );
    }
    for team in &report.capacity.realtime_teams {
        let _ = writeln!(
            output,
            "realtime_team id={} bytes={} messages={}",
            hex(&team.team_id),
            team.bytes,
            team.messages
        );
    }
    output
}

pub fn report_json(report: &UsageReport) -> serde_json::Value {
    serde_json::json!({
        "capacity": report.capacity.usage.iter().map(|usage| serde_json::json!({
            "resource": usage.resource.label(),
            "used": usage.used,
            "limit": usage.limit,
            "remaining": usage.remaining(),
        })).collect::<Vec<_>>(),
        "database_free_bytes": report.capacity.database_free_bytes,
        "wal_bytes": report.wal_bytes,
        "kv_namespaces": report.capacity.kv_namespaces.iter().map(|namespace| serde_json::json!({
            "namespace_id": hex(&namespace.namespace_id),
            "bytes": namespace.bytes,
            "objects": namespace.objects,
        })).collect::<Vec<_>>(),
        "realtime_teams": report.capacity.realtime_teams.iter().map(|team| serde_json::json!({
            "team_id": hex(&team.team_id),
            "bytes": team.bytes,
            "messages": team.messages,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_reads_a_database_the_writer_holds_open() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("host.sqlite");
        let config = foks_server_db::Config {
            maximum_waitlist_entries: 3,
            ..foks_server_db::Config::default()
        };
        let mut database = foks_server_db::Database::open(&path, config.clone()).unwrap();
        let mut id = [7; 13];
        id[0] = 1;
        database.join_waitlist(&id, "a@example.com", 5).unwrap();
        // The writer stays open: the report must not need writer ownership.
        let _guard = crate::DatabaseWriterGuard::acquire(&path).unwrap();

        let report = usage_report(&path, config, 0).unwrap();
        let text = report_text(&report);
        assert!(text.contains("resource=waitlist_entries used=1 limit=3 remaining=2\n"));
        assert!(text.contains("resource=merkle_epoch used=0 limit=65535 remaining=65535\n"));
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("resource="))
                .count(),
            foks_server_db::CapacityResource::ALL.len()
        );
        let json = report_json(&report);
        assert_eq!(
            json["capacity"][7],
            serde_json::json!({
                "resource": "waitlist_entries",
                "used": 1,
                "limit": 3,
                "remaining": 2,
            })
        );
        assert_eq!(json["kv_namespaces"], serde_json::json!([]));
        assert_eq!(json["wal_bytes"], report.wal_bytes);
        drop(database);
    }
}
