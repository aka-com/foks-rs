//! Passive, immutable inspection for exclusively reserved state snapshots.
use crate::{Error, HardStateStore, Result};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::path::Path;

impl HardStateStore {
    /// Counts every workflow that prevents a terminal-only archive, including
    /// owners whose protected records have already been legitimately erased.
    pub fn portability_blockers(&self) -> Result<Vec<(&'static str, u64)>> {
        let mut blockers = Vec::new();
        for (workflow, sql) in [
            (
                "import verification",
                "SELECT count(*) FROM import_readiness WHERE required=1",
            ),
            (
                "signup",
                "SELECT count(*) FROM signup_operations WHERE state!=3",
            ),
            (
                "ad-hoc team creation",
                "SELECT count(*) FROM adhoc_team_operations WHERE state!=3",
            ),
            (
                "team mutation",
                "SELECT count(*) FROM team_mutation_operations WHERE state NOT IN (5,6,7)",
            ),
            (
                "mutation",
                "SELECT count(*) FROM mutation_operations WHERE state NOT IN (5,6)",
            ),
            (
                "federation saga",
                "SELECT count(*) FROM federation_saga_operations WHERE state NOT IN (5,6)",
            ),
            (
                "chat send",
                "SELECT count(*) FROM chat_operations WHERE state IN (0,1)",
            ),
            (
                "SSO",
                "SELECT count(*) FROM sso_flows WHERE state IN (0,1,2,3,7)",
            ),
            (
                "MCP submission",
                "SELECT count(*) FROM kv_adapter_submissions WHERE state=0",
            ),
        ] {
            let count: i64 = self.connection.query_row(sql, [], |r| r.get(0))?;
            if count != 0 {
                blockers.push((
                    workflow,
                    crate::stored_unsigned("pending portability workflow", count)?,
                ));
            }
        }
        Ok(blockers)
    }

    /// Opens an existing clean snapshot without initializing, recovering or changing it.
    /// The caller must hold writer exclusion for the entire inspection lifetime.
    pub fn inspect_existing(path: &Path) -> Result<Self> {
        let connection = open_existing(path, crate::APPLICATION_ID, crate::SCHEMA_VERSION, |c| {
            crate::initialize_or_verify(c)
        })?;
        let store = Self { connection };
        store.metadata()?;
        Ok(store)
    }
}

pub(crate) fn open_existing(
    path: &Path,
    application: i64,
    version: u32,
    reference_schema: impl FnOnce(&mut Connection) -> Result<()>,
) -> Result<Connection> {
    let before = inspect_file(path)?;
    if before.len() < 100 || before.len() > 2 * 1024 * 1024 * 1024 {
        return Err(Error::SnapshotInspection(
            "database is empty or exceeds the snapshot limit",
        ));
    }
    inspect_sidecars(path)?;
    let canonical = path.canonicalize()?;
    let mut uri = url::Url::from_file_path(&canonical)
        .map_err(|_| Error::SnapshotInspection("database path cannot be represented"))?;
    uri.query_pairs_mut().append_pair("immutable", "1");
    let connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.pragma_update(None, "trusted_schema", false)?;
    let actual_application: i64 =
        connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let actual_version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if actual_application != application {
        return Err(Error::WrongApplicationId {
            found: actual_application,
            expected: application,
        });
    }
    if actual_version != version {
        return Err(Error::UnsupportedSchema {
            found: actual_version,
            supported: version,
        });
    }
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(Error::SnapshotInspection("database integrity check failed"));
    }
    if connection
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        return Err(Error::SnapshotInspection(
            "database foreign-key check failed",
        ));
    }
    // Version numbers alone do not prove tables, constraints or revision triggers.
    let mut reference = Connection::open_in_memory()?;
    reference_schema(&mut reference)?;
    if schema(&connection)? != schema(&reference)? {
        return Err(Error::SnapshotInspection(
            "database schema or revision triggers differ",
        ));
    }
    let after = inspect_file(path)?;
    if changed(&before, &after) {
        return Err(Error::SnapshotInspection(
            "database changed during inspection",
        ));
    }
    inspect_sidecars(path)?;
    Ok(connection)
}
#[derive(Eq, PartialEq)]
struct SchemaObject {
    kind: String,
    name: String,
    table: String,
    sql: Option<String>,
}
fn schema(c: &Connection) -> Result<Vec<SchemaObject>> {
    let (count, total, name): (i64, i64, i64) = c.query_row(
        "SELECT count(*),coalesce(sum(length(sql)),0),coalesce(max(length(name)),0) FROM sqlite_schema", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if count > 8192 || total > 4 * 1024 * 1024 || name > 512 {
        return Err(Error::SnapshotInspection(
            "database schema exceeds snapshot bounds",
        ));
    }
    Ok(c.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name")?
        .query_map([], |r| Ok(SchemaObject{kind:r.get(0)?,name:r.get(1)?,table:r.get(2)?,sql:r.get(3)?}))?
        .collect::<rusqlite::Result<_>>()?)
}
fn inspect_file(path: &Path) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(Error::SnapshotInspection(
            "database or sidecar is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if m.nlink() != 1 {
            return Err(Error::SnapshotInspection(
                "database or sidecar has multiple hard links",
            ));
        }
    }
    Ok(m)
}
fn changed(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return true;
        }
    }
    before.len() != after.len() || before.modified().ok() != after.modified().ok()
}
fn inspect_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-journal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = Path::new(&name);
        match fs::symlink_metadata(sidecar) {
            Ok(_) => {
                let metadata = inspect_file(sidecar)?;
                if suffix != "-shm" && metadata.len() != 0 {
                    return Err(Error::SnapshotInspection("database has a nonempty WAL or journal; recover and checkpoint normally before retrying"));
                }
                if suffix == "-shm" && metadata.len() > 1024 * 1024 {
                    return Err(Error::SnapshotInspection(
                        "database shared-memory sidecar exceeds the limit",
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sqlite_like_prefix_does_not_hide_user_schema_objects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite3");
        drop(HardStateStore::open(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE sqliteUserTable (value TEXT);")
            .unwrap();
        drop(connection);
        assert!(matches!(
            HardStateStore::inspect_existing(&path),
            Err(Error::SnapshotInspection(
                "database schema or revision triggers differ"
            ))
        ));
    }
    #[test]
    fn inspection_does_not_create_initialize_recover_or_chmod() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard.sqlite3");
        assert!(HardStateStore::inspect_existing(&path).is_err());
        assert!(!path.exists());
        fs::write(&path, []).unwrap();
        assert!(HardStateStore::inspect_existing(&path).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        fs::remove_file(&path).unwrap();
        drop(HardStateStore::open(&path).unwrap());
        let before = fs::read(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions();
        let db = HardStateStore::inspect_existing(&path).unwrap();
        db.metadata().unwrap();
        drop(db);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().permissions(), mode);
        assert!(!dir.path().join("hard.sqlite3-wal").exists());
        fs::write(dir.path().join("hard.sqlite3-wal"), [1]).unwrap();
        assert!(HardStateStore::inspect_existing(&path).is_err());
        assert_eq!(fs::read(dir.path().join("hard.sqlite3-wal")).unwrap(), [1]);
    }
    #[test]
    fn schema_tampering_and_hardlinks_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard.sqlite3");
        drop(HardStateStore::open(&path).unwrap());
        let duplicate = dir.path().join("copy");
        fs::hard_link(&path, &duplicate).unwrap();
        assert!(HardStateStore::inspect_existing(&path).is_err());
        fs::remove_file(duplicate).unwrap();
        let c = Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER hard_state_revision_hosts_insert")
            .unwrap();
        drop(c);
        assert!(matches!(
            HardStateStore::inspect_existing(&path),
            Err(Error::SnapshotInspection(
                "database schema or revision triggers differ"
            ))
        ));
    }
    #[test]
    fn empty_sidecars_and_verified_soft_state_are_passive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("soft.sqlite3");
        drop(crate::SoftStateStore::open(&path).unwrap());
        fs::write(dir.path().join("soft.sqlite3-wal"), []).unwrap();
        let before = fs::read(&path).unwrap();
        drop(crate::SoftStateStore::inspect_existing(&path).unwrap());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::metadata(dir.path().join("soft.sqlite3-wal"))
                .unwrap()
                .len(),
            0
        );
    }
}
