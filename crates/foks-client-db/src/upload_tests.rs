// Uses the authenticated host fixture from the parent test module.
fn upload_fixture(id: u8) -> UploadOperation {
    UploadOperation {
        id: [id; 16],
        host_id: snapshot().host_id,
        uid: vec![1; 33],
        party_id: vec![1; 33],
        file_id: [id; 17],
        manifest_hash: [3; 32],
        state: UploadState::Preparing,
        chunk_count: 0,
        acknowledged_chunks: 0,
        encoded_bytes: 0,
        reserved_bytes: 1024,
        plaintext_bytes: None,
        namespace_child: None,
        adapter_parent: None,
        receipt_version: None,
        receipt_directories: None,
        receipt_entries: None,
        created_at: 100,
        chunks: vec![],
    }
}
fn register_upload_fixture(db: &mut HardStateStore, op: &UploadOperation) -> UploadHandle {
    let handle = UploadHandle::new(db.metadata().unwrap().database_id, 100, op.id);
    let scope = UploadScope {
        host_id: &op.host_id,
        uid: &op.uid,
        party_id: &op.party_id,
    };
    let binding = UploadIntentBinding {
        fingerprint: [7; 32],
    };
    db.register_upload_intent(handle, op.id, scope, binding, adapter_time(100, 0))
        .unwrap();
    db.claim_upload_source(handle, scope, binding).unwrap();
    handle
}
fn erase_upload_fixture(db: &mut HardStateStore, id: &[u8; 16]) {
    while let Some(cursor) = db.upload_cleanup(id).unwrap() {
        if cursor.phase == UploadCleanupPhase::Complete {
            break;
        }
        db.advance_upload_cleanup(id, &cursor).unwrap();
    }
}
fn upload_ready(db: &mut HardStateStore, id: &[u8; 16]) {
    db.record_upload_chunk(
        id,
        &UploadChunkRecord {
            sequence: 0,
            offset: 0,
            final_chunk: true,
            encoded_bytes: 100,
            payload_hash: [4; 32],
        },
    )
    .unwrap();
    db.ready_upload(id, 3000).unwrap();
}
#[test]
fn uploads_migrate_true_v40_schema_preserving_metadata_and_canonical_schema() {
    let (dir, db) = store();
    let before = db.metadata().unwrap();
    db.connection
        .execute_batch(repositories::uploads::REMOVE_SCHEMA)
        .unwrap();
    db.connection
        .pragma_update(None, "user_version", 40)
        .unwrap();
    let path = db.connection.path().unwrap().to_owned();
    drop(db);
    let db = HardStateStore::open(std::path::Path::new(&path)).unwrap();
    assert_eq!(db.metadata().unwrap().database_id, before.database_id);
    assert_eq!(db.upload_limits().unwrap().max_pending, 16);
    let fresh = HardStateStore::open(&dir.path().join("fresh.db")).unwrap();
    fn schema(db: &HardStateStore) -> Vec<(String, String)> {
        db.connection
            .prepare(
                "SELECT name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
    assert_eq!(schema(&db), schema(&fresh));
}
#[test]
fn upload_spool_admission_rechecks_aggregate_and_survives_reopen() {
    let (_dir, mut db) = store();
    db.set_upload_limits(UploadLimits {
        max_reserved_bytes: 16 * 1024 * 1024,
        max_pending: 1,
    })
    .unwrap();
    let mut op = upload_fixture(1);
    op.reserved_bytes = 16 * 1024 * 1024 - 5;
    db.preflight_upload(op.reserved_bytes).unwrap();
    register_upload_fixture(&mut db, &op);
    db.record_upload(&op).unwrap();
    assert!(db.preflight_upload(1).is_err());
    assert!(db.record_upload(&upload_fixture(2)).is_err());
    assert!(db.reserve_upload_bytes(&op.id, 6).is_err());
    assert_eq!(
        db.upload(&op.id).unwrap().unwrap().reserved_bytes,
        op.reserved_bytes
    );
    db.reserve_upload_bytes(&op.id, 5).unwrap();
    let path = db.connection.path().unwrap().to_owned();
    drop(db);
    let mut db = HardStateStore::open(std::path::Path::new(&path)).unwrap();
    assert_eq!(
        db.upload(&op.id).unwrap().unwrap().reserved_bytes,
        16 * 1024 * 1024
    );
    db.cancel_upload(&op.id).unwrap();
    assert!(
        db.preflight_upload(1).is_err(),
        "cancel does not release un-erased protected bytes"
    );
    erase_upload_fixture(&mut db, &op.id);
    register_upload_fixture(&mut db, &upload_fixture(2));
    db.record_upload(&upload_fixture(2)).unwrap();
}
#[test]
fn upload_chunk_cursor_replay_requires_exact_order_and_complete_acknowledgments() {
    let (_dir, mut db) = store();
    let op = upload_fixture(1);
    register_upload_fixture(&mut db, &op);
    db.record_upload(&op).unwrap();
    assert!(db.ready_upload(&op.id, 3000).is_err());
    upload_ready(&mut db, &op.id);
    assert!(db.begin_upload_chunk(&op.id, 0).is_err());
    db.begin_upload_pass(&op.id).unwrap();
    assert!(db
        .advance_upload(&op.id, UploadState::ContentComplete)
        .is_err());
    assert!(db.begin_upload_chunk(&op.id, 1).is_err());
    db.begin_upload_chunk(&op.id, 0).unwrap();
    // A crash/ambiguous response restarts a replay pass without treating attempt as ACK.
    db.begin_upload_pass(&op.id).unwrap();
    assert_eq!(db.upload(&op.id).unwrap().unwrap().acknowledged_chunks, 0);
    db.begin_upload_chunk(&op.id, 0).unwrap();
    db.acknowledge_upload_chunk(&op.id, 0).unwrap();
    assert!(db.acknowledge_upload_chunk(&op.id, 0).is_err());
    db.advance_upload(&op.id, UploadState::ContentComplete)
        .unwrap();
    db.cancel_upload(&op.id).unwrap();
    erase_upload_fixture(&mut db, &op.id);
    let done = db.upload(&op.id).unwrap().unwrap();
    assert!(done.chunks.is_empty());
    assert_eq!(done.chunk_count, 1);
    assert!(
        db.record_upload(&op).is_err(),
        "terminal receipt prevents ID reuse"
    );
}
#[test]
fn upload_namespace_binding_and_child_commit_are_atomic() {
    let (_dir, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = upload_fixture(1);
    register_upload_fixture(&mut db, &op);
    db.record_upload(&op).unwrap();
    upload_ready(&mut db, &op.id);
    db.begin_upload_pass(&op.id).unwrap();
    db.begin_upload_chunk(&op.id, 0).unwrap();
    db.acknowledge_upload_chunk(&op.id, 0).unwrap();
    db.advance_upload(&op.id, UploadState::ContentComplete)
        .unwrap();
    let mut child = adapter_fixture_operation(2);
    child.kind = MutationKind::KvNamespace;
    child.scope_id = op.party_id.clone();
    let mut wrong = child.clone();
    wrong.scope_id = vec![9; 33];
    assert!(db.record_upload_namespace(&wrong, &op.id, None).is_err());
    assert!(db.mutation(&child.operation_id).unwrap().is_none());
    db.record_upload_namespace(&child, &op.id, None).unwrap();
    assert!(db.cancel_upload(&op.id).is_err());
    assert!(db.verify_upload(&op.id, 1, 0, 0).is_err());
    db.begin_mutation_submission(&child.operation_id, 101)
        .unwrap();
    db.connection
        .execute(
            "UPDATE mutation_operations SET state=4 WHERE operation_id=?1",
            [child.operation_id],
        )
        .unwrap();
    db.verify_upload(&op.id, 7, 2, 3).unwrap();
    db.advance_upload(&op.id, UploadState::Finalized).unwrap();
    let done = db.upload(&op.id).unwrap().unwrap();
    assert_eq!(done.receipt_version, Some(7));
    assert_eq!(done.receipt_entries, Some(3));
}
#[test]
fn upload_needs_source_cannot_resume_or_publish() {
    let (_dir, mut db) = store();
    let op = upload_fixture(1);
    register_upload_fixture(&mut db, &op);
    db.record_upload(&op).unwrap();
    db.advance_upload(&op.id, UploadState::NeedsSource).unwrap();
    assert!(db.begin_upload_pass(&op.id).is_err());
    assert!(db.reserve_upload_bytes(&op.id, 1).is_err());
    assert!(db.ready_upload(&op.id, 3000).is_err());
    db.cancel_upload(&op.id).unwrap();
}

#[test]
fn upload_source_identity_is_consumed_once_before_any_manifest() {
    let (_dir, mut db) = store();
    let op = upload_fixture(44);
    let handle = UploadHandle::new(db.metadata().unwrap().database_id, 100, op.id);
    let scope = UploadScope {
        host_id: &op.host_id,
        uid: &op.uid,
        party_id: &op.party_id,
    };
    let binding = UploadIntentBinding {
        fingerprint: [7; 32],
    };
    db.register_upload_intent(handle, op.id, scope, binding, adapter_time(100, 0))
        .unwrap();
    assert_eq!(
        db.upload(&op.id).unwrap().unwrap().state,
        UploadState::AwaitingSource
    );
    db.claim_upload_source(handle, scope, binding).unwrap();
    assert!(db.claim_upload_source(handle, scope, binding).is_err());
    db.record_upload(&op).unwrap();
    assert_eq!(
        db.upload(&op.id).unwrap().unwrap().reserved_bytes,
        op.reserved_bytes
    );
    // A crash here has a charged owner even though the manifest does not yet exist.
    assert!(db.record_upload(&op).is_err());
    db.cancel_upload(&op.id).unwrap();
}

#[test]
fn resumed_upload_adapter_enters_submission_only_without_unresolved_children() {
    for prior_state in [
        None,
        Some(MutationState::Prepared),
        Some(MutationState::Submitting),
        Some(MutationState::SubmissionUnknown),
    ] {
        let (_dir, mut db) = store();
        db.accept_host_parts(snapshot().parts()).unwrap();
        let adapter = adapter_fixture_operation(60);
        db.record_adapter_submission(
            SubmissionHandle::new(200_000, [60; 16]),
            &adapter,
            adapter_time(200_000, 0),
        )
        .unwrap();
        db.begin_mutation_submission(&adapter.operation_id, 101)
            .unwrap();
        if let Some(state) = prior_state {
            let mut prior = adapter_fixture_operation(61);
            prior.kind = MutationKind::KvRoot;
            db.record_child_mutation(&prior, &adapter.operation_id, false)
                .unwrap();
            if state != MutationState::Prepared {
                db.begin_mutation_submission(&prior.operation_id, 101)
                    .unwrap();
            }
            if state == MutationState::SubmissionUnknown {
                db.advance_mutation(&prior.operation_id, state, 102)
                    .unwrap();
            }
        }
        db.advance_mutation(&adapter.operation_id, MutationState::SubmissionUnknown, 102)
            .unwrap();
        let mut upload = upload_fixture(62);
        upload.adapter_parent = Some(adapter.operation_id);
        register_upload_fixture(&mut db, &upload);
        db.record_upload(&upload).unwrap();
        upload_ready(&mut db, &upload.id);
        db.begin_upload_pass(&upload.id).unwrap();
        db.begin_upload_chunk(&upload.id, 0).unwrap();
        db.acknowledge_upload_chunk(&upload.id, 0).unwrap();
        db.advance_upload(&upload.id, UploadState::ContentComplete)
            .unwrap();
        let mut child = adapter_fixture_operation(63);
        child.kind = MutationKind::KvNamespace;
        let result =
            db.record_upload_namespace(&child, &upload.id, Some((adapter.operation_id, true)));
        if prior_state.is_some() {
            assert!(result.is_err());
            assert!(db.mutation(&child.operation_id).unwrap().is_none());
            assert_eq!(
                db.mutation(&adapter.operation_id).unwrap().unwrap().state,
                MutationState::SubmissionUnknown
            );
            assert_eq!(
                db.upload(&upload.id).unwrap().unwrap().state,
                UploadState::ContentComplete
            );
        } else {
            result.unwrap();
            assert_eq!(
                db.mutation(&adapter.operation_id).unwrap().unwrap().state,
                MutationState::Submitting
            );
            assert_eq!(
                db.upload(&upload.id).unwrap().unwrap().namespace_child,
                Some(child.operation_id)
            );
        }
    }
}

#[test]
fn upload_rejected_children_remain_owned_until_each_protected_erase_and_terminal_compaction() {
    let (_dir, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = upload_fixture(80);
    register_upload_fixture(&mut db, &op);
    db.record_upload(&op).unwrap();
    upload_ready(&mut db, &op.id);
    db.begin_upload_pass(&op.id).unwrap();
    db.begin_upload_chunk(&op.id, 0).unwrap();
    db.acknowledge_upload_chunk(&op.id, 0).unwrap();
    db.advance_upload(&op.id, UploadState::ContentComplete)
        .unwrap();
    let mut first = adapter_fixture_operation(81);
    first.kind = MutationKind::KvNamespace;
    first.scope_id = op.party_id.clone();
    db.record_upload_namespace(&first, &op.id, None).unwrap();
    assert!(matches!(
        db.mark_upload_child_erased(&op.id, &first.operation_id),
        Err(Error::UploadCleanupDeferred)
    ));
    db.connection
        .execute(
            "UPDATE mutation_operations SET state=5 WHERE operation_id=?1",
            [first.operation_id],
        )
        .unwrap();
    db.retry_upload_namespace(&op.id).unwrap();
    assert!(db
        .upload_for_namespace(&first.operation_id)
        .unwrap()
        .is_some());
    let mut second = first.clone();
    second.operation_id = [82; 16];
    db.record_upload_namespace(&second, &op.id, None).unwrap();
    db.connection
        .execute(
            "UPDATE mutation_operations SET state=5 WHERE operation_id=?1",
            [second.operation_id],
        )
        .unwrap();
    db.cancel_upload(&op.id).unwrap();
    assert_eq!(db.upload_children(&op.id).unwrap().len(), 2);
    let cursor = db.upload_cleanup(&op.id).unwrap().unwrap();
    assert_eq!(
        cursor.last_chunk,
        Some(1),
        "include the next possible unjournaled chunk"
    );
    while db.upload_cleanup(&op.id).unwrap().unwrap().phase != UploadCleanupPhase::Dependencies {
        let c = db.upload_cleanup(&op.id).unwrap().unwrap();
        db.advance_upload_cleanup(&op.id, &c).unwrap();
    }
    let debt = db.upload_cleanup(&op.id).unwrap().unwrap();
    assert!(db.advance_upload_cleanup(&op.id, &debt).is_err());
    assert_eq!(
        db.upload(&op.id).unwrap().unwrap().reserved_bytes,
        op.reserved_bytes
    );
    assert!(db.retire_upload_child(&op.id, &first.operation_id).is_err());
    for child in [first.operation_id, second.operation_id] {
        assert_eq!(db.next_upload_child_cleanup(&op.id).unwrap(), Some(child));
        db.mark_upload_child_erased(&op.id, &child).unwrap();
        db.retire_upload_child(&op.id, &child).unwrap();
    }
    assert!(db.next_upload_child_cleanup(&op.id).unwrap().is_none());
    db.advance_upload_cleanup(&op.id, &debt).unwrap();
    assert_eq!(db.upload(&op.id).unwrap().unwrap().reserved_bytes, 0);
}

#[test]
fn upload_adapter_compaction_requires_persisted_erasure_of_children_and_parent() {
    let (_dir, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let adapter = adapter_fixture_operation(90);
    let handle = SubmissionHandle::new(200_000, [90; 16]);
    db.record_adapter_submission(handle, &adapter, adapter_time(200_000, 0))
        .unwrap();
    db.begin_mutation_submission(&adapter.operation_id, 200_000)
        .unwrap();
    let mut op = upload_fixture(91);
    op.adapter_parent = Some(adapter.operation_id);
    register_upload_fixture(&mut db, &op);
    db.record_upload(&op).unwrap();
    upload_ready(&mut db, &op.id);
    db.begin_upload_pass(&op.id).unwrap();
    db.begin_upload_chunk(&op.id, 0).unwrap();
    db.acknowledge_upload_chunk(&op.id, 0).unwrap();
    db.advance_upload(&op.id, UploadState::ContentComplete)
        .unwrap();
    let mut child = adapter_fixture_operation(92);
    child.kind = MutationKind::KvNamespace;
    child.scope_id = op.party_id.clone();
    db.record_upload_namespace(&child, &op.id, Some((adapter.operation_id, true)))
        .unwrap();
    db.connection
        .execute(
            "UPDATE mutation_operations SET state=5 WHERE operation_id=?1",
            [child.operation_id],
        )
        .unwrap();
    db.cancel_upload(&op.id).unwrap();
    assert!(matches!(
        db.compact_adapter_submission(handle),
        Err(Error::AdapterCleanupDeferred)
    ));
    db.mark_upload_child_erased(&op.id, &child.operation_id)
        .unwrap();
    assert!(db.compact_adapter_submission(handle).is_err());
    db.mark_upload_parent_erased(&op.id).unwrap();
    db.compact_adapter_submission(handle).unwrap();
    assert!(db.upload(&op.id).unwrap().unwrap().adapter_parent.is_none());
    assert_eq!(
        db.upload_children(&op.id).unwrap(),
        vec![child.operation_id]
    );
    db.retire_upload_child(&op.id, &child.operation_id).unwrap();
    erase_upload_fixture(&mut db, &op.id);
    assert_eq!(db.upload(&op.id).unwrap().unwrap().reserved_bytes, 0);
}

#[test]
fn genuine_legacy_prepared_upload_keeps_exact_chunks_and_resumes_without_new_source_claim() {
    let (_dir, mut db) = store();
    let op = upload_fixture(95);
    register_upload_fixture(&mut db, &op);
    db.record_upload(&op).unwrap();
    upload_ready(&mut db, &op.id);
    let before = db.upload(&op.id).unwrap().unwrap();
    let generation = db.metadata().unwrap().database_id;
    db.connection.execute_batch("DROP TABLE kv_upload_children; DROP TABLE kv_upload_cleanup; DROP TABLE kv_upload_clocks; DROP TABLE kv_upload_handles; DROP TABLE kv_upload_receipt_limits;").unwrap();
    db.connection
        .pragma_update(None, "user_version", 41)
        .unwrap();
    let path = db.connection.path().unwrap().to_owned();
    drop(db);
    let mut db = HardStateStore::open(std::path::Path::new(&path)).unwrap();
    assert_eq!(db.metadata().unwrap().database_id, generation);
    assert_eq!(db.upload(&op.id).unwrap().unwrap(), before);
    let scope = UploadScope {
        host_id: &op.host_id,
        uid: &op.uid,
        party_id: &op.party_id,
    };
    let legacy = UploadHandle::legacy(op.id);
    assert!(db
        .upload_registration(legacy, scope)
        .unwrap()
        .unwrap()
        .intent
        .is_none());
    assert!(matches!(
        db.claim_upload_source(
            legacy,
            scope,
            UploadIntentBinding {
                fingerprint: [7; 32]
            }
        ),
        Err(Error::UploadLegacyReadOnly)
    ));
    db.begin_upload_pass(&op.id).unwrap();
    db.begin_upload_chunk(&op.id, 0).unwrap();
    db.acknowledge_upload_chunk(&op.id, 0).unwrap();
    db.advance_upload(&op.id, UploadState::ContentComplete)
        .unwrap();
}
