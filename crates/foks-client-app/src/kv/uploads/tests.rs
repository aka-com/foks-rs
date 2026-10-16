use super::*;
use crate::checkpoint::CheckpointStore as _;
use crate::test_support::AccountFixture;
use foks_server_testkit::TestFault;
use std::io::{Cursor, Read};

fn handle(f: &AccountFixture, seed: u8) -> UploadHandle {
    f.run(|s, v, _| {
        let context = s.kv_upload_issuance("me", None, v)?;
        Ok(UploadHandle::new(
            context.database_id,
            context.issued_at,
            [seed; 16],
        ))
    })
}
fn drain(s: &CheckedProfileSession<'_>, master: &[u8; 32]) -> Result<()> {
    let mut cursor = crate::AdapterMaintenanceCursor::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        s.maintain_adapter_state(&mut cursor, master)?;
        let hard = HardStateStore::open(&s.paths.hard_database)?;
        let mut after = None;
        let mut outstanding = None;
        while let Some(summary) = hard.next_upload_cleanup(after)? {
            after = Some(summary.internal_id);
            if summary.state == UploadState::RemoteVerified
                || (summary.state.terminal()
                    && hard
                        .upload_cleanup(&summary.internal_id)?
                        .is_some_and(|p| p.phase != foks_client_db::UploadCleanupPhase::Complete))
            {
                outstanding = Some((summary.state, hard.upload_cleanup(&summary.internal_id)?));
                break;
            }
        }
        if outstanding.is_none() {
            return Ok(());
        }
        assert!(
            std::time::Instant::now() < deadline,
            "cleanup failed to finish: {outstanding:?}"
        );
    }
}

#[test]
fn prepared_uploads_survive_lost_init_chunk_and_namespace_replies_without_source() {
    for (number, fault) in [
        TestFault::UploadInitAfterCommitBeforeResponse,
        TestFault::BetweenLargeFileChunks,
        TestFault::KvPutAfterCommitBeforeResponse,
    ]
    .into_iter()
    .enumerate()
    {
        let f = AccountFixture::start();
        f.run(|s, v, k| s.create_account("me", "uploadrecovery", "laptop", "", "", None, v, k));
        let id = handle(&f, number as u8 + 1);
        let original = vec![0x70 + number as u8; 4 * 1024 * 1024 + 123];
        let before = f.environment.arm_fault(fault);
        f.run(|s, v, k| {
            let attempt = s.put_kv_file_identified(
                "me",
                "/file.bin",
                &mut Cursor::new(&original),
                id,
                false,
                false,
                v,
                k,
            );
            if number < 2 {
                assert!(attempt.is_err());
            }
            // A namespace reply loss may already be reconciled by the normal proof path.
            Ok(())
        });
        assert_eq!(f.environment.fault_hits(), before + 1);
        let recorded = f.run(|s, v, _| s.list_kv_uploads("me", v));
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].prepared_chunks, 2);
        // A fresh checked profile action has no source; exact ciphertext is replayed.
        let done = f.run(|s, v, k| s.resume_kv_upload("me", id, v, k));
        assert_eq!(done.state, "completed");
        assert!(done.reserved_bytes > 0);
        f.run(|s, _, k| drain(s, k));
        assert_eq!(
            f.run(|s, v, _| s.read_kv_file("me", "/file.bin", v)),
            original
        );
        let same = f.run(|s, v, k| s.resume_kv_upload("me", id, v, k));
        assert_eq!(same.node_id, done.node_id);
        assert_eq!(same.version, done.version);
        f.run(|s, v, k| {
            assert!(s
                .put_kv_file_identified(
                    "me",
                    "/different",
                    &mut Cursor::new(vec![1; 3000]),
                    id,
                    false,
                    false,
                    v,
                    k
                )
                .is_err());
            Ok(())
        });
    }
}
#[test]
fn incomplete_source_preparation_never_sends_content_and_can_be_cancelled() {
    struct Broken {
        remaining: usize,
    }
    impl Read for Broken {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Err(std::io::Error::other("source disconnected"));
            }
            let n = out.len().min(self.remaining);
            out[..n].fill(3);
            self.remaining -= n;
            Ok(n)
        }
    }
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadbroken", "laptop", "", "", None, v, k));
    let id = handle(&f, 9);
    let hits = f
        .environment
        .arm_fault(TestFault::UploadInitAfterCommitBeforeResponse);
    f.run(|s, v, k| {
        assert!(s
            .put_kv_file_identified(
                "me",
                "/broken",
                &mut Broken {
                    remaining: 4 * 1024 * 1024 + 1
                },
                id,
                false,
                false,
                v,
                k
            )
            .is_err());
        Ok(())
    });
    assert_eq!(f.environment.fault_hits(), hits);
    f.run(|s, v, k| {
        let uploads = s.list_kv_uploads("me", v)?;
        assert_eq!(uploads[0].state, "needs-source");
        assert!(s.resume_kv_upload("me", id, v, k).is_err());
        let cancelled = s.cancel_kv_upload("me", id, v, k)?;
        assert_eq!(cancelled.state, "cancelled");
        assert!(cancelled.reserved_bytes > 0);
        drain(s, k)?;
        Ok(())
    });
    assert_eq!(f.environment.fault_hits(), hits);
}
#[test]
fn completed_upload_receipt_is_local_even_after_server_shutdown() {
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadreceipt", "laptop", "", "", None, v, k));
    let id = handle(&f, 7);
    f.run(|s, v, k| {
        s.put_kv_file_identified(
            "me",
            "/receipt",
            &mut Cursor::new(vec![5; 3000]),
            id,
            false,
            false,
            v,
            k,
        )
    });
    let expected = f.run(|s, v, k| s.resume_kv_upload("me", id, v, k));
    f._server.shutdown().unwrap();
    let session = ProfileSession::open(&f.registry, "local").unwrap();
    let actual = f
        .credentials
        .with_checked_session(&session, |s| {
            let master = f.credentials.master_key()?;
            let mut store = foks_keystore::EncryptedFileSecretStore::open(
                &s.paths.credential_store,
                derive_vault_key(&master),
            )?;
            s.resume_kv_upload("me", id, &mut AccountVault::new(&mut store), &master)
        })
        .unwrap();
    assert_eq!(actual.state, "completed");
    assert_eq!(actual.node_id, expected.node_id);
}
#[test]
fn changed_destination_blocks_prepared_overwrite_without_selecting_new_identity() {
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadoverwrite", "laptop", "", "", None, v, k));
    f.run(|s, v, k| {
        s.put_kv_file(
            "me",
            "/replace",
            &mut Cursor::new(b"original"),
            false,
            false,
            v,
            k,
        )
    });
    let id = handle(&f, 8);
    f.environment
        .arm_fault(TestFault::UploadInitAfterCommitBeforeResponse);
    f.run(|s, v, k| {
        assert!(s
            .put_kv_file_identified(
                "me",
                "/replace",
                &mut Cursor::new(vec![2; 3000]),
                id,
                true,
                false,
                v,
                k
            )
            .is_err());
        Ok(())
    });
    f.run(|s, v, k| {
        s.put_kv_file(
            "me",
            "/replace",
            &mut Cursor::new(b"new value"),
            true,
            false,
            v,
            k,
        )
    });
    f.run(|s, v, k| {
        assert!(s.resume_kv_upload("me", id, v, k).is_err());
        Ok(())
    });
    assert_eq!(
        f.run(|s, v, _| s.read_kv_file("me", "/replace", v)),
        b"new value"
    );
}

#[test]
fn adapter_upload_resume_persists_application_receipt_before_erasing_spool() {
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadadapter", "laptop", "", "", None, v, k));
    let body = vec![6; 3000];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let handle = SubmissionHandle::new(now, [22; 16]);
    let spec = DataWriteSpec {
        kind: DataWriteKind::Put,
        path: "/adapter".into(),
        destination: None,
        team_selector: None,
        overwrite: false,
        mkdir_p: false,
        recursive: false,
        body_length: body.len() as u64,
        body_hash: foks_crypto::kv_adapter_body_hash(&body),
    };
    f.run(|s, v, k| s.prepare_data_write("me", handle, spec, v, k));
    f.environment
        .arm_fault(TestFault::UploadInitAfterCommitBeforeResponse);
    let attempted =
        f.run(|s, v, k| s.execute_data_write("me", handle, &mut Cursor::new(&body), v, k));
    assert_eq!(attempted.status, DataWriteStatus::SubmissionUnknown);
    let id = f.run(|s, v, _| {
        let report = s.list_kv_uploads("me", v)?;
        let id = report[0].upload_id.parse::<UploadHandle>().unwrap();
        Ok(id)
    });
    let done = f.run(|s, v, k| s.resume_kv_upload("me", id, v, k));
    assert_eq!(done.state, "completed");
    assert!(done.reserved_bytes > 0);
    f.run(|s, _, k| drain(s, k));
    let app = f.run(|s, v, k| s.data_write_status("me", handle, v, k));
    assert_eq!(app.status, DataWriteStatus::Committed);
    assert_eq!(app.node_id, done.node_id);
    assert_eq!(f.run(|s, v, _| s.read_kv_file("me", "/adapter", v)), body);
}

#[test]
fn declared_identity_survives_failure_before_the_first_source_chunk() {
    struct NoBytes;
    impl Read for NoBytes {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other(
                "source disconnected before first byte",
            ))
        }
    }
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploademptyfail", "laptop", "", "", None, v, k));
    let id = handle(&f, 33);
    f.run(|s, v, k| {
        assert!(s
            .put_kv_file_identified("me", "/file", &mut NoBytes, id, false, false, v, k)
            .is_err());
        let report = s.list_kv_uploads("me", v)?;
        assert_eq!(report[0].state, "needs-source");
        assert!(s
            .put_kv_file_identified(
                "me",
                "/file",
                &mut Cursor::new(vec![3; 3000]),
                id,
                false,
                false,
                v,
                k
            )
            .is_err());
        assert_eq!(s.cancel_kv_upload("me", id, v, k)?.reserved_bytes, 0);
        Ok(())
    });
}
#[test]
fn explicit_small_upload_identity_is_terminal_and_does_not_claim_verified_success() {
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadsmall", "laptop", "", "", None, v, k));
    let id = handle(&f, 34);
    f.run(|s, v, k| {
        s.put_kv_file_identified(
            "me",
            "/small",
            &mut Cursor::new(b"tiny"),
            id,
            false,
            false,
            v,
            k,
        )
    });
    let report = f.run(|s, v, _| s.list_kv_uploads("me", v));
    assert_eq!(report[0].state, "inline-untracked");
    assert_eq!(report[0].reserved_bytes, 0);
    assert!(report[0].node_id.is_none());
    assert_eq!(f.run(|s, v, _| s.read_kv_file("me", "/small", v)), b"tiny");
    f.run(|s, v, k| {
        assert!(s.resume_kv_upload("me", id, v, k).is_err());
        Ok(())
    });
}
#[test]
fn cancelling_adapter_upload_commits_rejected_receipt_and_releases_pending_slot() {
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadcancel", "laptop", "", "", None, v, k));
    let body = vec![8; 3000];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let handle = SubmissionHandle::new(now, [35; 16]);
    let spec = DataWriteSpec {
        kind: DataWriteKind::Put,
        path: "/cancel".into(),
        destination: None,
        team_selector: None,
        overwrite: false,
        mkdir_p: false,
        recursive: false,
        body_length: body.len() as u64,
        body_hash: foks_crypto::kv_adapter_body_hash(&body),
    };
    f.run(|s, v, k| s.prepare_data_write("me", handle, spec, v, k));
    f.environment
        .arm_fault(TestFault::UploadInitAfterCommitBeforeResponse);
    f.run(|s, v, k| s.execute_data_write("me", handle, &mut Cursor::new(&body), v, k));
    f.run(|s, v, k| {
        let report = s.list_kv_uploads("me", v)?;
        let id = report[0].upload_id.parse::<UploadHandle>().unwrap();
        assert_eq!(s.cancel_kv_upload("me", id, v, k)?.state, "cancelled");
        assert_eq!(
            s.data_write_status("me", handle, v, k)?.status,
            DataWriteStatus::Rejected
        );
        assert!(s.pending_data_writes("me", v)?.is_empty());
        Ok(())
    });
}

#[test]
fn manifest_install_failures_keep_durable_owner_and_spool_charge() {
    use foks_client::{ProtectedMutationStore, ProtectedStoreError};
    struct FailedManifest {
        inner: EncryptedFileMutationStore,
        install: bool,
    }
    impl ProtectedMutationStore for FailedManifest {
        fn put_if_absent(
            &mut self,
            key: &[u8],
            value: &[u8],
        ) -> std::result::Result<(), ProtectedStoreError> {
            if key.starts_with(b"foks-upload-manifest-v1:") {
                if self.install {
                    self.inner.put_if_absent(key, value)?;
                }
                return Err(ProtectedStoreError::Backend(
                    "injected manifest failure".into(),
                ));
            }
            self.inner.put_if_absent(key, value)
        }
        fn get(
            &mut self,
            key: &[u8],
        ) -> std::result::Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
            self.inner.get(key)
        }
        fn remove(&mut self, key: &[u8]) -> std::result::Result<(), ProtectedStoreError> {
            self.inner.remove(key)
        }
    }
    for install in [false, true] {
        let f = AccountFixture::start();
        f.run(|s, v, k| s.create_account("me", "uploadmanifest", "laptop", "", "", None, v, k));
        let id = handle(&f, 55);
        let hits = f
            .environment
            .arm_fault(TestFault::UploadInitAfterCommitBeforeResponse);
        f.run(|s, v, k| {
            let host = s.pinned_host()?;
            let account = v.account("me")?;
            let authenticated = s.client.authenticate_and_pin(&host, &account.credential)?;
            let mut protected = FailedManifest {
                inner: EncryptedFileMutationStore::open(
                    &s.paths.protected_mutations,
                    derive_mutation_key(k),
                )?,
                install,
            };
            let mut session = s.client.user_kv_write_session(
                &host,
                &account.credential,
                &authenticated.verified,
                &authenticated.puks,
                &s.paths.soft_database,
                &mut protected,
            )?;
            let root = session.sync()?[0].root_directory_id;
            let intent = KvUploadIntent {
                path: "/manifest".into(),
                expected_size: None,
                overwrite: false,
                expected_version: None,
                selected_dirent: None,
                read_role: KvRoleSummary::Owner,
                write_role: KvRoleSummary::Owner,
                mkdir_p: false,
            };
            session.bind_file_upload(s.claim_upload("me", None, id, &intent, v)?)?;
            assert!(session
                .put_file(
                    root,
                    "manifest",
                    &mut Cursor::new(vec![2; 3000]),
                    KvWriteOptions {
                        read_role: Role::OWNER,
                        write_role: Role::OWNER,
                        overwrite: false,
                        expected_version: None
                    }
                )
                .is_err());
            drop(session);
            let hard = HardStateStore::open(&s.paths.hard_database)?;
            let upload = s.checked_upload("me", None, id, v)?;
            assert_eq!(upload.state, UploadState::NeedsSource);
            assert!(upload.reserved_bytes > 0);
            let mut inventory = foks_client::ProtectedRecordInventory::default();
            loop {
                let progress = inventory.advance(
                    &hard,
                    std::time::Instant::now() + std::time::Duration::from_secs(1),
                )?;
                assert!(!progress.saturated);
                if progress.complete {
                    break;
                }
            }
            assert!(inventory
                .records(hard.metadata()?)?
                .values()
                .any(|record| matches!(
                    record.family,
                    foks_client::ProtectedRecordFamily::UploadManifest
                ) && !record.exportable()));
            assert!(s.cancel_kv_upload("me", id, v, k)?.reserved_bytes > 0);
            drain(s, k)?;
            assert_eq!(s.checked_upload("me", None, id, v)?.reserved_bytes, 0);
            Ok(())
        });
        assert_eq!(f.environment.fault_hits(), hits);
    }
}

#[test]
fn source_claim_is_checkpointed_before_first_byte_and_cannot_be_rebound() {
    struct Inspect<F> {
        inspect: F,
        bytes: Cursor<Vec<u8>>,
        reads: usize,
    }
    impl<F: FnMut()> Read for Inspect<F> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.reads == 0 {
                (self.inspect)();
            }
            self.reads += 1;
            self.bytes.read(output)
        }
    }
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadbarrier", "laptop", "", "", None, v, k));
    let id = handle(&f, 62);
    f.run(|s, v, k| {
        let pinned = s.pinned_host()?;
        let host = pinned.host_id();
        let uid = v.account("me")?.credential.uid;
        let mut source = Inspect {
            inspect: || {
                let current = s.rollback_checkpoint().unwrap();
                f.credentials
                    .with_native_manifest(|manifest| {
                        let persisted: RollbackCheckpoint =
                            serde_json::from_slice(&manifest.get(&rollback_record_key("local")?)?)?;
                        assert_eq!(persisted.digest()?, current.digest()?);
                        let hard = HardStateStore::open(&s.paths.hard_database)?;
                        let registration = hard
                            .upload_registration(
                                id,
                                foks_client_db::UploadScope {
                                    host_id: host.as_bytes(),
                                    uid: uid.as_bytes(),
                                    party_id: uid.as_bytes(),
                                },
                            )?
                            .unwrap();
                        assert_eq!(registration.state, UploadState::Preparing);
                        Ok(())
                    })
                    .unwrap();
            },
            bytes: Cursor::new(vec![9; 3000]),
            reads: 0,
        };
        s.put_kv_file_identified("me", "/barrier", &mut source, id, false, false, v, k)?;
        assert!(source.reads > 0);
        let mut forbidden = Inspect {
            inspect: || panic!("reused handle read source"),
            bytes: Cursor::new(vec![8; 3000]),
            reads: 0,
        };
        assert!(s
            .put_kv_file_identified("me", "/barrier", &mut forbidden, id, false, false, v, k)
            .is_err());
        assert_eq!(forbidden.reads, 0);
        Ok(())
    });
}

#[test]
fn failed_claim_checkpoint_never_reads_source_and_preserves_consumed_intent() {
    struct Never;
    impl Read for Never {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("checkpoint failure read source")
        }
    }
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadbarrierfailure", "laptop", "", "", None, v, k));
    let id = handle(&f, 63);
    f.run(|s, v, k| {
        crate::checkpoint::FAIL_EFFECT_CHECKPOINT.set(2); // registration succeeds; claim publication fails
        assert!(s
            .put_kv_file_identified("me", "/barrier", &mut Never, id, false, false, v, k)
            .is_err());
        assert_eq!(
            s.checked_upload("me", None, id, v)?.state,
            UploadState::Preparing
        );
        Ok(())
    });
    f.run(|s, v, k| {
        assert!(s
            .put_kv_file_identified("me", "/barrier", &mut Never, id, false, false, v, k)
            .is_err());
        s.cancel_kv_upload("me", id, v, k)?;
        Ok(())
    });
}

#[test]
fn cleanup_sync_failure_keeps_cursor_and_debt_then_retries_one_record() {
    use foks_client::{ProtectedMutationStore, ProtectedStoreError};
    struct DeferredSync {
        inner: EncryptedFileMutationStore,
        fail: bool,
    }
    impl ProtectedMutationStore for DeferredSync {
        fn put_if_absent(
            &mut self,
            k: &[u8],
            v: &[u8],
        ) -> std::result::Result<(), ProtectedStoreError> {
            self.inner.put_if_absent(k, v)
        }
        fn get(
            &mut self,
            k: &[u8],
        ) -> std::result::Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
            self.inner.get(k)
        }
        fn remove(&mut self, k: &[u8]) -> std::result::Result<(), ProtectedStoreError> {
            self.inner.remove(k)
        }
        fn sync(&mut self) -> std::result::Result<(), ProtectedStoreError> {
            if self.fail {
                Err(ProtectedStoreError::Backend(
                    "injected cleanup sync failure".into(),
                ))
            } else {
                self.inner.sync()
            }
        }
    }
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadcleanupsync", "laptop", "", "", None, v, k));
    let id = handle(&f, 64);
    f.run(|s, v, k| {
        s.put_kv_file_identified(
            "me",
            "/cleanup",
            &mut Cursor::new(vec![5; 3000]),
            id,
            false,
            false,
            v,
            k,
        )?;
        let op = s.checked_upload("me", None, id, v)?;
        let hard = HardStateStore::open(&s.paths.hard_database)?;
        let before = hard.upload_cleanup(&op.id)?.unwrap();
        let mut store = DeferredSync {
            inner: EncryptedFileMutationStore::open(
                &s.paths.protected_mutations,
                derive_mutation_key(k),
            )?,
            fail: true,
        };
        let mut budget = 1;
        assert!(foks_client::cleanup_file_upload_records(
            &s.paths.hard_database,
            &mut store,
            op.id,
            &mut budget,
            std::time::Instant::now() + Duration::from_secs(1)
        )
        .is_err());
        let failed = hard.upload_cleanup(&op.id)?.unwrap();
        assert_eq!(
            (failed.phase, failed.next_chunk),
            (before.phase, before.next_chunk)
        );
        assert_eq!(
            hard.upload(&op.id)?.unwrap().reserved_bytes,
            op.reserved_bytes
        );
        store.fail = false;
        foks_client::cleanup_file_upload_records(
            &s.paths.hard_database,
            &mut store,
            op.id,
            &mut budget,
            std::time::Instant::now() + Duration::from_secs(1),
        )?;
        assert_eq!(budget, 0);
        assert_eq!(
            hard.upload(&op.id)?.unwrap().reserved_bytes,
            op.reserved_bytes
        );
        drain(s, k)?;
        assert_eq!(s.checked_upload("me", None, id, v)?.reserved_bytes, 0);
        Ok(())
    });
}

#[test]
fn injected_upload_clock_survives_core_completion_and_maintenance_without_reanchoring() {
    use std::sync::{Arc, Mutex};
    struct Clock(Mutex<foks_client_db::AdapterTimeSample>);
    impl crate::AdapterClock for Clock {
        fn sample(&self) -> Result<foks_client_db::AdapterTimeSample> {
            Ok(*self.0.lock().unwrap())
        }
    }
    let f = AccountFixture::start();
    f.run(|s, v, k| s.create_account("me", "uploadclockorigin", "laptop", "", "", None, v, k));
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let clock = Arc::new(Clock(Mutex::new(foks_client_db::AdapterTimeSample {
        process_id: [91; 16],
        wall_seconds: wall,
        monotonic_seconds: 0,
    })));
    let session = ProfileSession::open(&f.registry, "local")
        .unwrap()
        .with_adapter_clock(clock.clone());
    f.credentials
        .with_checked_session(&session, |s| {
            let master = f.credentials.master_key()?;
            let mut store = foks_keystore::EncryptedFileSecretStore::open(
                &s.paths.credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            s.put_kv_file(
                "me",
                "/implicit",
                &mut Cursor::new(vec![7; 3000]),
                false,
                false,
                &mut vault,
                &master,
            )?;
            drain(s, &master)?;
            let account = vault.account("me")?;
            let host = s.pinned_host()?;
            assert_eq!(
                HardStateStore::open(&s.paths.hard_database)?
                    .upload_clock(host.host_id().as_bytes(), account.credential.uid.as_bytes())?
                    .unwrap()
                    .process_id,
                [91; 16]
            );
            *clock.0.lock().unwrap() = foks_client_db::AdapterTimeSample {
                process_id: [91; 16],
                wall_seconds: wall + 3600,
                monotonic_seconds: 1,
            };
            assert!(matches!(
                s.kv_upload_issuance("me", None, &mut vault),
                Err(Error::ClientDatabase(
                    foks_client_db::Error::UploadClockUntrusted
                ))
            ));
            Ok::<_, Error>(())
        })
        .unwrap();
    assert_eq!(
        crate::SystemAdapterClock.sample().unwrap().process_id,
        foks_client::upload_time_sample().unwrap().process_id
    );
}
