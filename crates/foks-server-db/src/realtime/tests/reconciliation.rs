use super::*;
use RealtimeReconcileOutcome::{AlreadyClean, PageComplete, PageIncomplete};
use RealtimeReconcileState::{Clean, Dirty, Incomplete, Missing};

fn state(f: &Fixture) -> RealtimeInboxState {
    f.reader()
        .snapshot()
        .unwrap()
        .rt_inbox_state(&f.member, RtAppId::Chat, 300)
        .unwrap()
}
fn reconcile(f: &mut Fixture) -> RealtimeReconcileReport {
    f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 300)
        .unwrap()
        .value
}
fn clean(f: &mut Fixture) {
    reconcile(f);
    assert_eq!(state(f).reconciliation, Clean);
}

#[test]
fn cold_empty_clean_and_activity_do_not_repeat_reconciliation() {
    let mut f = Fixture::new();
    assert_eq!(state(&f).reconciliation, Missing);
    assert_eq!(reconcile(&mut f).outcome, PageComplete);
    assert_eq!(reconcile(&mut f).outcome, AlreadyClean);
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    assert_eq!(state(&f).reconciliation, Clean);
    let message = f.send(2);
    f.db.rt_send(&f.owner, &message, 200).unwrap();
    f.db.rt_read_through(
        &f.member,
        &RtReadThroughArgument {
            read: RtReadThrough {
                channel: f.create.metadata.id,
                sequence: 1,
            },
        },
        200,
    )
    .unwrap();
    assert_eq!(state(&f).reconciliation, Clean);
    assert_eq!(
        reconcile(&mut f),
        RealtimeReconcileReport {
            outcome: AlreadyClean,
            restarted: false,
            candidates: 0,
            accessibility_changes: 0,
        }
    );
    // Clean reads still authenticate the presented device and certificate.
    let mut expired = f.member.clone();
    expired.certificate_expires_at = 300;
    assert!(matches!(
        f.reader()
            .snapshot()
            .unwrap()
            .rt_inbox_state(&expired, RtAppId::Chat, 300),
        Err(Error::AuthorizationChanged)
    ));
    f.db.connection
        .execute(
            "UPDATE devices SET active=0 WHERE device_id=?1",
            params![f.member.credential],
        )
        .unwrap();
    assert!(matches!(
        f.reader()
            .snapshot()
            .unwrap()
            .rt_inbox_state(&f.member, RtAppId::Chat, 300),
        Err(Error::AuthorizationChanged)
    ));
    assert!(matches!(
        f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 300),
        Err(Error::AuthorizationChanged)
    ));
    assert!(f
        .reader()
        .snapshot()
        .unwrap()
        .rt_inbox_state(&f.owner, RtAppId::Chat, 300)
        .is_ok());
}

#[test]
fn invalidation_tracks_access_columns_and_both_parties_but_not_key_updates() {
    for assignment in [
        "scoped_host_id=zeroblob(33)",
        "source_role_type=2",
        "source_visibility=-1",
        "role_type=2",
        "visibility=-1",
    ] {
        let mut f = Fixture::new();
        clean(&mut f);
        f.db.connection
            .execute(
                &format!("UPDATE team_members SET {assignment} WHERE party_id=?1"),
                params![f.member.uid],
            )
            .unwrap();
        assert_eq!(state(&f).reconciliation, Dirty, "{assignment}");
        assert!(reconcile(&mut f).restarted);
    }
    let mut f = Fixture::new();
    clean(&mut f);
    for assignment in [
        "generation=generation+1",
        "verify_key=zeroblob(33)",
        "hepk_fingerprint=zeroblob(32)",
        "role_type=role_type",
    ] {
        f.db.connection
            .execute(
                &format!("UPDATE team_members SET {assignment} WHERE party_id=?1"),
                params![f.member.uid],
            )
            .unwrap();
        assert_eq!(state(&f).reconciliation, Clean, "{assignment}");
    }
    // Host changes invalidate members even without changing any roster row.
    f.db.connection
        .execute("UPDATE teams SET host_id=zeroblob(33)", [])
        .unwrap();
    assert_eq!(state(&f).reconciliation, Dirty);
    clean(&mut f);
    f.db.rt_reconcile_inbox(&f.owner, RtAppId::Chat, 300)
        .unwrap();
    f.db.connection
        .execute(
            "DELETE FROM team_members WHERE party_id=?1",
            params![f.owner.uid],
        )
        .unwrap();
    f.db.rt_reconcile_inbox(&f.owner, RtAppId::Chat, 300)
        .unwrap();
    f.db.connection
        .execute(
            "UPDATE team_members SET party_id=?1 WHERE party_id=?2",
            params![f.owner.uid, f.member.uid],
        )
        .unwrap();
    assert_eq!(state(&f).reconciliation, Dirty);
    assert_eq!(
        f.reader()
            .snapshot()
            .unwrap()
            .rt_inbox_state(&f.owner, RtAppId::Chat, 300)
            .unwrap()
            .reconciliation,
        Dirty
    );
}

#[test]
fn bounded_pages_continue_without_wakes_restart_on_edits_and_preserve_reads() {
    let mut f = Fixture::new();
    for i in 1..=3 {
        let mut create = f.create.clone();
        create.metadata.id = RtChannelId([i; 16]);
        create.metadata.updated_at = i as u64;
        create.set_version = i as u64;
        f.db.rt_create_channel(&f.owner, &create, 100).unwrap();
    }
    let message = f.send(9);
    f.db.rt_send(&f.owner, &message, 200).unwrap();
    f.db.rt_read_through(
        &f.member,
        &RtReadThroughArgument {
            read: RtReadThrough {
                channel: f.create.metadata.id,
                sequence: 1,
            },
        },
        200,
    )
    .unwrap();
    for i in 0..3 {
        let commit =
            f.db.rt_reconcile_inbox_page(&f.member, RtAppId::Chat, 300, 1)
                .unwrap();
        assert!(commit.wake.is_empty());
        assert_eq!(commit.value.accessibility_changes, 0);
        assert_eq!(commit.value.restarted, i == 0);
        assert_eq!(
            commit.value.outcome,
            if i == 2 { PageComplete } else { PageIncomplete }
        );
        assert_eq!(
            state(&f).reconciliation,
            if i == 2 { Clean } else { Incomplete }
        );
    }
    f.db.connection.execute_batch("CREATE TEMP TABLE saved_member AS SELECT * FROM team_members WHERE role_type=1; DELETE FROM team_members WHERE role_type=1;").unwrap();
    let first =
        f.db.rt_reconcile_inbox_page(&f.member, RtAppId::Chat, 300, 1)
            .unwrap();
    assert_eq!(first.value.accessibility_changes, 1);
    assert_eq!(state(&f).reconciliation, Incomplete);
    f.db.connection
        .execute("INSERT INTO team_members SELECT * FROM saved_member", [])
        .unwrap();
    assert_eq!(state(&f).reconciliation, Dirty);
    let restarted =
        f.db.rt_reconcile_inbox_page(&f.member, RtAppId::Chat, 300, 1)
            .unwrap();
    assert!(restarted.value.restarted);
    assert_eq!(restarted.value.accessibility_changes, 1);
    let read: i64 =
        f.db.connection
            .query_row(
                "SELECT read_through FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
                params![f.member.uid, f.create.metadata.id.0.as_slice()],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(read, 1);
    // Continuation and dirtiness survive reopening the writer.
    let path = f.db.path.clone();
    drop(f.db);
    f.db = Database::open_existing(path, Config::default()).unwrap();
    assert_eq!(state(&f).reconciliation, Incomplete);
    assert!(!reconcile(&mut f).restarted);
    assert_eq!(state(&f).reconciliation, Clean);
}

#[test]
fn membership_rollback_and_foreign_parties_do_not_invalidate_or_create_inboxes() {
    let mut f = Fixture::new();
    clean(&mut f);
    f.db.connection
        .execute_batch("BEGIN; DELETE FROM team_members; ROLLBACK;")
        .unwrap();
    assert_eq!(state(&f).reconciliation, Clean);
    let before = f.count("rt_user_inboxes");
    f.db.connection.execute("INSERT INTO team_members SELECT team_id,zeroblob(33),scoped_host_id,source_role_type,source_visibility,role_type,visibility,generation,verify_key,hepk_fingerprint,removal_key_commitment FROM team_members LIMIT 1", []).unwrap();
    assert_eq!(f.count("rt_user_inboxes"), before);
    assert_eq!(state(&f).reconciliation, Clean);
}

// Turn a test-only fresh database into the exact previous table layout.
fn legacy(f: &mut Fixture) {
    f.db.connection.execute_batch("DROP TRIGGER rt_membership_insert; DROP TRIGGER rt_membership_delete; DROP TRIGGER rt_membership_update; DROP TRIGGER rt_team_access_update; ALTER TABLE rt_user_inboxes DROP COLUMN reconcile_dirty; PRAGMA user_version=43;").unwrap();
}
#[test]
fn migration_preserves_inbox_data_invalidates_legacy_state_and_readers_refuse_upgrade() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    let message = f.send(2);
    f.db.rt_send(&f.owner, &message, 200).unwrap();
    clean(&mut f);
    let head = state(&f).version;
    legacy(&mut f);
    f.db.connection
        .execute(
            "UPDATE rt_user_inboxes SET reconcile_memberships=X'0102',reconcile_after=zeroblob(16)",
            [],
        )
        .unwrap();
    let path = f.db.path.clone();
    drop(f.db);
    assert!(matches!(
        ReadDatabase::open(&path, Config::default()),
        Err(Error::SchemaVersion { found: 43 })
    ));
    f.db = Database::open(&path, Config::default()).unwrap();
    assert_eq!(
        state(&f),
        RealtimeInboxState {
            version: head,
            reconciliation: Dirty
        }
    );
    assert_eq!(f.count("rt_messages"), 1);
    assert_eq!(f.count("rt_channels"), 1);
    assert_eq!(reconcile(&mut f).accessibility_changes, 0);
    f.db.connection
        .execute(
            "DELETE FROM team_members WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    assert_eq!(state(&f).reconciliation, Dirty);
    assert_eq!(reconcile(&mut f).accessibility_changes, 1);
    assert!(f.db.integrity_check().unwrap());
    let fresh = Fixture::new();
    let triggers = |c: &Connection| {
        c.prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' AND (name LIKE 'rt_membership_%' OR name='rt_team_access_update') ORDER BY name").unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap()
            .collect::<std::result::Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(triggers(&f.db.connection), triggers(&fresh.db.connection));
    let columns = |c: &Connection| {
        c.prepare("SELECT name,type,\"notnull\",dflt_value,pk FROM pragma_table_info('rt_user_inboxes') ORDER BY cid").unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?))).unwrap()
            .collect::<std::result::Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(columns(&f.db.connection), columns(&fresh.db.connection));
}
#[test]
fn failed_upgrade_rolls_back_column_triggers_and_version() {
    let mut f = Fixture::new();
    legacy(&mut f);
    f.db.connection
        .execute_batch(
            "CREATE TRIGGER rt_membership_delete AFTER DELETE ON team_members BEGIN SELECT 1; END;",
        )
        .unwrap();
    let path = f.db.path.clone();
    drop(f.db);
    assert!(Database::open(&path, Config::default()).is_err());
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        43
    );
    assert!(c
        .prepare("SELECT reconcile_dirty FROM rt_user_inboxes")
        .is_err());
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='rt_membership_insert'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn team_moves_kind_changes_and_repeated_edits_reset_pending_work() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    clean(&mut f);
    f.db.connection.execute_batch("UPDATE teams SET team_kind=20,normalized_name=NULL,team_name_sequence=0,team_name_commitment_key=NULL;").unwrap();
    assert_eq!(state(&f).reconciliation, Dirty);
    assert_eq!(reconcile(&mut f).accessibility_changes, 1);
    f.db.connection.execute_batch("INSERT INTO teams SELECT zeroblob(33),team_kind,host_id,normalized_name,team_name_utf8,team_name_sequence,team_name_commitment_key,member_load_floor_type,member_load_floor_visibility,created_at FROM teams;").unwrap();
    f.db.connection
        .execute(
            "UPDATE team_members SET team_id=zeroblob(33) WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    assert_eq!(state(&f).reconciliation, Dirty);
    // Simulate an incomplete pass. A new edit must discard its cursor and old blob.
    f.db.connection.execute("UPDATE rt_user_inboxes SET reconcile_dirty=0,reconcile_after=zeroblob(16),reconcile_memberships=X'01'", []).unwrap();
    f.db.connection
        .execute(
            "UPDATE team_members SET scoped_host_id=zeroblob(33) WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    let row: (i64, Option<Vec<u8>>, Option<Vec<u8>>) = f.db.connection.query_row("SELECT reconcile_dirty,reconcile_after,reconcile_memberships FROM rt_user_inboxes WHERE uid=?1", params![f.member.uid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(row, (1, None, None));
    let before = f.db.connection.total_changes();
    f.db.connection
        .execute(
            "UPDATE team_members SET visibility=visibility-1 WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    assert_eq!(
        f.db.connection.total_changes() - before,
        1,
        "already-dirty inbox was rewritten"
    );
}

#[test]
fn reconciliation_by_another_device_does_not_authorize_revoked_credential() {
    let mut f = Fixture::new();
    let mut second = f.member.clone();
    second.credential = id(ENTITY_DEVICE, 9);
    f.db.connection.execute(
        "INSERT INTO devices(device_id,uid,active,role_type,visibility,hepk_fingerprint,self_token,exact_hepk,exact_name)
         VALUES (?1,?2,1,3,0,zeroblob(32),?3,X'00',X'00')",
        params![second.credential, second.uid, vec![9u8;17]],
    ).unwrap();
    f.db.connection
        .execute(
            "UPDATE devices SET active=0 WHERE device_id=?1",
            params![f.member.credential],
        )
        .unwrap();
    f.db.rt_reconcile_inbox(&second, RtAppId::Chat, 300)
        .unwrap();
    assert_eq!(
        f.reader()
            .snapshot()
            .unwrap()
            .rt_inbox_state(&second, RtAppId::Chat, 300)
            .unwrap()
            .reconciliation,
        Clean
    );
    assert!(matches!(
        f.reader()
            .snapshot()
            .unwrap()
            .rt_inbox_state(&f.member, RtAppId::Chat, 300),
        Err(Error::AuthorizationChanged)
    ));
    assert!(matches!(
        f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 300),
        Err(Error::AuthorizationChanged)
    ));
}

#[test]
fn channel_created_behind_partial_cursor_is_already_fanned_out() {
    let mut f = Fixture::new();
    for i in 1..=2 {
        let mut create = f.create.clone();
        create.metadata.id = RtChannelId([i; 16]);
        create.metadata.updated_at = i as u64;
        create.set_version = i as u64;
        f.db.rt_create_channel(&f.owner, &create, 100).unwrap();
    }
    assert_eq!(
        f.db.rt_reconcile_inbox_page(&f.member, RtAppId::Chat, 300, 1)
            .unwrap()
            .value
            .outcome,
        PageIncomplete
    );
    let mut create = f.create.clone();
    let mut id = [0; 16];
    id[15] = 9;
    create.metadata.id = RtChannelId(id);
    create.metadata.updated_at = 3;
    create.set_version = 3;
    f.db.rt_create_channel(&f.owner, &create, 300).unwrap();
    assert_eq!(state(&f).reconciliation, Incomplete);
    let page =
        f.db.rt_reconcile_inbox_page(&f.member, RtAppId::Chat, 300, 1)
            .unwrap()
            .value;
    assert_eq!(page.outcome, PageComplete);
    assert!(!page.restarted);
    assert_eq!(page.accessibility_changes, 0);
    let delta = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(
            &f.member,
            &RtGetChangedThreadsArgument {
                query: RtChangedThreads {
                    app: RtAppId::Chat,
                    since: 0,
                    maximum: 100,
                },
            },
            300,
        )
        .unwrap();
    assert_eq!(delta.channels.len(), 3);
    assert!(delta
        .channels
        .iter()
        .any(|c| c.metadata.id == create.metadata.id));
}
