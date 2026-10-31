use super::*;
fn database() -> (tempfile::TempDir, HardStateStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = HardStateStore::open(&dir.path().join("hard.db")).unwrap();
    (dir, db)
}
fn scope() -> UploadScope<'static> {
    UploadScope {
        host_id: &[1; 33],
        uid: &[2; 33],
        party_id: &[2; 33],
    }
}
fn binding() -> UploadIntentBinding {
    UploadIntentBinding {
        fingerprint: [3; 32],
    }
}
fn time(wall: u64) -> UploadTimeSample {
    UploadTimeSample {
        process_id: [4; 16],
        boot_id: None,
        wall_seconds: wall,
        monotonic_seconds: wall,
    }
}
fn handle(db: &HardStateStore, t: u64, n: u64) -> UploadHandle {
    let mut random = [0; 16];
    random[..8].copy_from_slice(&n.to_be_bytes());
    UploadHandle::new(db.metadata().unwrap().database_id, t, random)
}
fn id(n: u64) -> [u8; 16] {
    let mut id = [8; 16];
    id[..8].copy_from_slice(&n.to_be_bytes());
    id
}
fn register(db: &mut HardStateStore, h: UploadHandle, n: u64, t: u64) -> UploadRegistration {
    db.register_upload_intent(h, id(n), scope(), binding(), time(t))
        .unwrap()
}
fn clean(db: &mut HardStateStore, id: &[u8; 16]) {
    loop {
        let c = db.upload_cleanup(id).unwrap().unwrap();
        if c.phase == UploadCleanupPhase::Complete {
            break;
        }
        db.advance_upload_cleanup(id, &c).unwrap();
    }
}
#[test]
fn full_handle_registration_is_known_first_scoped_and_source_is_single_claim() {
    let (_dir, mut db) = database();
    let h = handle(&db, 100, 1);
    let first = register(&mut db, h, 7, 100);
    assert_ne!(first.internal_id, h.legacy_id().unwrap_or([0; 16]));
    db.set_upload_retention_limits(UploadRetentionLimits {
        max_rows: 1,
        max_bytes: 1024,
    })
    .unwrap();
    let repeated = db
        .register_upload_intent(h, id(8), scope(), binding(), time(u64::MAX))
        .unwrap();
    assert_eq!(first, repeated);
    assert!(matches!(
        db.register_upload_intent(
            h,
            id(8),
            scope(),
            UploadIntentBinding {
                fingerprint: [5; 32]
            },
            time(100)
        ),
        Err(Error::UploadIdentityConflict)
    ));
    let wrong = UploadScope {
        party_id: &[9; 33],
        ..scope()
    };
    assert!(matches!(
        db.upload_registration(h, wrong),
        Err(Error::UploadIdentityConflict)
    ));
    let new = handle(&db, 100, 2);
    assert!(matches!(
        db.register_upload_intent(new, id(9), scope(), binding(), time(100)),
        Err(Error::UploadRetentionFull)
    ));
    assert_eq!(
        db.claim_upload_source(h, scope(), binding()).unwrap(),
        id(7)
    );
    assert!(db.claim_upload_source(h, scope(), binding()).is_err());
    assert!(matches!(
        db.register_upload_intent(
            UploadHandle::legacy(id(9)),
            id(9),
            scope(),
            binding(),
            time(100)
        ),
        Err(Error::UploadLegacyReadOnly)
    ));
    let wronggeneration = UploadHandle::new([0; 16], 100, [1; 16]);
    assert!(matches!(
        db.register_upload_intent(wronggeneration, id(9), scope(), binding(), time(100)),
        Err(Error::UploadGenerationMismatch)
    ));
}
#[test]
fn clock_domains_are_separate_and_expired_admission_commits_a_rejection_floor() {
    let (_dir, mut db) = database();
    let old = handle(&db, 1, 1);
    let before = db.metadata().unwrap();
    assert!(matches!(
        db.register_upload_intent(old, id(1), scope(), binding(), time(200_000)),
        Err(Error::UploadExpired)
    ));
    let after = db.metadata().unwrap();
    assert!(after.revision > before.revision);
    assert_ne!(after.write_token, before.write_token);
    let clock = db
        .upload_clock(scope().host_id, scope().uid)
        .unwrap()
        .unwrap();
    assert_eq!(
        clock.reject_issued_before,
        200_000 - MAX_NEW_UPLOAD_AGE_SECONDS
    );
    assert!(db
        .adapter_clock(scope().host_id, scope().uid)
        .unwrap()
        .is_none());
    let future = handle(&db, 200_301, 2);
    assert!(matches!(
        db.register_upload_intent(future, id(2), scope(), binding(), time(200_000)),
        Err(Error::UploadFutureHandle)
    ));
    let mut restarted = time(400_000);
    restarted.process_id = [5; 16];
    assert!(matches!(
        db.upload_issuance_context(scope(), restarted),
        Err(Error::UploadClockUntrusted)
    ));
    let preview = db
        .preview_upload_clock_reanchor(scope().host_id, scope().uid, time(5))
        .unwrap();
    assert_eq!(preview.reject_issued_before, clock.reject_issued_before);
    db.reanchor_upload_clock(scope().host_id, scope().uid, &clock, time(5))
        .unwrap();
    assert!(db
        .reanchor_upload_clock(scope().host_id, scope().uid, &clock, time(5))
        .is_err());
    assert!(matches!(
        db.upload_issuance_context(scope(), time(5)),
        Err(Error::UploadClockUntrusted)
    ));
}
#[test]
fn terminal_evidence_survives_bad_clock_cleanup_cursor_and_reopen() {
    let (dir, mut db) = database();
    let h = handle(&db, 100, 1);
    register(&mut db, h, 1, 100);
    db.cancel_upload(&id(1)).unwrap();
    db.record_upload_terminal(&id(1), Some(time(u64::MAX)))
        .unwrap();
    let expires: Option<i64> = db
        .connection
        .query_row("SELECT expires_at FROM kv_upload_handles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(expires, None);
    let current = db.upload_cleanup(&id(1)).unwrap().unwrap();
    let next = db.advance_upload_cleanup(&id(1), &current).unwrap();
    assert!(db.advance_upload_cleanup(&id(1), &current).is_err());
    drop(db);
    let mut db = HardStateStore::open(&dir.path().join("hard.db")).unwrap();
    assert_eq!(db.upload_cleanup(&id(1)).unwrap(), Some(next));
    db.record_upload_terminal(&id(1), Some(time(101))).unwrap();
    let expiry = 101 + UPLOAD_TERMINAL_RETENTION_SECONDS;
    assert_eq!(
        db.prune_upload_receipts(scope().host_id, scope().uid, time(expiry), 128)
            .unwrap(),
        0
    );
    clean(&mut db, &id(1));
    assert_eq!(
        db.prune_upload_receipts(scope().host_id, scope().uid, time(expiry), 128)
            .unwrap(),
        1
    );
    assert!(db.upload_registration(h, scope()).unwrap().is_none());
    assert!(matches!(
        db.register_upload_intent(h, id(1), scope(), binding(), time(100)),
        Err(Error::UploadExpired)
    ));
}
#[test]
fn rolling_receipts_exceed_old_lifetime_cap_and_preserve_older_live_handles() {
    let (_dir, mut db) = database();
    db.set_upload_retention_limits(UploadRetentionLimits {
        max_rows: 2,
        max_bytes: 2048,
    })
    .unwrap();
    let live = handle(&db, 100, 0);
    register(&mut db, live, 0, 100);
    let mut now = 101;
    for n in 1..=1030 {
        let h = handle(&db, now, n);
        register(&mut db, h, n, now);
        db.cancel_upload(&id(n)).unwrap();
        db.record_upload_terminal(&id(n), Some(time(now))).unwrap();
        clean(&mut db, &id(n));
        now += UPLOAD_TERMINAL_RETENTION_SECONDS;
        assert_eq!(
            db.prune_upload_receipts(scope().host_id, scope().uid, time(now), 128)
                .unwrap(),
            1
        );
    }
    assert_eq!(
        db.register_upload_intent(live, id(99), scope(), binding(), time(u64::MAX))
            .unwrap()
            .internal_id,
        id(0)
    );
    let counters: (i64, i64) = db
        .connection
        .query_row(
            "SELECT reserved_rows,reserved_bytes FROM kv_upload_receipt_limits",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(counters, (1, 1024));
}
#[test]
fn summary_pages_bound_metadata_without_loading_chunk_records() {
    let (_dir, mut db) = database();
    db.set_upload_limits(UploadLimits {
        max_pending: 128,
        max_reserved_bytes: 268435456,
    })
    .unwrap();
    for n in 0..128 {
        let h = handle(&db, 100, n);
        register(&mut db, h, n, 100);
    }
    let first = db.upload_summaries_page(scope(), None, 73).unwrap();
    assert_eq!(first.items.len(), 73);
    assert!(first.next.is_some());
    let second = db.upload_summaries_page(scope(), first.next, 73).unwrap();
    assert_eq!(second.items.len(), 55);
    assert!(second.next.is_none());
    let mut ids = first
        .items
        .into_iter()
        .chain(second.items)
        .map(|s| s.internal_id)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 128);
    assert!(db.upload_summaries_page(scope(), None, 129).is_err());
}
#[test]
fn genuine_v41_migration_preserves_legacy_receipts_and_does_not_admit_unseen_legacy() {
    let (dir, mut db) = database();
    let h = handle(&db, 100, 1);
    register(&mut db, h, 1, 100);
    let before = db.metadata().unwrap();
    db.connection.execute_batch("DROP TABLE kv_upload_children; DROP TABLE kv_upload_cleanup; DROP TABLE kv_upload_clocks; DROP TABLE kv_upload_handles; DROP TABLE kv_upload_receipt_limits;").unwrap();
    db.connection
        .pragma_update(None, "user_version", 41)
        .unwrap();
    drop(db);
    let mut db = HardStateStore::open(&dir.path().join("hard.db")).unwrap();
    assert_eq!(db.metadata().unwrap().database_id, before.database_id);
    assert!(db.metadata().unwrap().revision > before.revision);
    let legacy = UploadHandle::legacy(id(1));
    assert_eq!(
        db.upload_registration(legacy, scope())
            .unwrap()
            .unwrap()
            .internal_id,
        id(1)
    );
    assert!(matches!(
        db.claim_upload_source(legacy, scope(), binding()),
        Err(Error::UploadLegacyReadOnly)
    ));
    db.cancel_upload(&id(1)).unwrap();
    clean(&mut db, &id(1));
    let before_legacy_stamp = db.metadata().unwrap();
    let before_legacy_clock = db.upload_clock(scope().host_id, scope().uid).unwrap();
    db.record_upload_terminal(&id(1), Some(time(100))).unwrap();
    assert_eq!(
        db.metadata().unwrap().revision,
        before_legacy_stamp.revision
    );
    assert_eq!(
        db.metadata().unwrap().write_token,
        before_legacy_stamp.write_token
    );
    assert_eq!(
        db.upload_clock(scope().host_id, scope().uid).unwrap(),
        before_legacy_clock
    );
    assert_eq!(
        db.prune_upload_receipts(scope().host_id, scope().uid, time(3_000_000), 128)
            .unwrap(),
        0
    );
    assert!(db.upload_registration(legacy, scope()).unwrap().is_some());
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
fn pending_rows_and_future_receipt_bytes_are_distinct_admission_budgets() {
    let (_dir, mut db) = database();
    db.set_upload_limits(UploadLimits {
        max_pending: 1,
        max_reserved_bytes: 268435456,
    })
    .unwrap();
    db.set_upload_retention_limits(UploadRetentionLimits {
        max_rows: 4,
        max_bytes: 1024,
    })
    .unwrap();
    let a = handle(&db, 100, 1);
    register(&mut db, a, 1, 100);
    let b = handle(&db, 100, 2);
    assert!(matches!(
        db.register_upload_intent(b, id(2), scope(), binding(), time(100)),
        Err(Error::UploadPendingFull)
    ));
    db.cancel_upload(&id(1)).unwrap();
    assert!(matches!(
        db.register_upload_intent(b, id(2), scope(), binding(), time(100)),
        Err(Error::UploadRetentionFull)
    ));
    db.set_upload_retention_limits(UploadRetentionLimits {
        max_rows: 1,
        max_bytes: 4096,
    })
    .unwrap();
    assert!(matches!(
        db.register_upload_intent(b, id(2), scope(), binding(), time(100)),
        Err(Error::UploadRetentionFull)
    ));
    db.set_upload_retention_limits(UploadRetentionLimits {
        max_rows: 2,
        max_bytes: 4096,
    })
    .unwrap();
    register(&mut db, b, 2, 100);
    assert!(db
        .set_upload_retention_limits(UploadRetentionLimits {
            max_rows: 1,
            max_bytes: 4096
        })
        .is_err());
    assert_eq!(db.upload_retention_limits().unwrap().max_rows, 2);
}

#[test]
fn stamped_terminal_is_a_true_noop_and_complete_receipts_still_get_stamped() {
    let (_dir, mut db) = database();
    let h = handle(&db, 100, 1);
    register(&mut db, h, 1, 100);
    db.cancel_upload(&id(1)).unwrap();
    clean(&mut db, &id(1));
    assert!(db.next_upload_cleanup(None).unwrap().is_none());
    let unstamped = db.next_upload_unstamped(None).unwrap().unwrap();
    assert_eq!(unstamped.internal_id, id(1));
    assert_eq!(unstamped.handle, h);
    db.record_upload_terminal(&id(1), Some(time(101))).unwrap();
    assert!(db.next_upload_unstamped(None).unwrap().is_none());
    let metadata = db.metadata().unwrap();
    let clock = db.upload_clock(scope().host_id, scope().uid).unwrap();
    db.record_upload_terminal(&id(1), Some(time(102))).unwrap();
    db.record_upload_terminal(&id(1), Some(time(u64::MAX)))
        .unwrap();
    let after = db.metadata().unwrap();
    assert_eq!(after.revision, metadata.revision);
    assert_eq!(after.write_token, metadata.write_token);
    assert_eq!(
        db.upload_clock(scope().host_id, scope().uid).unwrap(),
        clock
    );
    assert_eq!(
        db.next_upload_clock_account(None).unwrap(),
        Some((vec![1; 33], vec![2; 33]))
    );
    assert!(db
        .next_upload_clock_account(Some((scope().host_id, scope().uid)))
        .unwrap()
        .is_none());
    assert_eq!(
        db.prune_upload_receipts(
            scope().host_id,
            scope().uid,
            time(101 + UPLOAD_TERMINAL_RETENTION_SECONDS),
            128
        )
        .unwrap(),
        1
    );
}

// Construct real-schema retained metadata in one transaction, without spending
// filesystem fsync time per simulated historical receipt. Every owner/receipt is
// valid and charged; only source execution is omitted from these SQL cost fixtures.
fn fixture_receipt(
    c: &Connection,
    generation: [u8; 16],
    n: u64,
    host: u8,
    state: u8,
    phase: u8,
    stamped: bool,
) {
    let operation = id(n);
    let h = UploadHandle::new(generation, 100, operation);
    c.execute("INSERT INTO kv_uploads(operation_id,host_id,uid,party_id,file_id,manifest_hash,state,reserved_bytes,created_at) VALUES(?1,?2,?3,?3,zeroblob(17),zeroblob(32),?4,0,100)", params![operation, [host;33], [2u8;33], state]).unwrap();
    c.execute("INSERT INTO kv_upload_handles(handle_hash,handle,handle_version,issued_at,operation_id,intent_hash,terminal_at,expires_at,receipt_bytes) VALUES(?1,?2,1,100,?3,zeroblob(32),?4,?5,1024)", params![upload_handle_hash(h),h.to_string(),operation,stamped.then_some(101),stamped.then_some(i64::try_from(101 + UPLOAD_TERMINAL_RETENTION_SECONDS).unwrap())]).unwrap();
    c.execute(
        "INSERT INTO kv_upload_cleanup(operation_id,phase) VALUES(?1,?2)",
        params![operation, phase],
    )
    .unwrap();
}
fn vm_steps(c: &Connection, sql: &str, parameters: impl rusqlite::Params) -> i32 {
    let mut statement = c.prepare(sql).unwrap();
    statement
        .query_map(parameters, |_| Ok(()))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    statement.get_status(rusqlite::StatementStatus::VmStep)
}
fn query_plan(c: &Connection, sql: &str, parameters: impl rusqlite::Params) -> String {
    c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(parameters, |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("\n")
}

#[test]
fn maintenance_selectors_exclude_retained_history_and_seek_the_exact_internal_id() {
    let (_dir, mut db) = database();
    db.set_upload_retention_limits(UploadRetentionLimits {
        max_rows: 65536,
        max_bytes: 67108864,
    })
    .unwrap();
    let generation = db.metadata().unwrap().database_id;
    // Two actionable receipts have different owners. Selecting by scope again
    // instead of decoding the selected row would lose this exact cursor contract.
    fixture_receipt(&db.connection, generation, 1_000_000, 1, 9, 0, false);
    fixture_receipt(&db.connection, generation, 2_000_000, 3, 11, 0, false);
    fixture_receipt(&db.connection, generation, 3_000_000, 1, 9, 4, true);
    let mut inserted = 0;
    let mut baseline = None;
    for retained in [0, 100, 8192] {
        let tx = db.connection.transaction().unwrap();
        for n in inserted..retained {
            fixture_receipt(&tx, generation, n, 7, 9, 4, true);
        }
        tx.commit().unwrap();
        inserted = retained;
        let after = id(1_000_000);
        let costs = [
            vm_steps(&db.connection, CLEANUP_FIRST, []),
            vm_steps(&db.connection, CLEANUP_AFTER, [after.as_slice()]),
            vm_steps(&db.connection, UNSTAMPED_FIRST, []),
            vm_steps(&db.connection, UNSTAMPED_AFTER, [after.as_slice()]),
            vm_steps(
                &db.connection,
                PRUNE_CANDIDATES,
                params![[1u8; 33], [2u8; 33], 3_000_000i64, 128],
            ),
            vm_steps(
                &db.connection,
                PRUNE_CANDIDATES,
                params![[8u8; 33], [2u8; 33], 3_000_000i64, 128],
            ),
        ];
        eprintln!("retained_other_account={retained} selector/prune_steps={costs:?}");
        if let Some(expected) = baseline {
            assert_eq!(costs, expected);
        } else {
            baseline = Some(costs);
        }
        assert_eq!(
            db.next_upload_cleanup(None).unwrap().unwrap().internal_id,
            id(1_000_000)
        );
        let selected = db.next_upload_cleanup(Some(after)).unwrap().unwrap();
        assert_eq!(selected.internal_id, id(2_000_000));
        assert_eq!(selected.host_id, vec![3; 33]);
        assert_eq!(selected.state, UploadState::InlineUntracked);
        assert_eq!(
            db.next_upload_unstamped(Some(after)).unwrap(),
            Some(selected)
        );
        assert!(db
            .next_upload_cleanup(Some(id(2_000_000)))
            .unwrap()
            .is_none());
    }
    for (sql, index) in [
        (CLEANUP_FIRST, "kv_upload_cleanup_pending"),
        (UNSTAMPED_FIRST, "kv_upload_handles_unstamped"),
    ] {
        let plan = query_plan(&db.connection, sql, []);
        assert!(plan.contains(index), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
    assert!(
        query_plan(&db.connection, CLEANUP_AFTER, [id(1_000_000).as_slice()])
            .contains("operation_id>?")
    );
    assert!(
        query_plan(&db.connection, UNSTAMPED_AFTER, [id(1_000_000).as_slice()])
            .contains("operation_id>?")
    );
    let prune_plan = query_plan(
        &db.connection,
        PRUNE_CANDIDATES,
        params![[1u8; 33], [2u8; 33], 3_000_000i64, 128],
    );
    assert!(prune_plan.contains("kv_uploads_by_owner"), "{prune_plan}");
    // Cost is bounded by the pending limit, not independent of unresolved owners.
    db.set_upload_limits(UploadLimits {
        max_pending: 128,
        max_reserved_bytes: 268435456,
    })
    .unwrap();
    let before_pending = vm_steps(&db.connection, CLEANUP_FIRST, []);
    let mut inserted_pending = 0;
    let mut pending_steps = before_pending;
    for pending in [16, 128] {
        let tx = db.connection.transaction().unwrap();
        for n in inserted_pending..pending {
            fixture_receipt(&tx, generation, 900_000 + n, 7, 10, 0, false);
        }
        tx.commit().unwrap();
        inserted_pending = pending;
        let steps = vm_steps(&db.connection, CLEANUP_FIRST, []);
        assert!(steps > pending_steps);
        assert!(
            steps <= before_pending + 32 * pending as i32,
            "pending={pending}, steps={steps}"
        );
        let unstamped = vm_steps(&db.connection, UNSTAMPED_FIRST, []);
        eprintln!("pending={pending} cleanup_steps={steps} unstamped_steps={unstamped}");
        pending_steps = steps;
        assert_eq!(
            db.next_upload_unstamped(None).unwrap().unwrap().internal_id,
            id(1_000_000)
        );
    }
    // Account-local pruning intentionally scales with retained rows of that
    // account (up to the configured cap); report rather than hide that cost.
    let own_before = vm_steps(
        &db.connection,
        PRUNE_CANDIDATES,
        params![[1u8; 33], [2u8; 33], 3_000_000i64, 128],
    );
    let tx = db.connection.transaction().unwrap();
    for n in 0..100 {
        fixture_receipt(&tx, generation, 4_000_000 + n, 1, 9, 4, true);
    }
    tx.commit().unwrap();
    let own_after = vm_steps(
        &db.connection,
        PRUNE_CANDIDATES,
        params![[1u8; 33], [2u8; 33], 3_000_000i64, 128],
    );
    assert!(own_after > own_before);
    eprintln!("matching_account_receipts_added=100 prune_steps={own_before}->{own_after}");
}

#[test]
fn cleanup_debt_includes_unresolved_owners_and_refuses_missing_cleanup_rows() {
    let (_dir, mut db) = database();
    assert_eq!(
        db.upload_cleanup_debt().unwrap(),
        UploadCleanupDebt {
            reserved_bytes: 0,
            incomplete: 0,
            actionable: 0
        }
    );
    for n in 1..=3 {
        let h = handle(&db, 100, n);
        register(&mut db, h, n, 100);
    }
    db.connection
        .execute(
            "UPDATE kv_uploads SET reserved_bytes=123 WHERE operation_id=?1",
            [id(1)],
        )
        .unwrap();
    db.cancel_upload(&id(2)).unwrap();
    db.cancel_upload(&id(3)).unwrap();
    clean(&mut db, &id(3));
    assert_eq!(
        db.upload_cleanup_debt().unwrap(),
        UploadCleanupDebt {
            reserved_bytes: 123,
            incomplete: 2,
            actionable: 1
        }
    );
    db.connection
        .execute(
            "DELETE FROM kv_upload_cleanup WHERE operation_id=?1",
            [id(1)],
        )
        .unwrap();
    assert!(db.upload_cleanup_debt().is_err());
}
