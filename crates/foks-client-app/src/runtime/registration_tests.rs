use super::registration_fixture::*;
use super::*;
use foks_client_db::registration_test_support::observe;
use foks_keystore::{MemorySecretStore, SecretStore};

#[test]
fn repeated_inventory_batches_deduplicate_users_and_preserve_first_run_times() {
    let f = Fixture::new(true);
    let checked = crate::CheckedProfileSession {
        session: &f.session,
    };
    let mut secrets = CountingStore::new(MemorySecretStore::default());
    for id in 1..=10 {
        let c = credential(id);
        for alias in [format!("user{id}"), format!("duplicate{id}")] {
            AccountVault::new(&mut secrets)
                .commit_created(&alias, "synthetic", &c)
                .unwrap();
        }
    }
    for (now, writes) in [(100, 1), (200, 0)] {
        secrets.reset();
        let (result, work) = observe(|| {
            checked.ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), now)
        });
        result.unwrap();
        assert_eq!((secrets.scans, secrets.account_reads), (1, 20));
        assert_eq!(
            (
                work.database_opens,
                work.batches,
                work.read_transactions,
                work.write_transactions
            ),
            (1, 1, 1, writes)
        );
    }
    let store = HardStateStore::open(&f.session.paths.hard_database).unwrap();
    for id in 1..=10 {
        for (kind, interval) in [
            (
                USER_REFRESH_JOB_TYPE_ID,
                DEFAULT_USER_REFRESH_INTERVAL_MICROS,
            ),
            (
                TEAM_REFRESH_JOB_TYPE_ID,
                DEFAULT_TEAM_REFRESH_INTERVAL_MICROS,
            ),
        ] {
            let job = store
                .scheduled_job(&refresh_job_id(kind, &f.host, &credential(id).uid))
                .unwrap()
                .unwrap();
            assert_eq!(job.next_run_at, 100 + interval);
        }
    }
}

#[test]
fn empty_pending_and_invalid_yubi_inventories_do_not_require_a_host() {
    let f = Fixture::new(false);
    let checked = crate::CheckedProfileSession {
        session: &f.session,
    };
    let mut secrets = CountingStore::new(MemorySecretStore::default());
    for key in [
        "pending-yubi.pending",
        "yubi-account.corrupt",
        "backup.other",
        "account-not-a-prefix",
    ] {
        secrets.put(key, b"invalid").unwrap();
    }
    let (result, work) =
        observe(|| checked.ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100));
    result.unwrap();
    assert_eq!(secrets.scans, 1);
    assert_eq!(work.database_opens, 0);
    secrets.inner = MemorySecretStore::default();
    secrets.reset();
    checked
        .ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
        .unwrap();
    assert_eq!(secrets.scans, 1);
}

#[test]
fn bot_precedence_and_malformed_records_retain_existing_error_policy() {
    let f = Fixture::new(false);
    let checked = crate::CheckedProfileSession {
        session: &f.session,
    };
    let mut secrets = CountingStore::new(MemorySecretStore::default());
    let mut bot = vec![2; 33];
    bot[0] = foks_proto::ENTITY_BOT_TOKEN_KEY;
    for alias in ["onlybot", "shared"] {
        let selection = crate::BotSelection {
            alias: alias.into(),
            username: "synthetic".into(),
            host_id: f.host.as_bytes().to_vec(),
            uid: credential(1).uid.as_bytes().to_vec(),
            device_id: bot.clone(),
        };
        secrets
            .put(
                &format!("bot-account.{alias}"),
                &serde_json::to_vec(&selection).unwrap(),
            )
            .unwrap();
    }
    // Even a malformed software record is ignored when a valid bot selects the alias.
    secrets.put("account.shared", b"invalid").unwrap();
    let (result, work) =
        observe(|| checked.ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100));
    result.unwrap();
    assert_eq!(work.database_opens, 0);
    assert_eq!(secrets.account_reads, 0);
    secrets.put("bot-account.shared", b"invalid").unwrap();
    assert!(checked
        .ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
        .is_err());
    secrets.remove("bot-account.shared").unwrap();
    assert!(checked
        .ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
        .is_err());
}

#[test]
fn overflow_and_late_bad_account_cannot_leave_partial_defaults() {
    let f = Fixture::new(true);
    let checked = crate::CheckedProfileSession {
        session: &f.session,
    };
    let c = credential(1);
    // User delay fits, team delay overflows: build both before persisting either.
    let now = u64::MAX - DEFAULT_USER_REFRESH_INTERVAL_MICROS;
    assert!(checked
        .register_default_refresh_jobs_for(&c.uid, now)
        .is_err());
    let mut secrets = MemorySecretStore::default();
    AccountVault::new(&mut secrets)
        .commit_created("a-valid", "synthetic", &c)
        .unwrap();
    secrets.put("account.z-invalid", b"invalid").unwrap();
    assert!(checked
        .ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
        .is_err());
    assert_eq!(
        HardStateStore::open(&f.session.paths.hard_database)
            .unwrap()
            .next_scheduled_run()
            .unwrap(),
        None
    );
}

#[test]
fn separate_checked_slices_repair_new_rebound_and_deleted_defaults_after_restart() {
    let f = Fixture::new(true);
    let master = f.credentials.master_key().unwrap();
    let mut secrets = MemorySecretStore::default();
    let mut user = credential(1);
    AccountVault::new(&mut secrets)
        .commit_created("primary", "synthetic", &user)
        .unwrap();
    // Vault commit deliberately omits lifecycle registration, as in a crash gap.
    for slice in 0..3 {
        let session = crate::ProfileSession::open(&f.registry, "local").unwrap();
        let (result, work) = observe(|| {
            f.credentials.with_checked_session(&session, |checked| {
                checked.run_next_due_job_with_federation(
                    100,
                    &mut AccountVault::new(&mut secrets),
                    &f.registry,
                    &f.credentials,
                    &master,
                )
            })
        });
        assert!(result.unwrap().runs.is_empty());
        assert_eq!((work.database_opens, work.write_transactions), (1, 1));
        let id = refresh_job_id(USER_REFRESH_JOB_TYPE_ID, &f.host, &user.uid);
        assert!(HardStateStore::open(&session.paths.hard_database)
            .unwrap()
            .scheduled_job(&id)
            .unwrap()
            .is_some());
        if slice == 0 {
            user = credential(2);
            AccountVault::new(&mut secrets)
                .commit_created("primary", "synthetic", &user)
                .unwrap();
        } else if slice == 1 {
            f.credentials
                .with_checked_session(&session, |checked| {
                    HardStateStore::open(&checked.paths.hard_database)?
                        .remove_scheduled_job(&id)?;
                    Ok::<_, crate::Error>(())
                })
                .unwrap();
        }
    }
    // An operation error after repair still publishes state; the next slice
    // revalidates inventory rather than trusting a remembered success flag.
    let result = f.credentials.with_checked_session(&f.session, |checked| {
        checked.ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)?;
        Err::<(), _>(crate::Error::InvalidConfig("injected post-repair failure"))
    });
    assert!(result.is_err());
    let (result, work) = observe(|| {
        f.credentials.with_checked_session(&f.session, |checked| {
            checked.ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
        })
    });
    result.unwrap();
    assert_eq!((work.database_opens, work.write_transactions), (1, 0));
}

#[test]
fn teams_capability_is_rechecked_and_newly_available_defaults_are_repaired() {
    let mut f = Fixture::new(true);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let seed = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    let artifact = foks_compat_artifact::CompatibilityArtifact {
        schema_version: foks_compat_artifact::SCHEMA_VERSION,
        generation: 1,
        target: "foks.app".into(),
        run_id: "registration-test".into(),
        generated_at: now,
        expires_at: now + 3600,
        protocol_metadata_sha256: crate::PINNED_PROTOCOL_METADATA_SHA256.into(),
        mutation_digest: "22".repeat(32),
        read_digest: "33".repeat(32),
        outcome: crate::CompatibilityOutcome::Compatible,
        capabilities: std::collections::BTreeSet::from(["user-sync".into()]),
        drift_reason: String::new(),
    };
    f.session.profile.protocol = crate::ProtocolPolicy::CurrentValidated {
        compatibility_artifact_public_key:
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into(),
        lease_url: "https://updates.example.test/foks/compatibility-artifact.json".into(),
        artifact: Box::new(
            foks_compat_artifact::SignedCompatibilityArtifact::sign(artifact, &seed).unwrap(),
        ),
    };
    let user = credential(1).uid;
    crate::CheckedProfileSession {
        session: &f.session,
    }
    .register_default_refresh_jobs_for(&user, 100)
    .unwrap();
    let store = HardStateStore::open(&f.session.paths.hard_database).unwrap();
    assert!(store
        .scheduled_job(&refresh_job_id(USER_REFRESH_JOB_TYPE_ID, &f.host, &user))
        .unwrap()
        .is_some());
    assert!(store
        .scheduled_job(&refresh_job_id(TEAM_REFRESH_JOB_TYPE_ID, &f.host, &user))
        .unwrap()
        .is_none());
    f.session.profile.protocol = crate::ProtocolPolicy::V019;
    let (result, work) = observe(|| {
        crate::CheckedProfileSession {
            session: &f.session,
        }
        .register_default_refresh_jobs_for(&user, 200)
    });
    result.unwrap();
    assert_eq!(work.write_transactions, 1);
    assert_eq!(
        store
            .scheduled_job(&refresh_job_id(TEAM_REFRESH_JOB_TYPE_ID, &f.host, &user))
            .unwrap()
            .unwrap()
            .next_run_at,
        200 + DEFAULT_TEAM_REFRESH_INTERVAL_MICROS
    );
}

#[test]
fn software_and_yubi_aliases_share_one_default_pair_without_loading_pending_records() {
    let f = Fixture::new(true);
    let c = credential(1);
    let mut secrets = CountingStore::new(MemorySecretStore::default());
    AccountVault::new(&mut secrets)
        .commit_created("software", "synthetic", &c)
        .unwrap();
    let seed = foks_proto::SecretSeed::new([0x41; 32]);
    let pq = [0x42; 33];
    let locator = foks_yubi::YubiDeviceLocator {
        card: foks_yubi::CardId {
            name: "synthetic".into(),
            serial: 1,
        },
        signing_slot: foks_yubi::SlotId::new(0x82).unwrap(),
        pq_slot: foks_yubi::SlotId::new(0x83).unwrap(),
        signing_public_key: [0x44; 33],
        pq_public_key: pq,
        pq_key_id: foks_crypto::yubi_pq_key_id(&pq).unwrap(),
    };
    let record = serde_json::json!({"version":1,"alias":"hardware","username":"synthetic","uid":c.uid.as_bytes(),"locator":locator,"subkey_id":foks_crypto::derive_subkey_id(&seed).unwrap().as_bytes(),"subkey_seed":seed.as_bytes(),"certificate_chain":[[0x45]],"management_key":null,"pending_management_key":null,"management_enrolled":false,"management_generation":null,"management_refresh_source":null});
    secrets
        .put(
            "yubi-account.hardware",
            &serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    secrets
        .put(
            "pending-yubi.hardware",
            b"invalid pending record is not loaded",
        )
        .unwrap();
    secrets
        .put(
            "pending-yubi.onlypending",
            b"invalid pending record is not loaded",
        )
        .unwrap();
    assert_eq!(
        AccountVault::new(&mut secrets)
            .yubi_account("hardware")
            .unwrap()
            .uid,
        c.uid
    );
    secrets.reset();
    let checked = crate::CheckedProfileSession {
        session: &f.session,
    };
    let (result, work) =
        observe(|| checked.ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100));
    result.unwrap();
    assert_eq!(secrets.scans, 1);
    assert_eq!(secrets.account_reads, 3); // software, hardware, missing completed record
    assert_eq!((work.database_opens, work.write_transactions), (1, 1));
    let c = rusqlite::Connection::open(&f.session.paths.hard_database).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM scheduled_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn registration_work_counts_scale_with_slices_not_aliases_or_users() {
    for users in [1u8, 10, 50] {
        for due in [0u8, 1, 10, 16] {
            for present in [false, true] {
                let f = Fixture::new(true);
                let checked = crate::CheckedProfileSession {
                    session: &f.session,
                };
                let mut secrets = CountingStore::new(MemorySecretStore::default());
                for id in 1..=users {
                    for alias in [format!("user{id}"), format!("duplicate{id}")] {
                        AccountVault::new(&mut secrets)
                            .commit_created(&alias, "synthetic", &credential(id))
                            .unwrap();
                    }
                }
                if present {
                    checked
                        .ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
                        .unwrap();
                }
                let scheduler = FoksScheduler::new(
                    &f.session.paths.hard_database,
                    SchedulerConfig {
                        claim_limit: 1,
                        jitter_percent: 0,
                        ..Default::default()
                    },
                )
                .unwrap();
                for id in 0..due {
                    scheduler
                        .register(ScheduledJobRegistration {
                            job_id: [0xa0 + id; 16],
                            kind: ScheduledJobKind::MutationReconcile,
                            host_id: f.host.as_bytes().to_vec(),
                            scope_id: vec![id; 16],
                            interval_micros: 10000,
                            first_run_at: 100,
                            registered_at: 1,
                        })
                        .unwrap();
                }
                secrets.reset();
                let (slices, work) = observe(|| {
                    let mut slices = 0;
                    for _ in 0..16 {
                        checked
                            .ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
                            .unwrap();
                        slices += 1;
                        if scheduler.run_due(100, |_| Ok(())).unwrap().runs.is_empty() {
                            break;
                        }
                    }
                    slices
                });
                assert_eq!(slices, (due as usize + 1).min(16));
                assert_eq!(
                    (work.database_opens, work.batches, work.read_transactions),
                    (slices, slices, slices)
                );
                assert_eq!(work.write_transactions, usize::from(!present));
                assert_eq!(secrets.scans, slices);
                assert_eq!(secrets.account_reads, users as usize * 2 * slices);
            }
        }
    }
}
