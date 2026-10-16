use super::*;
use crate::test_support::AccountFixture as Fixture;
fn id(p: &BotEnrollmentReport) -> [u8; 16] {
    std::array::from_fn(|i| u8::from_str_radix(&p.operation_id[i * 2..i * 2 + 2], 16).unwrap())
}
#[test]
fn bot_export_replays_until_durable_completion_and_selection_survives_without_secret() {
    let f = Fixture::start();
    f.run(|s, v, k| s.create_account("work", "appbotowner", "laptop", "", "", None, v, k));
    let p = f.run(|s, v, k| s.prepare_bot_account("work", Role::OWNER, None, v, k));
    assert_eq!(p.state, "prepared");
    f.run(|s, v, k| {
        assert!(s
            .prepare_bot_account("work", Role::OWNER, None, v, k)
            .is_err());
        assert!(s
            .bot_account_enrollment("other", id(&p), BotEnrollmentAction::Attempt, None, v, k)
            .is_err());
        assert!(s
            .bot_account_enrollment("work", id(&p), BotEnrollmentAction::Export, None, v, k)
            .is_err());
        assert_eq!(s.bot_account_enrollments("work", v)?.len(), 1);
        Ok(())
    });
    f.environment
        .arm_fault(foks_server_testkit::TestFault::ProvisionAfterCommitBeforeResponse);
    f.run(|s, v, k| {
        assert!(s
            .bot_account_enrollment("work", id(&p), BotEnrollmentAction::Attempt, None, v, k)
            .is_err());
        Ok(())
    });
    let done = f.run(|s, v, k| {
        s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Attempt, None, v, k)
    });
    assert_eq!(done.report.state, "complete");
    assert!(done.report.export_available);
    let export = f
        .run(|s, v, k| {
            s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Export, None, v, k)
        })
        .exported
        .unwrap();
    f.run(|s, v, k| {
        // Each Fixture run reopens the protected store: offered state survives
        // a discarded response and resumes with exactly the original token.
        let replay =
            s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Export, None, v, k)?;
        assert_eq!(
            replay.exported.as_deref().map(String::as_str),
            Some(export.as_str())
        );
        assert!(replay.report.export_available);
        let pending = PendingBot::load(v, id(&p), "work", s.pinned_host()?.host_id())?;
        assert!(pending.export_offered);
        for _ in 0..2 {
            let completed = s.bot_account_enrollment(
                "work",
                id(&p),
                BotEnrollmentAction::CompleteExport,
                None,
                v,
                k,
            )?;
            assert!(completed.exported.is_none());
            assert!(!completed.report.export_available);
        }
        assert!(s
            .bot_account_enrollment("work", id(&p), BotEnrollmentAction::Export, None, v, k)
            .is_err());
        Ok(())
    });
    f.run(|s, v, _| {
        let loaded = s.load_bot_account("automation", &export, v)?;
        v.attach_loaded_bot(loaded)?;
        assert_eq!(
            v.account("automation")?.credential.key_kind,
            foks_client::SoftwareKeyKind::BotToken
        );
        s.sync_account("automation", v)
    });
    f.run(|s, v, k| {
        assert_eq!(v.account_display_name("automation")?, "appbotowner");
        assert!(matches!(
            v.account("automation"),
            Err(Error::BotTokenLocked)
        ));
        assert_eq!(
            s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Status, None, v, k)?
                .report
                .state,
            "complete"
        );
        for key in v.store.keys()? {
            let bytes = v.store.get(&key)?;
            assert!(!bytes.windows(export.len()).any(|w| w == export.as_bytes()));
        }
        Ok(())
    });
}

#[test]
fn hardware_owner_enrolls_and_revokes_a_bot_with_no_unattended_prompt() {
    use foks_yubi::{MockYubiProvider, Pin, SlotId, YubiProvider as _};
    let f = Fixture::start();
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("bot", 779, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    f.run(|s, v, k| {
        s.create_yubi_account(
            YubiSignupInput {
                alias: "key".into(),
                username: "botyubi".into(),
                device_name: "owner key".into(),
                email: String::new(),
                invite: String::new(),
                passphrase: None,
                card,
                signing_slot: SlotId::new(0x82)?,
                pq_slot: SlotId::new(0x83)?,
                retry_configuration: None,
            },
            Pin::new("654321")?,
            &provider,
            v,
            k,
        )
    });
    let parent = f.run(|_, v, _| Ok(provider.open(&v.yubi_account("key")?.locator, Some(&pin))?));
    let p = f.run(|s, v, k| s.prepare_bot_account("key", Role::OWNER, Some(parent.as_ref()), v, k));
    assert!(
        f.run(|s, v, k| s.bot_account_enrollment(
            "key",
            id(&p),
            BotEnrollmentAction::Attempt,
            None,
            v,
            k
        ))
        .report
        .hardware_required
    );
    let done = f.run(|s, v, k| {
        s.bot_account_enrollment(
            "key",
            id(&p),
            BotEnrollmentAction::Attempt,
            Some(parent.as_ref()),
            v,
            k,
        )
    });
    assert_eq!(done.report.state, "complete");
    let export = f
        .run(|s, v, k| {
            s.bot_account_enrollment("key", id(&p), BotEnrollmentAction::Export, None, v, k)
        })
        .exported
        .unwrap();
    let bot = f.run(|s, v, _| s.load_bot_account("automation", &export, v));
    let host = f.run(|s, _, _| s.pinned_host());
    let client = f.run(|s, _, _| Ok(s.client.clone()));
    assert_eq!(
        client.ping(&host, &bot.credential).unwrap(),
        bot.credential.uid
    );
    let revoked = f.run(|s, v, k| {
        s.revoke_bot_account_credential("key", &p.device_id, Some(parent.as_ref()), v, k)
    });
    assert!(!revoked.currently_active);
    assert!(client.ping(&host, &bot.credential).is_err());
    f.run(|s, v, _| {
        assert!(s.load_bot_account("automation", &export, v).is_err());
        Ok(())
    });
}

#[test]
fn software_revoke_lost_reply_verifies_ppe_and_keeps_original_completion_record() {
    let f = Fixture::start();
    f.run(|s, v, k| {
        s.create_account(
            "work",
            "botrevoker",
            "laptop",
            "",
            "",
            Some(Passphrase::new("bot owner passphrase")?),
            v,
            k,
        )
    });
    let p = f.run(|s, v, k| s.prepare_bot_account("work", Role::OWNER, None, v, k));
    f.run(|s, v, k| {
        s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Attempt, None, v, k)
    });
    let token = f
        .run(|s, v, k| {
            s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Export, None, v, k)
        })
        .exported
        .unwrap();
    f.environment
        .arm_fault(foks_server_testkit::TestFault::RevokeAfterCommitBeforeResponse);
    let done = f.run(|s, v, k| s.revoke_bot_account_credential("work", &p.device_id, None, v, k));
    assert_eq!(done.state, "complete");
    assert!(!done.currently_active);
    f.run(|s, v, _k| {
        let operation = entity_handle(done.operation_id.as_ref().unwrap());
        let op = HardStateStore::open(&s.paths.hard_database)?
            .mutation(&operation)?
            .unwrap();
        assert_eq!(op.attempt_count, 1);
        assert_eq!(op.state, MutationState::Finalized);
        s.verify_passphrase("work", Passphrase::new("bot owner passphrase")?, v)?;
        assert!(s.load_bot_account("bad", &token, v).is_err());
        Ok(())
    });
}
fn entity_handle(text: &str) -> [u8; 16] {
    std::array::from_fn(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap())
}
#[test]
fn completion_record_cleanup_retains_pending_and_unexported_tokens() {
    let f = Fixture::start();
    f.run(|s, v, k| s.create_account("work", "botcleanup", "laptop", "", "", None, v, k));
    let p = f.run(|s, v, k| s.prepare_bot_account("work", Role::OWNER, None, v, k));
    for complete in [false, true] {
        if complete {
            f.run(|s, v, k| {
                s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Attempt, None, v, k)
            });
        }
        f.run(|s, v, k| {
            let uid = v.account("work")?.credential.uid;
            s.clean_bot_completion_records(uid.as_bytes(), v, k, u64::MAX / 2)?;
            assert_eq!(s.bot_account_enrollments("work", v)?.len(), 1);
            Ok(())
        });
    }
    f.run(|s, v, k| {
        s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Export, None, v, k)
    });
    f.run(|s, v, k| {
        let uid = v.account("work")?.credential.uid;
        s.clean_bot_completion_records(uid.as_bytes(), v, k, u64::MAX / 2)?;
        assert_eq!(s.bot_account_enrollments("work", v)?.len(), 1);
        s.bot_account_enrollment(
            "work",
            id(&p),
            BotEnrollmentAction::CompleteExport,
            None,
            v,
            k,
        )?;
        s.clean_bot_completion_records(uid.as_bytes(), v, k, u64::MAX / 2)?;
        assert!(s.bot_account_enrollments("work", v)?.is_empty());
        assert!(HardStateStore::open(&s.paths.hard_database)?
            .mutation(&id(&p))?
            .is_none());
        Ok(())
    });
}

#[test]
fn export_completion_refuses_unoffered_and_cancelled_without_consuming_token() {
    let f = Fixture::start();
    f.run(|s, v, k| s.create_account("work", "botackowner", "laptop", "", "", None, v, k));
    let p = f.run(|s, v, k| s.prepare_bot_account("work", Role::OWNER, None, v, k));
    for finalized in [false, true] {
        if finalized {
            f.run(|s, v, k| {
                s.bot_account_enrollment("work", id(&p), BotEnrollmentAction::Attempt, None, v, k)
            });
        }
        f.run(|s, v, k| {
            let before = v.store.get(&enrollment_key(&id(&p)))?;
            let op = HardStateStore::open(&s.paths.hard_database)?.mutation(&id(&p))?;
            assert!(s
                .bot_account_enrollment(
                    "work",
                    id(&p),
                    BotEnrollmentAction::CompleteExport,
                    None,
                    v,
                    k
                )
                .is_err());
            assert_eq!(
                v.store.get(&enrollment_key(&id(&p)))?.as_slice(),
                before.as_slice()
            );
            let after = HardStateStore::open(&s.paths.hard_database)?.mutation(&id(&p))?;
            assert_eq!(
                after.as_ref().map(|o| (o.state, o.attempt_count)),
                op.as_ref().map(|o| (o.state, o.attempt_count))
            );
            Ok(())
        });
    }
    // Simulate a pre-upgrade successful export lacking the new offered field.
    f.run(|s, v, k| {
        let key = enrollment_key(&id(&p));
        let mut raw: serde_json::Value = serde_json::from_slice(&v.store.get(&key)?)?;
        raw["token"] = serde_json::Value::Null;
        raw.as_object_mut().unwrap().remove("export_offered");
        v.store
            .put(&key, &Zeroizing::new(serde_json::to_vec(&raw)?))?;
        let completed = s.bot_account_enrollment(
            "work",
            id(&p),
            BotEnrollmentAction::CompleteExport,
            None,
            v,
            k,
        )?;
        assert!(!completed.report.export_available);
        Ok(())
    });
    let cancelled = f.run(|s, v, k| s.prepare_bot_account("work", Role::OWNER, None, v, k));
    f.run(|s, v, k| {
        s.bot_account_enrollment(
            "work",
            id(&cancelled),
            BotEnrollmentAction::Cancel,
            None,
            v,
            k,
        )?;
        assert!(s
            .bot_account_enrollment(
                "work",
                id(&cancelled),
                BotEnrollmentAction::CompleteExport,
                None,
                v,
                k
            )
            .is_err());
        Ok(())
    });
}
