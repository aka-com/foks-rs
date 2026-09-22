//! Local-only retention work. The resident agent supplies fair profile scheduling.
use crate::*;
use foks_client_db::{adapter_handle_hash, AdapterLedgerState};

#[derive(Default)]
pub struct AdapterMaintenanceCursor {
    after_handle: Vec<u8>,
    after_account: Option<(Vec<u8>, Vec<u8>)>,
    temporary: Option<foks_client::ProtectedTemporaryScan>,
    final_scan: Option<foks_client::ProtectedTemporaryScan>,
    inventory: foks_client::ProtectedRecordInventory,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AdapterMaintenanceReport {
    pub compacted: u64,
    pub pruned: u64,
    pub temporary_removed: u64,
    pub final_removed: u64,
    pub inventory_saturated: bool,
    pub examined: u64,
    pub cleanup_deferred: bool,
    pub clock_untrusted: bool,
}

impl CheckedProfileSession<'_> {
    pub fn maintain_adapter_state(
        &self,
        cursor: &mut AdapterMaintenanceCursor,
        master: &[u8; 32],
    ) -> Result<AdapterMaintenanceReport> {
        let started = std::time::Instant::now();
        let mut report = AdapterMaintenanceReport::default();
        let mut hard = HardStateStore::open(&self.paths.hard_database)?;
        let mut protected = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master),
        )?;
        if let Some(entry) = hard.next_adapter_maintenance(&cursor.after_handle)? {
            cursor.after_handle = adapter_handle_hash(entry.handle).to_vec();
            report.examined += 1;
            if let Some(id) = entry.internal_id {
                let children = hard.mutation_children(&id)?;
                for (child, _) in &children {
                    if started.elapsed() >= Duration::from_millis(50) {
                        report.cleanup_deferred = true;
                        break;
                    }
                    if child.state == MutationState::RemoteVerified {
                        // The owning KV helper validates kind and durable state.
                        if self
                            .client
                            .finalize_verified_kv_material(
                                &self.paths.hard_database,
                                &mut protected,
                                child,
                            )
                            .is_err()
                        {
                            report.cleanup_deferred = true;
                        }
                    }
                }
                if entry.state == AdapterLedgerState::Live
                    && started.elapsed() < Duration::from_millis(50)
                {
                    let parent = hard
                        .mutation(&id)?
                        .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
                    let proof = matches!(
                        parent.state,
                        MutationState::RemoteVerified | MutationState::Finalized
                    ) || hard
                        .mutation_children(&id)?
                        .iter()
                        .any(|(child, last)| *last && child.state == MutationState::Finalized);
                    if proof {
                        hard.finish_adapter_submission(
                            entry.handle,
                            true,
                            false,
                            None,
                            self.adapter_clock.sample().ok(),
                        )?;
                    }
                }
            }
            let entry = hard
                .adapter_submission(entry.handle)?
                .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
            if entry.state != AdapterLedgerState::Live
                && started.elapsed() < Duration::from_millis(50)
            {
                if entry.terminal_at.is_none() {
                    hard.finish_adapter_submission(
                        entry.handle,
                        entry.state == AdapterLedgerState::Committed,
                        entry.ancillary_committed,
                        entry.node_id,
                        self.adapter_clock.sample().ok(),
                    )?;
                }
                match self.compact_data_write(
                    &mut hard,
                    &entry,
                    master,
                    started + Duration::from_millis(50),
                ) {
                    Ok(()) => report.compacted += u64::from(entry.internal_id.is_some()),
                    Err(Error::ProtectedStore(_))
                    | Err(Error::ClientDatabase(foks_client_db::Error::AdapterCleanupDeferred)) => {
                        report.cleanup_deferred = true
                    }
                    Err(error) => return Err(error),
                }
            }
        } else {
            cursor.after_handle.clear();
        }
        if started.elapsed() < Duration::from_millis(50) {
            if let Some((host, user)) = hard.next_adapter_clock_account(
                cursor
                    .after_account
                    .as_ref()
                    .map(|(h, u)| (h.as_slice(), u.as_slice())),
            )? {
                cursor.after_account = Some((host.clone(), user.clone()));
                match self.adapter_clock.sample().and_then(|sample| {
                    hard.prune_adapter_submissions(&host, &user, sample)
                        .map_err(Error::from)
                }) {
                    Ok(pruned) => report.pruned = pruned as u64,
                    Err(Error::ClientDatabase(foks_client_db::Error::AdapterClockUntrusted)) => {
                        report.clock_untrusted = true
                    }
                    Err(error) => return Err(error),
                }
            } else {
                cursor.after_account = None;
            }
        } else {
            report.cleanup_deferred = true;
        }
        if started.elapsed() < Duration::from_millis(50) {
            let deadline = started + Duration::from_millis(50);
            let progress = cursor.inventory.advance(&hard, deadline)?;
            report.examined += progress.examined;
            report.inventory_saturated = progress.saturated;
            report.cleanup_deferred |= !progress.complete;
            if progress.restarted {
                cursor.final_scan = None;
            }
            if progress.complete {
                if cursor.final_scan.is_none() {
                    cursor.final_scan = Some(protected.temporary_scan()?);
                }
                match protected.reconcile_unowned_until(
                    cursor.final_scan.as_mut().expect("scan initialized"),
                    &cursor.inventory,
                    hard.metadata()?,
                    deadline,
                ) {
                    Ok(scan) => {
                        report.final_removed += scan.final_removed;
                        report.temporary_removed += scan.removed - scan.final_removed;
                        report.examined += scan.examined;
                        if scan.complete {
                            cursor.final_scan = None;
                        }
                    }
                    Err(error) => {
                        cursor.final_scan = None;
                        cursor.inventory = Default::default();
                        return Err(error.into());
                    }
                }
            } else if std::time::Instant::now() < deadline {
                if cursor.temporary.is_none() {
                    cursor.temporary = Some(protected.temporary_scan()?);
                }
                match protected.cleanup_temporary_until(
                    cursor.temporary.as_mut().expect("scan initialized"),
                    deadline,
                ) {
                    Ok(scan) => {
                        report.temporary_removed += scan.removed;
                        report.examined += scan.examined;
                        if scan.complete {
                            cursor.temporary = None;
                        }
                    }
                    Err(error) => {
                        cursor.temporary = None;
                        return Err(error.into());
                    }
                }
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_client::ProtectedMutationStore;

    #[test]
    fn repeated_cleanup_failure_keeps_terminal_ownership_and_the_reserved_slot() {
        let f = crate::test_support::AccountFixture::start();
        f.run(|s, _, master| {
            let mut hard = HardStateStore::open(&s.paths.hard_database)?;
            let sample = s.adapter_clock.sample()?;
            let handle = SubmissionHandle::new(sample.wall_seconds, [19; 16]);
            let op = foks_client_db::MutationOperation {
                operation_id: [20; 16],
                kind: MutationKind::KvAdapter,
                host_id: s.pinned_host()?.host_id().as_bytes().to_vec(),
                scope_id: vec![1; 33],
                subject_id: Vec::new(),
                expected_version: None,
                request_hash: [3; 32],
                material_ref: vec![20; 16],
                material_hash: [4; 32],
                state: MutationState::Prepared,
                attempt_count: 0,
                created_at: 1,
                updated_at: 1,
            };
            let mut protected = EncryptedFileMutationStore::open(
                &s.paths.protected_mutations,
                derive_mutation_key(master),
            )?;
            protected.put_if_absent(&op.material_ref, b"prepared request")?;
            hard.record_adapter_submission(handle, &op, sample)?;
            hard.finish_adapter_submission(handle, false, false, None, Some(sample))?;
            let path =
                s.paths
                    .protected_mutations
                    .join(EncryptedFileMutationStore::record_filename(
                        &op.material_ref,
                    )?);
            // A directory in place of a file deterministically fails unlink on
            // every supported test platform, independent of user/root privileges.
            std::fs::remove_file(&path)?;
            std::fs::create_dir(&path)?;
            let elapsed = foks_client_db::TERMINAL_RETENTION_SECONDS + 1;
            let late = foks_client_db::AdapterTimeSample {
                wall_seconds: sample.wall_seconds + elapsed,
                monotonic_seconds: sample.monotonic_seconds + elapsed,
                ..sample
            };
            for _ in 0..3 {
                let entry = hard.adapter_submission(handle)?.unwrap();
                assert!(s
                    .compact_data_write(
                        &mut hard,
                        &entry,
                        master,
                        std::time::Instant::now() + Duration::from_secs(1)
                    )
                    .is_err());
                assert_eq!(
                    hard.prune_adapter_submissions(&op.host_id, &op.scope_id, late)?,
                    0
                );
                assert_eq!(
                    hard.adapter_submission(handle)?.unwrap().internal_id,
                    Some(op.operation_id)
                );
                assert!(hard.mutation(&op.operation_id)?.is_some());
            }
            std::fs::remove_dir(&path)?;
            let entry = hard.adapter_submission(handle)?.unwrap();
            s.compact_data_write(
                &mut hard,
                &entry,
                master,
                std::time::Instant::now() + Duration::from_secs(1),
            )?;
            assert_eq!(
                hard.prune_adapter_submissions(&op.host_id, &op.scope_id, late)?,
                1
            );
            assert!(hard.adapter_submission(handle)?.is_none());
            assert!(matches!(
                hard.check_adapter_admission(&op.host_id, &op.scope_id, handle, late),
                Err(foks_client_db::Error::AdapterExpired)
            ));
            Ok(())
        });
    }

    #[test]
    fn local_maintenance_needs_no_user_sync_and_skips_a_busy_profile() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                label: None,
                probe: "unreachable.invalid:443".into(),
                trust: TrustRoot::WebPki,
                protocol: ProtocolPolicy::CurrentProbeOnly {
                    compatibility_artifact_public_key: "00".repeat(32),
                    lease_url: "https://unreachable.invalid/lease".into(),
                    last_artifact: None,
                },
            })
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        assert!(session.profile().require(Capability::UserSync).is_err());
        credentials
            .with_checked_session(&session, |checked| {
                let master = credentials.master_key()?;
                let mut protected = EncryptedFileMutationStore::open(
                    &checked.paths.protected_mutations,
                    derive_mutation_key(&master),
                )?;
                protected.put_if_absent(b"interrupted before journal", b"orphan")?;
                std::fs::write(
                    checked
                        .paths
                        .protected_mutations
                        .join(format!(".tmp-{:032x}", 1)),
                    b"temporary",
                )?;
                Ok::<_, Error>(())
            })
            .unwrap();
        let lock = runtime::ProfileLock::operation(session.paths()).unwrap();
        assert!(credentials
            .try_with_checked_session(&session, |_| Ok::<_, Error>(()))
            .unwrap()
            .is_none());
        lock.release().unwrap();
        let mut cursor = AdapterMaintenanceCursor::default();
        let mut removed = 0;
        let mut temporary = 0;
        for _ in 0..10 {
            let report = credentials
                .try_with_checked_session(&session, |checked| {
                    checked.maintain_adapter_state(&mut cursor, &*credentials.master_key()?)
                })
                .unwrap()
                .unwrap();
            removed += report.final_removed;
            temporary += report.temporary_removed;
            if removed == 1 && temporary == 1 {
                break;
            }
        }
        assert_eq!(removed, 1);
        assert_eq!(temporary, 1);
        let report = credentials
            .try_with_checked_session(&session, |checked| {
                checked.maintain_adapter_state(&mut cursor, &*credentials.master_key()?)
            })
            .unwrap()
            .unwrap();
        assert_eq!(report.final_removed + report.temporary_removed, 0);
    }
}
