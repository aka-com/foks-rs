// Included inside the existing hard-state test module to reuse its authenticated host fixture.
fn adapter_fixture_operation(byte: u8) -> MutationOperation {
    MutationOperation {
        operation_id: [byte; 16],
        kind: MutationKind::KvAdapter,
        host_id: snapshot().host_id,
        scope_id: vec![1; 33],
        subject_id: Vec::new(),
        expected_version: None,
        request_hash: [3; 32],
        material_ref: vec![byte; 16],
        material_hash: [4; 32],
        state: MutationState::Prepared,
        attempt_count: 0,
        created_at: 100,
        updated_at: 100,
    }
}
fn adapter_time(wall: u64, monotonic: u64) -> AdapterTimeSample {
    AdapterTimeSample {
        process_id: [7; 16],
        wall_seconds: wall,
        monotonic_seconds: monotonic,
    }
}

#[test]
fn adapter_atomic_capacity_and_identity_binding() {
    let (_temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    for byte in 0..64 {
        let op = adapter_fixture_operation(byte);
        db.record_adapter_submission(
            SubmissionHandle::new(200_000, [byte; 16]),
            &op,
            adapter_time(200_000, 0),
        )
        .unwrap();
    }
    let op = adapter_fixture_operation(64);
    assert!(matches!(
        db.record_adapter_submission(
            SubmissionHandle::new(200_000, [64; 16]),
            &op,
            adapter_time(200_000, 0)
        ),
        Err(Error::AdapterActiveFull)
    ));
    assert!(db.mutation(&op.operation_id).unwrap().is_none());
    let handle = SubmissionHandle::new(200_000, [0; 16]);
    let mut changed = adapter_fixture_operation(0);
    changed.request_hash[0] ^= 1;
    assert!(matches!(
        db.record_adapter_submission(handle, &changed, adapter_time(200_000, 0)),
        Err(Error::AdapterIdentityConflict)
    ));
    assert!(db
        .adapter_submission(handle)
        .unwrap()
        .unwrap()
        .internal_id
        .is_some());
}

#[test]
fn adapter_compaction_preserves_proof_and_pruning_cannot_reopen_an_old_handle() {
    let (_temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = adapter_fixture_operation(1);
    let handle = SubmissionHandle::new(200_000, [1; 16]);
    let before = db.metadata().unwrap().revision;
    db.record_adapter_submission(handle, &op, adapter_time(200_000, 0))
        .unwrap();
    db.begin_mutation_submission(&op.operation_id, 101).unwrap();
    db.finish_adapter_submission(
        handle,
        true,
        true,
        Some([1; 17]),
        Some(adapter_time(200_001, 1)),
    )
    .unwrap();
    let retained = db.adapter_submission(handle).unwrap().unwrap();
    assert_eq!(retained.node_id, Some([1; 17]));
    assert!(retained.ancillary_committed);
    db.compact_adapter_submission(handle).unwrap();
    db.compact_adapter_submission(handle).unwrap();
    assert!(db.mutation(&op.operation_id).unwrap().is_none());
    let compact = db.adapter_submission(handle).unwrap().unwrap();
    assert_eq!(compact.node_id, retained.node_id);
    assert!(compact.internal_id.is_none());
    let elapsed = TERMINAL_RETENTION_SECONDS + 2;
    assert_eq!(
        db.prune_adapter_submissions(
            &op.host_id,
            &op.scope_id,
            adapter_time(200_000 + elapsed, elapsed)
        )
        .unwrap(),
        1
    );
    let clock = db
        .adapter_clock(&op.host_id, &op.scope_id)
        .unwrap()
        .unwrap();
    assert_eq!(clock.reject_issued_before, 200_001);
    db.reanchor_adapter_clock(
        &op.host_id,
        &op.scope_id,
        &clock,
        adapter_time(100_000, elapsed),
    )
    .unwrap();
    assert!(matches!(
        db.check_adapter_admission(
            &op.host_id,
            &op.scope_id,
            handle,
            adapter_time(100_000, elapsed)
        ),
        Err(Error::AdapterExpired)
    ));
    assert!(matches!(
        db.check_adapter_admission(
            &op.host_id,
            &op.scope_id,
            SubmissionHandle::new(200_001, [2; 16]),
            adapter_time(100_000, elapsed)
        ),
        Err(Error::AdapterFutureHandle) | Err(Error::AdapterClockUntrusted)
    ));
    assert!(db.metadata().unwrap().revision > before);
}

#[test]
fn adapter_unseen_expiry_is_irreversible_without_any_pruned_rows() {
    let (_temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = adapter_fixture_operation(1);
    let handle = SubmissionHandle::new(100_000, [1; 16]);
    assert!(matches!(
        db.check_adapter_admission(&op.host_id, &op.scope_id, handle, adapter_time(300_000, 0)),
        Err(Error::AdapterExpired)
    ));
    let clock = db
        .adapter_clock(&op.host_id, &op.scope_id)
        .unwrap()
        .unwrap();
    assert_eq!(clock.reject_issued_before, 300_000 - MAX_NEW_SUBMISSION_AGE_SECONDS);
    db.reanchor_adapter_clock(&op.host_id, &op.scope_id, &clock, adapter_time(100_000, 0))
        .unwrap();
    assert!(matches!(
        db.record_adapter_submission(handle, &op, adapter_time(100_000, 0)),
        Err(Error::AdapterExpired)
    ));
    assert!(db
        .connection
        .execute("UPDATE kv_adapter_clocks SET reject_issued_before=0", [])
        .is_err());
}

#[test]
fn adapter_poisoned_clock_neither_prunes_nor_blocks_retained_lookup() {
    let (_temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = adapter_fixture_operation(1);
    let handle = SubmissionHandle::new(200_000, [1; 16]);
    db.record_adapter_submission(handle, &op, adapter_time(200_000, 0))
        .unwrap();
    let before = db.adapter_clock(&op.host_id, &op.scope_id).unwrap();
    assert!(matches!(
        db.check_adapter_admission(
            &op.host_id,
            &op.scope_id,
            SubmissionHandle::new(900_000, [2; 16]),
            adapter_time(900_000, 1)
        ),
        Err(Error::AdapterClockUntrusted)
    ));
    assert!(matches!(
        db.prune_adapter_submissions(&op.host_id, &op.scope_id, adapter_time(900_000, 1)),
        Err(Error::AdapterClockUntrusted)
    ));
    db.check_adapter_admission(&op.host_id, &op.scope_id, handle, adapter_time(900_000, 1))
        .unwrap();
    assert_eq!(db.adapter_clock(&op.host_id, &op.scope_id).unwrap(), before);
    let restarted = AdapterTimeSample {
        process_id: [8; 16],
        ..adapter_time(900_000, 0)
    };
    assert!(matches!(
        db.prune_adapter_submissions(&op.host_id, &op.scope_id, restarted),
        Err(Error::AdapterClockUntrusted)
    ));
}

#[test]
fn adapter_terminal_parent_with_remote_verified_child_cannot_be_pruned_or_compacted() {
    let (_temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = adapter_fixture_operation(1);
    let handle = SubmissionHandle::new(200_000, [1; 16]);
    db.record_adapter_submission(handle, &op, adapter_time(200_000, 0))
        .unwrap();
    db.begin_mutation_submission(&op.operation_id, 101).unwrap();
    let mut child = adapter_fixture_operation(2);
    child.kind = MutationKind::KvNamespace;
    db.record_child_mutation(&child, &op.operation_id, true)
        .unwrap();
    db.begin_mutation_submission(&child.operation_id, 101)
        .unwrap();
    db.advance_mutation(&child.operation_id, MutationState::RemoteVerified, 102)
        .unwrap();
    db.finish_adapter_submission(handle, true, false, None, Some(adapter_time(200_001, 1)))
        .unwrap();
    assert!(matches!(
        db.compact_adapter_submission(handle),
        Err(Error::AdapterCleanupDeferred)
    ));
    let elapsed = TERMINAL_RETENTION_SECONDS + 2;
    assert_eq!(
        db.prune_adapter_submissions(
            &op.host_id,
            &op.scope_id,
            adapter_time(200_000 + elapsed, elapsed)
        )
        .unwrap(),
        0
    );
    assert!(db
        .adapter_submission(handle)
        .unwrap()
        .unwrap()
        .internal_id
        .is_some());
    db.advance_mutation(&child.operation_id, MutationState::Finalized, 103)
        .unwrap();
    db.compact_adapter_submission(handle).unwrap();
    assert!(db.mutation(&child.operation_id).unwrap().is_none());
    assert_eq!(
        db.prune_adapter_submissions(
            &op.host_id,
            &op.scope_id,
            adapter_time(200_000 + elapsed, elapsed)
        )
        .unwrap(),
        1
    );
}

#[test]
fn adapter_pending_age_does_not_shorten_terminal_retention() {
    let (_temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = adapter_fixture_operation(1);
    let handle = SubmissionHandle::new(200_000, [1; 16]);
    db.record_adapter_submission(handle, &op, adapter_time(200_000, 0))
        .unwrap();
    let elapsed = TERMINAL_RETENTION_SECONDS * 2;
    db.check_adapter_admission(
        &op.host_id,
        &op.scope_id,
        handle,
        adapter_time(200_000 + elapsed, elapsed),
    )
    .unwrap();
    db.finish_adapter_submission(
        handle,
        false,
        false,
        None,
        Some(adapter_time(200_000 + elapsed, elapsed)),
    )
    .unwrap();
    assert_eq!(
        db.adapter_submission(handle).unwrap().unwrap().expires_at,
        Some(200_000 + elapsed + TERMINAL_RETENTION_SECONDS)
    );
}

#[test]
fn adapter_retained_capacity_includes_terminal_rows_and_prunes_only_one_batch() {
    let (_temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = adapter_fixture_operation(1);
    let sample = adapter_time(200_000, 0);
    db.check_adapter_admission(
        &op.host_id,
        &op.scope_id,
        SubmissionHandle::new(200_000, [1; 16]),
        sample,
    )
    .unwrap();
    // Populate a full historical fixture efficiently, then restore the actual
    // insertion guard before testing both repository and alternate-writer paths.
    let trigger: String = db
        .connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='kv_adapter_capacity'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.connection
        .execute_batch("DROP TRIGGER kv_adapter_capacity")
        .unwrap();
    let started = std::time::Instant::now();
    {
        let tx = db.write_transaction().unwrap();
        let mut insert=tx.prepare("INSERT INTO kv_adapter_submissions(handle_hash,handle,handle_version,schema_version,
            host_id,user_id,team_id,input_hash,state,issued_at,created_at,terminal_at,expires_at)
            VALUES (?1,?2,1,1,?3,?4,X'',?5,1,200000,200000,200000,?6)").unwrap();
        for n in 0u128..65_536 {
            let handle = SubmissionHandle::new(200_000, n.to_be_bytes());
            insert
                .execute(params![
                    adapter_handle_hash(handle),
                    handle.to_string(),
                    op.host_id,
                    op.scope_id,
                    op.request_hash,
                    (200_000 + TERMINAL_RETENTION_SECONDS) as i64
                ])
                .unwrap();
        }
        drop(insert);
        tx.commit().unwrap();
    }
    db.connection.execute_batch(&trigger).unwrap();
    let bytes: i64 = db
        .connection
        .query_row(
            "SELECT page_count*page_size FROM pragma_page_count(),pragma_page_size()",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!(
        "65,536 retained rows: database bytes={bytes}, fixture build={:?}",
        started.elapsed()
    );
    let handle = SubmissionHandle::new(200_000, [0xff; 16]);
    assert!(matches!(
        db.record_adapter_submission(handle, &op, sample),
        Err(Error::AdapterRetentionFull)
    ));
    assert!(db.mutation(&op.operation_id).unwrap().is_none());
    assert!(db.connection.execute("INSERT INTO kv_adapter_submissions(handle_hash,handle,handle_version,schema_version,host_id,user_id,team_id,input_hash,state,issued_at,created_at,terminal_at,expires_at)
        VALUES (?1,?2,1,1,?3,?4,X'',?5,1,200000,200000,200000,3000000)",
        params![adapter_handle_hash(handle),handle.to_string(),op.host_id,op.scope_id,op.request_hash]).is_err());
    let elapsed = TERMINAL_RETENTION_SECONDS + 1;
    let started = std::time::Instant::now();
    assert_eq!(
        db.prune_adapter_submissions(
            &op.host_id,
            &op.scope_id,
            adapter_time(200_000 + elapsed, elapsed)
        )
        .unwrap(),
        128
    );
    eprintln!(
        "128-row prune transaction including commit: {:?}",
        started.elapsed()
    );
    db.record_adapter_submission(
        SubmissionHandle::new(200_000 + elapsed, [0xfe; 16]),
        &op,
        adapter_time(200_000 + elapsed, elapsed),
    )
    .unwrap();
}

#[test]
fn adapter_database_rejects_alternate_parent_and_evidence_rewrites() {
    let (_temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    let op = adapter_fixture_operation(1);
    let handle = SubmissionHandle::new(200_000, [1; 16]);
    assert!(db.record_mutation(&op).is_err());
    db.record_adapter_submission(handle, &op, adapter_time(200_000, 0))
        .unwrap();
    assert!(db
        .connection
        .execute(
            "UPDATE kv_adapter_submissions SET input_hash=?1",
            [[8u8; 32]]
        )
        .is_err());
    assert!(db
        .connection
        .execute("UPDATE kv_adapter_submissions SET issued_at=0", [])
        .is_err());
    db.finish_adapter_submission(handle, false, false, None, Some(adapter_time(200_000, 0)))
        .unwrap();
    assert!(db
        .connection
        .execute("UPDATE kv_adapter_submissions SET state=0", [])
        .is_err());
    assert!(db
        .connection
        .execute(
            "UPDATE kv_adapter_submissions SET expires_at=expires_at+1",
            []
        )
        .is_err());
}

#[test]
fn adapter_concurrent_admission_reserves_the_last_active_slot_once() {
    let (temp, mut db) = store();
    db.accept_host_parts(snapshot().parts()).unwrap();
    for byte in 0..63 {
        db.record_adapter_submission(
            SubmissionHandle::new(200_000, [byte; 16]),
            &adapter_fixture_operation(byte),
            adapter_time(200_000, 0),
        )
        .unwrap();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads = (63..65)
        .map(|byte| {
            let path = temp.path().join("hard.sqlite");
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = HardStateStore::open(&path).unwrap();
                barrier.wait();
                db.record_adapter_submission(
                    SubmissionHandle::new(200_000, [byte; 16]),
                    &adapter_fixture_operation(byte),
                    adapter_time(200_000, 0),
                )
            })
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(Error::AdapterActiveFull)))
            .count(),
        1
    );
    let op = adapter_fixture_operation(1);
    assert_eq!(
        db.adapter_submission_batch(&op.host_id, &op.scope_id, true)
            .unwrap()
            .len(),
        64
    );
}
