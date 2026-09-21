//! Conservative Merkle collection at exclusive checked-session admission.
use crate::*;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

// Large live histories can remain above the collection threshold indefinitely.
// Bound repeat scanning without adding durable state or affecting correctness.
static LAST_SCAN: OnceLock<Mutex<BTreeMap<[u8; 16], Instant>>> = OnceLock::new();
const SCAN_INTERVAL: Duration = Duration::from_secs(60);

impl ClientCredentials {
    // Called only from outermost exclusive session entry, after durable native
    // manifest admission (when configured). Shared/nested entry must not call it.
    pub(crate) fn compact_session_merkle_roots(
        &self,
        session: &CheckedProfileSession<'_>,
    ) -> Result<usize> {
        if !session.paths.hard_database.try_exists()? {
            return Ok(0);
        }
        let mut hard = HardStateStore::open(&session.paths.hard_database)?;
        if !hard.needs_merkle_compaction()? {
            return Ok(0);
        }
        let identity = hard.metadata()?.database_id;
        let scans = LAST_SCAN.get_or_init(Mutex::default);
        if scans
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&identity)
            .is_some_and(|last| last.elapsed() < SCAN_INTERVAL)
        {
            return Ok(0);
        }
        // Includes protected-only preparations that have not acquired a journal
        // owner yet. Terminal retained files defer too until normal cleanup.
        match fs::read_dir(&session.paths.protected_mutations) {
            Ok(mut entries) => {
                if entries.next().transpose()?.is_some() {
                    return Ok(0);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let checkpoint = registry::checkpoint_for_store(&session.profile, &hard)?;
        let host = checkpoint.host.as_ref();
        if session.paths.credential_store.try_exists()? {
            let master = self.master_key()?;
            let mut store = foks_keystore::EncryptedFileSecretStore::inspect_existing(
                &session.paths.credential_store,
                derive_vault_key(&master),
            )?;
            let keys = store.keys()?;
            let mut vault = AccountVault::new(&mut store);
            for key in keys {
                if !vault
                    .inventory_record(
                        &key,
                        &hard,
                        host.map(|h| h.host_id.as_slice()).unwrap_or(&[]),
                    )?
                    .exportable
                {
                    return Ok(0);
                }
            }
        }
        // Admission has already published this head; retaining it makes a
        // crash before the session's final checkpoint update recoverable.
        let pins = host
            .map(|h| foks_client_db::MerkleRootPin {
                host_id: h.host_id.clone(),
                epoch: h.merkle_epoch,
                root_hash: h.merkle_root_hash,
            })
            .into_iter()
            .collect::<Vec<_>>();
        let removed = hard.compact_merkle_roots(&pins)?;
        let mut scans = scans.lock().unwrap_or_else(|e| e.into_inner());
        if scans.len() >= 128 && !scans.contains_key(&identity) {
            scans.pop_first();
        }
        scans.insert(identity, Instant::now());
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::AccountFixture;
    use rusqlite::{params, Connection};
    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );

    fn seed(session: &ProfileSession) {
        let public = foks_verify::verify_public_host("foks.app", PROBE).unwrap();
        let mut hard = HardStateStore::open(&session.paths.hard_database).unwrap();
        hard.accept_verified_host(&public.snapshot).unwrap();
        let mut db = Connection::open(&session.paths.hard_database).unwrap();
        let tx = db.transaction().unwrap();
        for epoch in 1..300i64 {
            tx.execute(
                "INSERT OR IGNORE INTO merkle_roots VALUES (?1, ?2, ?3, NULL)",
                params![public.snapshot.host_id(), epoch, [42u8; 32].as_slice()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    fn count(session: &ProfileSession) -> i64 {
        Connection::open(&session.paths.hard_database)
            .unwrap()
            .query_row("SELECT count(*) FROM merkle_roots", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn exclusive_entry_collects_but_shared_and_nested_entry_do_not() {
        let f = AccountFixture::start();
        f.run(|s, _, _| {
            seed(s);
            Ok(())
        });
        let session = ProfileSession::open(&f.registry, "local").unwrap();
        let result = f
            .credentials
            .try_with_shared_checked_session(&session, |_| -> Result<()> {
                assert!(count(&session) > 128);
                f.credentials
                    .with_checked_session(&session, |_| -> Result<()> {
                        assert!(count(&session) > 128);
                        Ok(())
                    })
            })
            .unwrap();
        assert!(matches!(result, SharedSessionOutcome::Ran(())));
        f.credentials
            .try_with_checked_session(&session, |s| -> Result<()> {
                assert_eq!(count(s), 2);
                s.pinned_host()?;
                Ok(())
            })
            .unwrap()
            .unwrap();
    }

    #[test]
    fn private_preparation_or_unknown_vault_record_defers_collection() {
        let f = AccountFixture::start();
        let session = ProfileSession::open(&f.registry, "local").unwrap();
        f.run(|s, _, _| {
            seed(s);
            fs::create_dir_all(&s.paths.protected_mutations)?;
            fs::write(s.paths.protected_mutations.join("preparing"), b"opaque")?;
            Ok(())
        });
        f.run(|s, _, _| {
            assert!(count(s) > 128);
            Ok(())
        });
        fs::remove_file(session.paths.protected_mutations.join("preparing")).unwrap();
        let master = f.credentials.master_key().unwrap();
        let mut vault = foks_keystore::EncryptedFileSecretStore::open(
            &session.paths.credential_store,
            derive_vault_key(&master),
        )
        .unwrap();
        vault.put("unknown.preparation", b"opaque").unwrap();
        f.run(|s, _, _| {
            assert!(count(s) > 128);
            Ok(())
        });
        vault.remove("unknown.preparation").unwrap();
        f.run(|s, _, _| {
            assert_eq!(count(s), 2);
            Ok(())
        });
    }

    #[test]
    fn rejected_rollback_admission_does_not_collect() {
        let f = AccountFixture::start_native();
        f.run(|s, _, _| {
            seed(s);
            Ok(())
        });
        let session = ProfileSession::open(&f.registry, "local").unwrap();
        Connection::open(&session.paths.hard_database)
            .unwrap()
            .execute(
                "UPDATE hard_state_metadata SET hard_state_revision=hard_state_revision-1",
                [],
            )
            .unwrap();
        let result = f
            .credentials
            .with_checked_session(&session, |_| -> Result<()> {
                panic!("rollback admission must reject before the callback");
            });
        assert!(result.is_err());
        assert!(count(&session) > 128);
    }

    #[test]
    fn crash_after_collection_before_checkpoint_publication_reopens() {
        let f = AccountFixture::start_native();
        f.run(|s, _, _| {
            seed(s);
            Ok(())
        });
        let session = ProfileSession::open(&f.registry, "local").unwrap();
        let before = session.rollback_checkpoint().unwrap();
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.credentials
                .with_checked_session(&session, |s| -> Result<()> {
                    assert_eq!(count(s), 2);
                    panic!("interrupt before publishing the exit checkpoint");
                })
                .unwrap();
        }));
        assert!(crashed.is_err());
        let after = session.rollback_checkpoint().unwrap();
        assert!(after.hard_state_revision > before.hard_state_revision);
        after.verify_descends_from(&before).unwrap();
        f.run(|s, _, _| {
            s.pinned_host()?;
            assert_eq!(count(s), 2);
            Ok(())
        });
    }
}
