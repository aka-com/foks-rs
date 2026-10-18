//! Bounded local upload erasure. Ownership and the spool reservation survive
//! cancellation because the checked synchronous worker owns both until return.
use crate::*;
use foks_client_db::{UploadCleanupPhase, UploadState};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

#[derive(Default)]
pub(crate) struct UploadMaintenanceCursor {
    database_id: Option<[u8; 16]>,
    after: Option<[u8; 16]>,
    seek: Option<CleanupSeek>,
    unstamped: Option<[u8; 16]>,
    account: Option<(Vec<u8>, Vec<u8>)>,
    full_accounts: BTreeSet<(Vec<u8>, Vec<u8>)>,
    blocked: BTreeMap<[u8; 16], Instant>,
    clock_blocked: BTreeMap<(Vec<u8>, Vec<u8>), (Instant, bool)>,
    burst: u8,
    active: Option<[u8; 16]>,
    temporary: Option<([u8; 16], foks_client::ProtectedTemporaryScan)>,
}

impl CheckedProfileSession<'_> {
    /// Exact local debt for explicit drain/status; scans bounded retained receipts.
    pub fn upload_cleanup_debt(&self) -> Result<foks_client_db::UploadCleanupDebt> {
        Ok(HardStateStore::open(&self.paths.hard_database)?.upload_cleanup_debt()?)
    }

    #[cfg(test)]
    pub(crate) fn maintain_upload_state(
        &self,
        cursor: &mut UploadMaintenanceCursor,
        master: &[u8; 32],
    ) -> Result<AdapterMaintenanceReport> {
        self.maintain_upload_state_until(cursor, master, Instant::now() + Duration::from_millis(50))
    }

    pub(crate) fn maintain_upload_state_until(
        &self,
        cursor: &mut UploadMaintenanceCursor,
        master: &[u8; 32],
        deadline: Instant,
    ) -> Result<AdapterMaintenanceReport> {
        let mut report = AdapterMaintenanceReport::default();
        let mut hard = HardStateStore::open(&self.paths.hard_database)?;
        let database_id = hard.metadata()?.database_id;
        cursor.blocked.retain(|_, until| *until > Instant::now());
        if cursor.database_id != Some(database_id) {
            *cursor = UploadMaintenanceCursor {
                database_id: Some(database_id),
                ..Default::default()
            };
        }
        if let Some((owner, _)) = cursor.temporary.as_ref() {
            if hard
                .upload_cleanup(owner)?
                .is_none_or(|p| p.phase != UploadCleanupPhase::Temporaries)
            {
                cursor.temporary = None;
            }
        }
        // Clock work has its own account backoff, independent of eligible erasure.
        self.maintain_upload_receipt_time(&mut hard, cursor, deadline, &mut report)?;
        if Instant::now() >= deadline {
            report.more_work = true;
            return Ok(report);
        }
        let mut protected = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master),
        )?;
        let id = if let Some(id) = cursor.active {
            id
        } else {
            let selection = runnable_cleanup(
                &hard,
                cursor.seek.take().unwrap_or(CleanupSeek {
                    after: cursor.after,
                    ..Default::default()
                }),
                &cursor.blocked,
                deadline,
                131,
            )?;
            report.more_work |= selection.continuation.is_some();
            cursor.seek = selection.continuation;
            let Some(summary) = selection.candidate else {
                cursor.after = cursor.seek.as_ref().and_then(|seek| seek.after);
                report.blocked |= !cursor.blocked.is_empty();
                return Ok(report);
            };
            cursor.after = Some(summary.internal_id);
            cursor.blocked.remove(&summary.internal_id);
            cursor.burst = 0;
            cursor.active = Some(summary.internal_id);
            summary.internal_id
        };
        report.examined += 1;
        let before = hard.upload_cleanup(&id)?;
        let before_revision = hard.metadata()?;
        let mut scanned = false;
        let Some(op) = hard.upload(&id)? else {
            // Another serialized maintenance owner may have completed and
            // retired this receipt between passes. It is no longer our work.
            cursor.active = None;
            if cursor
                .temporary
                .as_ref()
                .is_some_and(|(owner, _)| *owner == id)
            {
                cursor.temporary = None;
            }
            return Ok(report);
        };
        // Receipt recovery above may finalize one namespace child. Reserve a
        // slot for that erasure rather than exceeding the shared pass budget.
        let mut remaining = if op.state == UploadState::RemoteVerified {
            7
        } else {
            8
        };
        let result = (|| -> Result<()> {
            if op.state == UploadState::RemoteVerified {
                self.finish_upload_adapter(id, master)?;
                foks_client::verified_file_upload(
                    &self.paths.hard_database,
                    &mut protected,
                    id,
                    &op.host_id,
                    &op.uid,
                    &op.party_id,
                )?;
                if hard.upload(&id)?.is_none_or(|op| !op.state.terminal()) {
                    cursor.active = None;
                    report.cleanup_deferred = true;
                    report.blocked = true;
                    return Ok(());
                }
            }
            for child in hard.upload_children(&id)? {
                if remaining == 0 || std::time::Instant::now() >= deadline {
                    break;
                }
                if let Some(child) = hard.mutation(&child)? {
                    if child.state == MutationState::RemoteVerified {
                        // Finalization removes one protected child request. It
                        // shares the spool/temporary/dependency erasure budget.
                        self.client.finalize_verified_kv_material(
                            &self.paths.hard_database,
                            &mut protected,
                            &child,
                        )?;
                        remaining -= 1;
                    }
                }
            }
            foks_client::cleanup_file_upload_records(
                &self.paths.hard_database,
                &mut protected,
                id,
                &mut remaining,
                deadline,
            )?;
            let Some(progress) = hard.upload_cleanup(&id)? else {
                return Ok(());
            };
            if progress.phase == UploadCleanupPhase::Temporaries
                && remaining > 0
                && std::time::Instant::now() < deadline
            {
                if cursor
                    .temporary
                    .as_ref()
                    .is_some_and(|(owner, _)| *owner != id)
                {
                    report.blocked = true;
                    return Ok(());
                }
                if cursor.temporary.is_none() {
                    cursor.temporary = Some((id, protected.temporary_scan()?));
                }
                let (_, scan) = cursor
                    .temporary
                    .as_mut()
                    .expect("upload temporary scan initialized");
                let sweep = protected.cleanup_temporary_budgeted(scan, deadline, remaining)?;
                scanned = sweep.examined > 0 || sweep.complete;
                remaining -= sweep.removed as usize;
                report.temporary_removed += sweep.removed;
                report.examined += sweep.examined;
                if !sweep.complete {
                    report.cleanup_deferred = true;
                    return Ok(());
                }
                // cleanup_temporary_budgeted fsyncs the directory, even if empty.
                hard.advance_upload_cleanup(&id, &progress)?;
                cursor.temporary = None;
            }
            foks_client::cleanup_file_upload_records(
                &self.paths.hard_database,
                &mut protected,
                id,
                &mut remaining,
                deadline,
            )?;
            let Some(progress) = hard.upload_cleanup(&id)? else {
                return Ok(());
            };
            if progress.phase == UploadCleanupPhase::Dependencies
                && std::time::Instant::now() < deadline
            {
                // This path only compacts already-erased upload-owned material;
                // compact_data_write refuses until every durable erasure flag is set.
                self.finish_upload_adapter(id, master)?;
                for child in hard.upload_children(&id)? {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    hard.retire_upload_child(&id, &child)?;
                }
                hard.advance_upload_cleanup(&id, &progress)?;
            }
            if hard
                .upload_cleanup(&id)?
                .is_some_and(|p| p.phase == UploadCleanupPhase::Complete)
            {
                cursor.active = None;
                report.compacted += 1;
            } else {
                report.cleanup_deferred = true;
            }
            Ok(())
        })();
        match result {
            Ok(()) => (),
            Err(Error::ProtectedStore(_))
            | Err(Error::Client(foks_client::Error::ProtectedStore(_)))
            | Err(Error::Client(foks_client::Error::Database(
                foks_client_db::Error::UploadCleanupDeferred
                | foks_client_db::Error::AdapterCleanupDeferred,
            )))
            | Err(Error::ClientDatabase(foks_client_db::Error::UploadCleanupDeferred))
            | Err(Error::ClientDatabase(foks_client_db::Error::AdapterCleanupDeferred)) => {
                // A blocked owner cannot monopolize all later uploads.
                cursor.active = None;
                if cursor
                    .temporary
                    .as_ref()
                    .is_some_and(|(owner, _)| *owner == id)
                {
                    cursor.temporary = None;
                }
                report.blocked = true;
                report.cleanup_deferred = true;
            }
            Err(error) => return Err(error),
        }
        let after = hard.upload_cleanup(&id)?;
        let owner_progress = before != after || before_revision != hard.metadata()? || scanned;
        report.made_progress |= owner_progress;
        report.more_work |= owner_progress
            && !report.blocked
            && after.is_some_and(|p| p.phase != UploadCleanupPhase::Complete);
        cursor.burst = cursor.burst.saturating_add(1);
        if !owner_progress || report.blocked {
            if cursor.blocked.len() >= 128 {
                cursor.blocked.pop_first();
            }
            cursor
                .blocked
                .insert(id, Instant::now() + Duration::from_secs(60));
            cursor.active = None;
        } else if cursor.burst >= 4 {
            // Keep a partial temporary scan, but rotate its owner fairly.
            cursor.active = None;
        }
        if cursor.active.is_none() {
            let selection = runnable_cleanup(
                &hard,
                CleanupSeek {
                    after: cursor.after,
                    ..Default::default()
                },
                &cursor.blocked,
                deadline,
                131,
            )?;
            report.more_work |= selection.candidate.is_some() || selection.continuation.is_some();
            // A lookahead may save only an unfinished seek, never skip the
            // selected candidate that the next physical turn must process.
            if selection.candidate.is_none() {
                cursor.seek = selection.continuation;
                if let Some(seek) = cursor.seek {
                    cursor.after = seek.after;
                }
            }
        }
        Ok(report)
    }

    fn maintain_upload_receipt_time(
        &self,
        hard: &mut HardStateStore,
        cursor: &mut UploadMaintenanceCursor,
        deadline: Instant,
        report: &mut AdapterMaintenanceReport,
    ) -> Result<()> {
        let now = Instant::now();
        cursor.clock_blocked.retain(|_, (until, _)| *until > now);
        if let Some(summary) = hard.next_upload_unstamped(cursor.unstamped)? {
            cursor.unstamped = Some(summary.internal_id);
            report.more_work |= hard
                .next_upload_unstamped(Some(summary.internal_id))?
                .is_some();
            let key = (summary.host_id, summary.uid);
            if !cursor.clock_blocked.get(&key).is_some_and(|(_, bad)| *bad) {
                let before = hard.metadata()?;
                match self
                    .adapter_clock
                    .sample()
                    .map_err(|_| Error::ClientDatabase(foks_client_db::Error::UploadClockUntrusted))
                    .and_then(|sample| {
                        hard.record_upload_terminal(&summary.internal_id, Some(sample))
                            .map_err(Error::from)
                    }) {
                    Ok(()) => {
                        let progress = before != hard.metadata()?;
                        report.made_progress |= progress;
                        if !progress {
                            cursor.full_accounts.remove(&key);
                            report.clock_untrusted = true;
                            if cursor.clock_blocked.len() >= 128 {
                                cursor.clock_blocked.pop_first();
                            }
                            cursor
                                .clock_blocked
                                .insert(key, (now + Duration::from_secs(60), true));
                        }
                    }
                    Err(Error::ClientDatabase(foks_client_db::Error::UploadClockUntrusted)) => {
                        cursor.full_accounts.remove(&key);
                        report.clock_untrusted = true;
                        if cursor.clock_blocked.len() >= 128 {
                            cursor.clock_blocked.pop_first();
                        }
                        cursor
                            .clock_blocked
                            .insert(key, (now + Duration::from_secs(60), true));
                    }
                    Err(error) => return Err(error),
                }
            } else {
                report.clock_untrusted |= cursor
                    .clock_blocked
                    .get(&key)
                    .is_some_and(|(_, untrusted)| *untrusted);
            }
        } else {
            cursor.unstamped = None;
        }
        if Instant::now() >= deadline {
            report.more_work = true;
            return Ok(());
        }
        let mut account = hard.next_upload_clock_account(
            cursor
                .account
                .as_ref()
                .map(|(h, u)| (h.as_slice(), u.as_slice())),
        )?;
        if account.is_none() {
            account = hard.next_upload_clock_account(None)?;
        }
        if let Some((host, uid)) = account {
            cursor.account = Some((host.clone(), uid.clone()));
            report.more_work |= hard
                .next_upload_clock_account(Some((&host, &uid)))?
                .is_some();
            let key = (host, uid);
            if !cursor.clock_blocked.contains_key(&key) {
                match self
                    .adapter_clock
                    .sample()
                    .map_err(|_| Error::ClientDatabase(foks_client_db::Error::UploadClockUntrusted))
                    .and_then(|sample| {
                        hard.prune_upload_receipts(&key.0, &key.1, sample, 128)
                            .map_err(Error::from)
                    }) {
                    Ok(count) => {
                        report.pruned += u64::from(count);
                        report.made_progress |= count > 0;
                        report.more_work |= count == 128;
                        if count == 128 {
                            if cursor.full_accounts.len() < 128 {
                                cursor.full_accounts.insert(key.clone());
                            }
                        } else {
                            cursor.full_accounts.remove(&key);
                            if cursor.clock_blocked.len() >= 128 {
                                cursor.clock_blocked.pop_first();
                            }
                            cursor
                                .clock_blocked
                                .insert(key, (now + Duration::from_secs(60), false));
                        }
                    }
                    Err(Error::ClientDatabase(foks_client_db::Error::UploadClockUntrusted)) => {
                        cursor.full_accounts.remove(&key);
                        report.clock_untrusted = true;
                        if cursor.clock_blocked.len() >= 128 {
                            cursor.clock_blocked.pop_first();
                        }
                        cursor
                            .clock_blocked
                            .insert(key, (now + Duration::from_secs(60), true));
                    }
                    Err(error) => return Err(error),
                }
            } else {
                report.clock_untrusted |= cursor
                    .clock_blocked
                    .get(&key)
                    .is_some_and(|(_, untrusted)| *untrusted);
            }
        } else {
            cursor.account = None;
        }
        report.more_work |= !cursor.full_accounts.is_empty();
        Ok(())
    }
}

/// One bounded circular seek. At most128 deferred owners are held in memory;
/// skipping them cannot turn a lone blocked row into a permanent fast loop.
#[derive(Clone, Copy, Debug, Default)]
struct CleanupSeek {
    after: Option<[u8; 16]>,
    first: Option<[u8; 16]>,
    wrapped: bool,
}
struct CleanupSelection {
    candidate: Option<foks_client_db::UploadSummary>,
    continuation: Option<CleanupSeek>,
}
fn runnable_cleanup(
    hard: &HardStateStore,
    mut seek: CleanupSeek,
    blocked: &BTreeMap<[u8; 16], Instant>,
    deadline: Instant,
    max_probes: usize,
) -> Result<CleanupSelection> {
    for _ in 0..max_probes {
        if Instant::now() >= deadline {
            break;
        }
        let Some(summary) = hard.next_upload_cleanup(seek.after)? else {
            if seek.wrapped || seek.after.is_none() {
                return Ok(CleanupSelection {
                    candidate: None,
                    continuation: None,
                });
            }
            seek.wrapped = true;
            seek.after = None;
            continue;
        };
        if seek.first == Some(summary.internal_id) {
            return Ok(CleanupSelection {
                candidate: None,
                continuation: None,
            });
        }
        seek.first.get_or_insert(summary.internal_id);
        seek.after = Some(summary.internal_id);
        if blocked
            .get(&summary.internal_id)
            .is_none_or(|until| *until <= Instant::now())
        {
            return Ok(CleanupSelection {
                candidate: Some(summary),
                continuation: None,
            });
        }
    }
    Ok(CleanupSelection {
        candidate: None,
        continuation: Some(seek),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cancelled_fixture(
        s: &CheckedProfileSession<'_>,
        master: &[u8; 32],
        seed: u8,
        chunks: u32,
        sample: foks_client_db::UploadTimeSample,
    ) -> Result<[u8; 16]> {
        use foks_client::ProtectedMutationStore;
        use foks_client_db::{UploadChunkRecord, UploadIntentBinding, UploadScope};
        let mut hard = HardStateStore::open(&s.paths.hard_database)?;
        let id = [seed; 16];
        let host = [seed; 33];
        let user = [2; 33];
        let party = [3; 33];
        let scope = UploadScope {
            host_id: &host,
            uid: &user,
            party_id: &party,
        };
        let intent = UploadIntentBinding {
            fingerprint: [seed; 32],
        };
        let context = hard.upload_issuance_context(scope, sample)?;
        let handle = foks_proto::UploadHandle::new(context.database_id, context.issued_at, id);
        hard.register_upload_intent(handle, id, scope, intent, sample)?;
        hard.claim_upload_source(handle, scope, intent)?;
        let mut op = hard.upload(&id)?.unwrap();
        op.file_id = [1; 17];
        op.manifest_hash = [4; 32];
        op.reserved_bytes = u64::from(chunks) + 1;
        hard.record_upload(&op)?;
        let mut protected = EncryptedFileMutationStore::open(
            &s.paths.protected_mutations,
            derive_mutation_key(master),
        )?;
        protected.put_if_absent(
            &foks_client::ProtectedRecordKey::UploadManifest(&id).encoded(),
            b"fixture",
        )?;
        for sequence in 0..chunks {
            protected.put_if_absent(
                &foks_client::ProtectedRecordKey::UploadChunk(&id, sequence).encoded(),
                b"fixture",
            )?;
            hard.record_upload_chunk(
                &id,
                &UploadChunkRecord {
                    sequence,
                    offset: u64::from(sequence),
                    final_chunk: sequence + 1 == chunks,
                    encoded_bytes: 1,
                    payload_hash: [5; 32],
                },
            )?;
        }
        hard.cancel_upload(&id)?;
        Ok(id)
    }

    #[test]
    fn chunk_debt_drains_with_eight_removal_turns_and_fair_owner_rotation() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, _, master| {
            let sample = s.adapter_clock.sample()?;
            let big = cancelled_fixture(s, master, 1, 256, sample)?;
            let small = cancelled_fixture(s, master, 2, 1, sample)?;
            let hard = HardStateStore::open(&s.paths.hard_database)?;
            let mut cursor = UploadMaintenanceCursor::default();
            let started = Instant::now();
            let count_files = || {
                std::fs::read_dir(&s.paths.protected_mutations)
                    .unwrap()
                    .filter(|v| {
                        v.as_ref()
                            .unwrap()
                            .path()
                            .extension()
                            .is_some_and(|ext| ext == "pms")
                    })
                    .count()
            };
            let mut small_at = None;
            for turn in 0..160 {
                let before = count_files();
                s.maintain_upload_state(&mut cursor, master)?;
                assert!(before.saturating_sub(count_files()) <= 8);
                if hard.upload_cleanup(&small)?.unwrap().phase == UploadCleanupPhase::Complete
                    && small_at.is_none()
                {
                    small_at = Some(turn);
                }
                if hard.upload_cleanup(&big)?.unwrap().phase == UploadCleanupPhase::Complete {
                    assert!(
                        turn >= 32,
                        "256 chunks cannot erase in fewer than33 eight-record passes"
                    );
                    assert!(
                        small_at.is_some_and(|turn| turn < 16),
                        "small owner starved: {small_at:?}"
                    );
                    assert_eq!(hard.upload_cleanup_debt()?.reserved_bytes, 0);
                    eprintln!(
                        "256 tiny chunk records + small owner drained in {} turns / {:?}",
                        turn + 1,
                        started.elapsed()
                    );
                    return Ok(());
                }
            }
            panic!(
                "bounded cleanup failed to drain: {:?}",
                hard.upload_cleanup(&big)?
            );
        });
    }

    #[test]
    fn remote_verified_protected_failure_yields_to_later_owner() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, _, master| {
            let sample = s.adapter_clock.sample()?;
            let bad = cancelled_fixture(s, master, 1, 1, sample)?;
            let good = cancelled_fixture(s, master, 2, 0, sample)?;
            let child = [33u8;16];
            let c = rusqlite::Connection::open(&s.paths.hard_database).unwrap();
            c.execute("INSERT INTO mutation_operations(operation_id,operation_kind,host_id,scope_id,subject_id,request_hash,material_ref,material_hash,state,attempt_count,created_at,updated_at) VALUES(?1,5,?2,?3,x'',zeroblob(32),?1,zeroblob(32),4,1,1,1)", rusqlite::params![child,s.pinned_host()?.host_id().as_bytes(),[2u8;33]]).unwrap();
            c.execute("UPDATE kv_uploads SET state=5,namespace_child=?2,plaintext_bytes=1,receipt_version=1,receipt_directories=0,receipt_entries=1 WHERE operation_id=?1", rusqlite::params![bad,child]).unwrap();
            // Its missing protected child causes a real read failure before
            // request decoding; other owners' ordinary files remain usable.
            let hard = HardStateStore::open(&s.paths.hard_database)?;
            let mut cursor = UploadMaintenanceCursor::default();
            let report = s.maintain_upload_state(&mut cursor, master)?;
            assert!(report.blocked && report.more_work);
            assert!(cursor.active.is_none() && cursor.blocked.contains_key(&bad));
            for _ in 0..8 {
                s.maintain_upload_state(&mut cursor, master)?;
                if hard.upload_cleanup(&good)?.unwrap().phase == UploadCleanupPhase::Complete { break; }
            }
            assert_eq!(hard.upload_cleanup(&good)?.unwrap().phase, UploadCleanupPhase::Complete);
            assert_eq!(hard.upload(&bad)?.unwrap().state, UploadState::RemoteVerified);
            assert!(hard.upload(&bad)?.unwrap().reserved_bytes > 0);
            Ok(())
        });
    }

    #[test]
    fn full_prune_batch_remains_runnable_across_account_wrap() {
        struct Fixed(foks_client_db::UploadTimeSample);
        impl AdapterClock for Fixed {
            fn sample(&self) -> Result<foks_client_db::UploadTimeSample> {
                Ok(self.0)
            }
        }
        let f = crate::test_support::AccountFixture::start();
        let old = foks_client_db::UploadTimeSample {
            wall_seconds: 100,
            monotonic_seconds: 100,
            process_id: [1; 16],
            boot_id: None,
        };
        let expiry = 101 + foks_client_db::UPLOAD_TERMINAL_RETENTION_SECONDS;
        f.run(|s, _, _| {
            let mut hard = HardStateStore::open(&s.paths.hard_database)?;
            let scope = foks_client_db::UploadScope { host_id:&[1;33], uid:&[2;33], party_id:&[3;33] };
            let context = hard.upload_issuance_context(scope, old)?;
            let mut c = rusqlite::Connection::open(&s.paths.hard_database).unwrap(); let tx = c.transaction().unwrap();
            for n in 1u8..=129 {
                let id = [n;16]; let handle = foks_proto::UploadHandle::new(context.database_id,100,id);
                tx.execute("INSERT INTO kv_uploads(operation_id,host_id,uid,party_id,file_id,manifest_hash,state,reserved_bytes,created_at) VALUES(?1,?2,?3,?4,zeroblob(17),zeroblob(32),9,0,100)",rusqlite::params![id,[1u8;33],[2u8;33],[3u8;33]]).unwrap();
                tx.execute("INSERT INTO kv_upload_handles(handle_hash,handle,handle_version,issued_at,operation_id,intent_hash,terminal_at,expires_at,receipt_bytes) VALUES(?1,?2,1,100,?3,zeroblob(32),101,?4,1024)",rusqlite::params![foks_client_db::upload_handle_hash(handle),handle.to_string(),id,i64::try_from(expiry).unwrap()]).unwrap();
                tx.execute("INSERT INTO kv_upload_cleanup(operation_id,phase,next_chunk,parent_erased) VALUES(?1,4,0,1)",[id]).unwrap();
            }
            tx.commit().unwrap(); Ok(())
        });
        let late = foks_client_db::UploadTimeSample {
            wall_seconds: expiry + 1,
            monotonic_seconds: expiry + 1,
            ..old
        };
        let session = ProfileSession::open(&f.registry, "local")
            .unwrap()
            .with_adapter_clock(std::sync::Arc::new(Fixed(late)));
        f.credentials
            .with_checked_session(&session, |s| {
                let master = f.credentials.master_key()?;
                let mut cursor = UploadMaintenanceCursor::default();
                let first = s.maintain_upload_state(&mut cursor, &master)?;
                assert_eq!(first.pruned, 128);
                assert!(first.more_work);
                let second = s.maintain_upload_state(&mut cursor, &master)?;
                assert_eq!(second.pruned, 1);
                assert!(!second.more_work);
                Ok::<_, Error>(())
            })
            .unwrap();
    }

    #[test]
    fn bounded_seek_resumes_past_blocked_prefix_and_finishes_blocked_round() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, _, master| {
            let sample = s.adapter_clock.sample()?;
            for seed in 1..=3 {
                cancelled_fixture(s, master, seed, 0, sample)?;
            }
            let hard = HardStateStore::open(&s.paths.hard_database)?;
            let until = Instant::now() + Duration::from_secs(60);
            let mut blocked = BTreeMap::from([([1; 16], until), ([2; 16], until)]);
            let deadline = Instant::now() + Duration::from_secs(2);
            let first = runnable_cleanup(&hard, CleanupSeek::default(), &blocked, deadline, 1)?;
            assert_eq!(first.continuation.unwrap().after, Some([1; 16]));
            let second =
                runnable_cleanup(&hard, first.continuation.unwrap(), &blocked, deadline, 1)?;
            assert_eq!(second.continuation.unwrap().after, Some([2; 16]));
            let third =
                runnable_cleanup(&hard, second.continuation.unwrap(), &blocked, deadline, 1)?;
            assert_eq!(third.candidate.unwrap().internal_id, [3; 16]);
            blocked.insert([3; 16], until);
            let mut seek = CleanupSeek::default();
            for _ in 0..8 {
                let result = runnable_cleanup(&hard, seek, &blocked, deadline, 1)?;
                assert!(result.candidate.is_none());
                if let Some(next) = result.continuation {
                    seek = next;
                } else {
                    return Ok(());
                }
            }
            panic!("a fully blocked round must terminate despite one-probe turns");
        });
    }

    #[test]
    fn singleton_largest_owner_keeps_fast_continuation_across_burst_wrap() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, _, master| {
            let id = cancelled_fixture(s, master, 255, 64, s.adapter_clock.sample()?)?;
            let hard = HardStateStore::open(&s.paths.hard_database)?;
            let mut cursor = AdapterMaintenanceCursor::default();
            for _ in 0..80 {
                let report = s.maintain_adapter_state(&mut cursor, master)?;
                if hard.upload_cleanup(&id)?.unwrap().phase == UploadCleanupPhase::Complete {
                    return Ok(());
                }
                assert!(
                    report.more_work,
                    "runnable largest-key debt must not get an idle delay: {report:?}"
                );
            }
            panic!("singleton cleanup did not finish");
        });
    }

    #[test]
    fn untrusted_stamp_backoff_does_not_block_erasure_or_other_accounts() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, _, master| {
            let sample = s.adapter_clock.sample()?;
            let bad = cancelled_fixture(
                s,
                master,
                1,
                0,
                foks_client_db::UploadTimeSample {
                    wall_seconds: sample.wall_seconds + 600,
                    ..sample
                },
            )?;
            let good = cancelled_fixture(s, master, 2, 0, sample)?;
            let hard = HardStateStore::open(&s.paths.hard_database)?;
            let mut cursor = UploadMaintenanceCursor::default();
            cursor.full_accounts.insert((vec![1; 33], vec![2; 33]));
            let mut untrusted = false;
            for _ in 0..16 {
                untrusted |= s
                    .maintain_upload_state(&mut cursor, master)?
                    .clock_untrusted;
            }
            assert!(untrusted);
            assert!(!cursor.full_accounts.contains(&(vec![1; 33], vec![2; 33])));
            assert_eq!(
                hard.upload_cleanup(&bad)?.unwrap().phase,
                UploadCleanupPhase::Complete
            );
            assert_eq!(
                hard.upload_cleanup(&good)?.unwrap().phase,
                UploadCleanupPhase::Complete
            );
            assert_eq!(hard.next_upload_unstamped(None)?.unwrap().internal_id, bad);
            assert!(hard.next_upload_unstamped(Some(bad))?.is_none());
            assert!(cursor
                .clock_blocked
                .contains_key(&(vec![1; 33], vec![2; 33])));
            let reports: Vec<_> = (0..4)
                .map(|_| s.maintain_upload_state(&mut cursor, master))
                .collect::<Result<_>>()?;
            assert!(
                reports.iter().any(|r| !r.more_work && !r.made_progress),
                "blocked-only work must allow idle scheduling"
            );
            Ok(())
        });
    }

    #[test]
    fn metadata_clock_progress_preserves_partial_temporary_scan() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, _, master| {
            let sample = s.adapter_clock.sample()?;
            let first = cancelled_fixture(s, master, 1, 0, sample)?;
            cancelled_fixture(s, master, 2, 0, sample)?;
            // ReadDir must cross several256-entry batches. These retained final
            // names are never read/deleted by temporary-only upload cleanup.
            for n in 1000..1600u32 {
                std::fs::write(
                    s.paths.protected_mutations.join(format!("{n:064x}.pms")),
                    b"retained",
                )?;
            }
            let hard = HardStateStore::open(&s.paths.hard_database)?;
            let mut cursor = UploadMaintenanceCursor::default();
            let mut examined = 0;
            let mut complete = false;
            for _ in 0..16 {
                examined += s.maintain_upload_state(&mut cursor, master)?.examined;
                if hard.upload_cleanup(&first)?.unwrap().phase == UploadCleanupPhase::Complete {
                    complete = true;
                    break;
                }
            }
            assert!(
                complete,
                "metadata-only writes must not restart every partial sweep"
            );
            assert!(
                examined < 700,
                "scan restarted across clock/prune writes: {examined}"
            );
            assert!(
                hard.next_upload_unstamped(None)?.is_none(),
                "both terminal clocks must progress while scanning"
            );
            Ok(())
        });
    }

    #[test]
    fn retired_active_owner_and_new_database_generation_reset_cursor() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, _, master| {
            let generation = HardStateStore::open(&s.paths.hard_database)?
                .metadata()?
                .database_id;
            let mut cursor = UploadMaintenanceCursor {
                database_id: Some(generation),
                active: Some([77; 16]),
                ..Default::default()
            };
            s.maintain_upload_state(&mut cursor, master)?;
            assert!(cursor.active.is_none());
            cursor.database_id = Some([0; 16]);
            cursor.active = Some([78; 16]);
            cursor.after = Some([255; 16]);
            s.maintain_upload_state(&mut cursor, master)?;
            assert_eq!(cursor.database_id, Some(generation));
            assert!(cursor.active.is_none());
            assert!(cursor.after.is_none());
            Ok(())
        });
    }
}
