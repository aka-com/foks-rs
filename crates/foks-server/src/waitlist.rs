//! Operator configuration and offline access for Reg.joinWaitList entries.
//!
//! The route is unauthenticated and stores email addresses, so it is off by
//! default. When enabled, the table holds one entry per address under ASCII
//! case folding, refuses new addresses at `maximum_entries`, and maintenance
//! deletes entries older than `retention_days`.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
/// Bounds the storage unauthenticated joins can occupy: an entry stores an
/// address of up to 320 bytes in the table and again in its unique index. The
/// cap check reads a trigger-maintained count, so its cost does not depend on
/// this value.
const MAXIMUM_ENTRIES_LIMIT: u64 = 1_000_000;

/// The `[waitlist]` table of an installation configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WaitlistConfig {
    /// Serves Reg.joinWaitList. When false the route answers Unsupported.
    pub enabled: bool,
    /// Total entries, at most 1,000,000. Every join is answered RateLimited
    /// at this count.
    pub maximum_entries: u64,
    /// Age after which maintenance deletes an entry.
    pub retention_days: u64,
}

impl Default for WaitlistConfig {
    fn default() -> Self {
        let database = foks_server_db::Config::default();
        Self {
            enabled: false,
            maximum_entries: database.maximum_waitlist_entries,
            retention_days: database.waitlist_retention.as_secs() / SECONDS_PER_DAY,
        }
    }
}

impl WaitlistConfig {
    pub fn validate(&self) -> Result<()> {
        if self.maximum_entries == 0
            || self.maximum_entries > MAXIMUM_ENTRIES_LIMIT
            || self.retention_days == 0
            || self
                .retention_days
                .checked_mul(SECONDS_PER_DAY * 1_000_000)
                .is_none()
        {
            return Err(Error::Config("invalid waitlist limit"));
        }
        Ok(())
    }

    /// Applies the entry cap and retention to the writer's database limits.
    pub fn database_config(&self, database: foks_server_db::Config) -> foks_server_db::Config {
        foks_server_db::Config {
            maximum_waitlist_entries: self.maximum_entries,
            waitlist_retention: Duration::from_secs(
                self.retention_days.saturating_mul(SECONDS_PER_DAY),
            ),
            ..database
        }
    }
}

/// Every entry, oldest first. Reads a snapshot, so the server may be running.
pub fn list_entries(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
) -> Result<Vec<foks_server_db::WaitlistEntry>> {
    Ok(foks_server_db::ReadDatabase::open(database_path, database_config)?.waitlist_entries()?)
}

/// Writes every entry as a JSON array to a new file readable only by its
/// owner and returns the number of entries written.
pub fn export_entries(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
    output: impl AsRef<Path>,
) -> Result<usize> {
    let entries = list_entries(database_path, database_config)?;
    let mut json = serde_json::to_vec_pretty(&entries_json(&entries))
        .map_err(|_| Error::Config("waitlist export encoding failed"))?;
    json.push(b'\n');
    crate::installation::write_new_file(output.as_ref(), &json, 0o600)?;
    Ok(entries.len())
}

/// Deletes the selected entries while the service is stopped and returns the
/// number deleted.
pub fn purge_entries(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
    selection: foks_server_db::WaitlistPurge<'_>,
) -> Result<u64> {
    let guard = crate::DatabaseWriterGuard::acquire(database_path.as_ref())?;
    Ok(guard
        .open_database(database_config)?
        .purge_waitlist(selection)?)
}

fn entries_json(entries: &[foks_server_db::WaitlistEntry]) -> serde_json::Value {
    serde_json::Value::Array(
        entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "waitlist_id": crate::installation::hex(&entry.waitlist_id),
                    "email": entry.email,
                    "created_at_us": entry.created_at,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_disable_the_route_and_match_the_database_limits() {
        let config = WaitlistConfig::default();
        assert!(!config.enabled);
        config.validate().unwrap();
        let database = config.database_config(foks_server_db::Config::default());
        let defaults = foks_server_db::Config::default();
        assert_eq!(
            database.maximum_waitlist_entries,
            defaults.maximum_waitlist_entries
        );
        assert_eq!(database.waitlist_retention, defaults.waitlist_retention);
        for invalid in [
            WaitlistConfig {
                maximum_entries: 0,
                ..WaitlistConfig::default()
            },
            WaitlistConfig {
                maximum_entries: MAXIMUM_ENTRIES_LIMIT + 1,
                ..WaitlistConfig::default()
            },
            WaitlistConfig {
                retention_days: 0,
                ..WaitlistConfig::default()
            },
            WaitlistConfig {
                retention_days: u64::MAX / SECONDS_PER_DAY,
                ..WaitlistConfig::default()
            },
        ] {
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn offline_purge_requires_writer_ownership_and_export_does_not_overwrite() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("host.sqlite");
        let mut database =
            foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap();
        let mut id = [7; 13];
        id[0] = 1;
        database.join_waitlist(&id, "a@example.com", 5).unwrap();
        drop(database);

        let output = temporary.path().join("waitlist.json");
        assert_eq!(
            export_entries(&path, foks_server_db::Config::default(), &output).unwrap(),
            1
        );
        let exported: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(
            exported,
            serde_json::json!([{
                "waitlist_id": "01070707070707070707070707",
                "email": "a@example.com",
                "created_at_us": 5,
            }])
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&output).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(export_entries(&path, foks_server_db::Config::default(), &output).is_err());

        let guard = crate::DatabaseWriterGuard::acquire(&path).unwrap();
        assert!(purge_entries(
            &path,
            foks_server_db::Config::default(),
            foks_server_db::WaitlistPurge::All
        )
        .is_err());
        drop(guard);
        assert_eq!(
            purge_entries(
                &path,
                foks_server_db::Config::default(),
                foks_server_db::WaitlistPurge::Email("A@example.com")
            )
            .unwrap(),
            1
        );
        assert!(list_entries(&path, foks_server_db::Config::default())
            .unwrap()
            .is_empty());
    }
}
