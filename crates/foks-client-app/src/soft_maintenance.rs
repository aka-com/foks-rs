//! Orphaned large-file stage reclamation at exclusive checked-session admission.
use crate::scan_limiter::ScanLimiter;
use crate::*;

// Each pass opens the soft cache and scans every staged large file. Bound
// repeat scanning per database without adding durable state.
static LAST_SCAN: ScanLimiter<PathBuf> = ScanLimiter::new();

/// Deletes large-file stages that no projected KV entry references, which a
/// process exit during a download leaves behind with their plaintext chunks.
///
/// Called only from outermost exclusive session entry, before the session's
/// operation runs. That entry holds `ProfileLock::operation`, which excludes
/// every shared checked session of this profile in any process, so no other
/// `SoftStateStore` can own an in-flight stage of this database. Shared and
/// nested entry must not call it.
pub(crate) fn compact_soft_large_file_stages(session: &CheckedProfileSession<'_>) -> Result<usize> {
    let path = &session.paths.soft_database;
    if !path.try_exists()? {
        return Ok(0);
    }
    if !LAST_SCAN.due(path) {
        return Ok(0);
    }
    let removed = foks_client_db::SoftStateStore::open(path)?.reclaim_orphaned_large_files()?;
    LAST_SCAN.record(path.clone());
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::AccountFixture;
    use foks_client_db::SoftStateStore;
    use rusqlite::Connection;

    // Leaks a store mid-download, as a process exit would: `Drop` never
    // removes the stage this store owns.
    fn leak_stage(session: &ProfileSession, tag: u8) {
        let mut store = SoftStateStore::open(&session.paths.soft_database).unwrap();
        let mut node_id = [tag; 17];
        node_id[0] = 2;
        let mut stage = store.begin_large_file(&[1; 33], &[2; 33], node_id).unwrap();
        store.append_large_file(&mut stage, b"plaintext").unwrap();
        std::mem::forget(store);
    }
    fn staged(session: &ProfileSession) -> (i64, i64) {
        let db = Connection::open(&session.paths.soft_database).unwrap();
        let count = |table: &str| {
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        };
        (count("kv_large_files"), count("kv_large_file_chunks"))
    }
    fn forget_scan(session: &ProfileSession) {
        LAST_SCAN.forget(&session.paths.soft_database);
    }

    #[test]
    fn exclusive_entry_reclaims_leaked_stages_but_shared_and_nested_entry_do_not() {
        let f = AccountFixture::start();
        let session = ProfileSession::open(&f.registry, "local").unwrap();
        leak_stage(&session, 7);
        forget_scan(&session);
        assert_eq!(staged(&session), (1, 1));
        let result = f
            .credentials
            .try_with_shared_checked_session(&session, |_| -> Result<()> {
                f.credentials
                    .with_checked_session(&session, |s| -> Result<()> {
                        assert_eq!(staged(s), (1, 1));
                        Ok(())
                    })
            })
            .unwrap();
        assert!(matches!(result, SharedSessionOutcome::Ran(())));
        assert_eq!(staged(&session), (1, 1));
        f.credentials
            .with_checked_session(&session, |s| -> Result<()> {
                assert_eq!(staged(s), (0, 0));
                Ok(())
            })
            .unwrap();

        leak_stage(&session, 8);
        forget_scan(&session);
        f.credentials
            .try_with_checked_session(&session, |s| -> Result<()> {
                assert_eq!(staged(s), (0, 0));
                Ok(())
            })
            .unwrap()
            .unwrap();
    }

    #[test]
    fn repeated_exclusive_entry_is_rate_limited() {
        let f = AccountFixture::start();
        let session = ProfileSession::open(&f.registry, "local").unwrap();
        leak_stage(&session, 7);
        forget_scan(&session);
        f.run(|s, _, _| {
            assert_eq!(staged(s), (0, 0));
            Ok(())
        });
        leak_stage(&session, 8);
        f.run(|s, _, _| {
            assert_eq!(staged(s), (1, 1));
            Ok(())
        });
        forget_scan(&session);
        f.run(|s, _, _| {
            assert_eq!(staged(s), (0, 0));
            Ok(())
        });
    }
}
