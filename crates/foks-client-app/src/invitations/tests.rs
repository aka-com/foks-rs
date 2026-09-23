use super::*;
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;
struct Fixture {
    _environment: TestEnvironment,
    _server: foks_server_testkit::InProcessServer,
    credentials: ClientCredentials,
    registry: ProfileRegistry,
}
impl Fixture {
    fn start() -> Self {
        let environment = TestEnvironment::new().unwrap();
        let server = environment.start_server().unwrap();
        let state = environment.client_path("sso-app", "state").unwrap();
        let root = environment.client_path("sso-app", "root.der").unwrap();
        environment.write_probe_root(&root).unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                label: None,
                probe: format!(
                    "localhost:{}",
                    environment.addresses().unwrap().probe.port()
                ),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let f = Self {
            _environment: environment,
            _server: server,
            credentials,
            registry,
        };
        f.run(|s, _, _| {
            s.probe_and_pin()?;
            Ok(())
        });
        f
    }
    fn run<T>(
        &self,
        op: impl FnOnce(&CheckedProfileSession<'_>, &mut AccountVault<'_>, &[u8; 32]) -> Result<T>,
    ) -> T {
        // Reopen the profile, vault and protected store for every foreground action.
        let session = ProfileSession::open(&self.registry, "local").unwrap();
        self.credentials
            .with_checked_session(&session, |s| {
                let master = self.credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &s.paths.credential_store,
                    derive_vault_key(&master),
                )?;
                op(s, &mut AccountVault::new(&mut store), &master)
            })
            .unwrap()
    }
}

fn action(f: &Fixture, alias: &str, a: InvitationAction) -> serde_json::Value {
    f.run(|s, v, k| s.invitation_action(alias, a, None, v, k))
}
fn operation(v: &serde_json::Value) -> String {
    v["operation_id"].as_str().unwrap().into()
}
#[test]
fn mixed_inbox_pages_are_not_mistaken_for_complete_pages() {
    let local = foks_proto::decode_team_inbox(include_bytes!(
        "../../../foks-snowpack/tests/fixtures/foks-v0.1.9/invitations/local.inbox"
    ))
    .unwrap()
    .remove(0);
    let mut remote = local.clone();
    remote.rsvp = foks_proto::TeamRsvp::new([56; 17]).unwrap();
    remote.request = foks_proto::RawInboxRequest::Remote(
        foks_proto::RemoteJoinRequest::decode(include_bytes!(
            "../../../foks-snowpack/tests/fixtures/foks-v0.1.9/invitations/remote.request"
        ))
        .unwrap(),
    );
    for (available, expected_limits, expected_len, truncated) in [
        (40, vec![100], 40, false),
        (150, vec![100, 1000], 150, false),
        (1200, vec![100, 1000], 1000, true),
    ] {
        let mut limits = Vec::new();
        let (rows, incomplete) = read_inbox_pages(|limit| {
            limits.push(limit);
            // Model Go's cap on a merged, alternating local/remote page.
            Ok((0..available.min(limit as usize))
                .map(|n| {
                    if n % 2 == 0 {
                        local.clone()
                    } else {
                        remote.clone()
                    }
                })
                .collect())
        })
        .unwrap();
        assert_eq!(limits, expected_limits);
        assert_eq!(rows.len(), expected_len);
        assert_eq!(incomplete, truncated);
    }
}

#[test]
fn inbox_handle_lookup_covers_go_timestamp_precision() {
    // Model Go's PostgreSQL ctime comparison in microseconds, while the
    // inbox response exports the timestamp truncated to milliseconds.
    let millisecond = 1_700_000_000_123_u64;
    for fraction in [0, 1, 456, 999] {
        let stored_microseconds = millisecond * 1000 + fraction;
        let exported = stored_microseconds / 1000;
        let page = inbox_handle_pagination(exported);
        assert!(stored_microseconds >= page.start * 1000);
        assert!(stored_microseconds <= page.end * 1000);
        assert!((millisecond - 1) * 1000 < page.start * 1000);
        assert!((millisecond + 1) * 1000 + 1 > page.end * 1000);
    }
    // Malformed extreme timestamps must not overflow or turn end into the
    // protocol's zero (unbounded) sentinel.
    assert_eq!(inbox_handle_pagination(u64::MAX).end, u64::MAX);
}

#[test]
fn local_invitations_reopen_scope_handles_and_reuse_durable_admission() {
    let f = Fixture::start();
    f.run(|s, v, k| s.create_account("owner", "inviteowner", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_account("joiner", "invitejoiner", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_named_team("owner", "project", "project", v, k));
    let prepared = action(
        &f,
        "owner",
        InvitationAction::Create {
            team_alias: "project".into(),
        },
    );
    f.run(|s, v, k| {
        assert!(s
            .invitation_action(
                "joiner",
                InvitationAction::Attempt {
                    operation_id: operation(&prepared)
                },
                None,
                v,
                k
            )
            .is_err());
        Ok(())
    });
    let published = action(
        &f,
        "owner",
        InvitationAction::Attempt {
            operation_id: operation(&prepared),
        },
    );
    assert_eq!(published["state"], "complete");
    let invite = published["invite"].as_str().unwrap().to_owned();
    let p = action(
        &f,
        "joiner",
        InvitationAction::Accept {
            invite: invite.clone(),
        },
    );
    let done = action(
        &f,
        "joiner",
        InvitationAction::Attempt {
            operation_id: operation(&p),
        },
    );
    assert_eq!(done["delivery_acknowledged"], true);
    assert_eq!(
        action(
            &f,
            "joiner",
            InvitationAction::Attempt {
                operation_id: operation(&p)
            }
        )["delivery_acknowledged"],
        true
    );
    let inbox = action(
        &f,
        "owner",
        InvitationAction::Inbox {
            team_alias: "project".into(),
        },
    );
    assert_eq!(inbox["rows"][0]["verified"], true);
    let id = inbox["rows"][0]["request_id"].as_str().unwrap().to_owned();
    assert!(!inbox.to_string().contains("permission"));
    // Another reader must not invalidate the handle already on screen.
    let refreshed = action(
        &f,
        "owner",
        InvitationAction::Inbox {
            team_alias: "project".into(),
        },
    );
    assert_eq!(refreshed["rows"][0]["request_id"], id);

    let rejection = action(
        &f,
        "owner",
        InvitationAction::Reject {
            team_alias: "project".into(),
            request_id: id.clone(),
        },
    );
    action(
        &f,
        "owner",
        InvitationAction::Attempt {
            operation_id: operation(&rejection),
        },
    );
    let p = action(&f, "joiner", InvitationAction::Accept { invite });
    action(
        &f,
        "joiner",
        InvitationAction::Attempt {
            operation_id: operation(&p),
        },
    );
    // A newly pending request must not make the old, rejected handle valid,
    // even if it falls within the widened timestamp window.
    f.run(|s, v, k| {
        let host = s.pinned_host()?;
        let account = v.account("owner")?;
        let team = EntityId::from_bytes(v.team("project")?.team_id.clone())?;
        let key = inbox_key(&host, &account.credential.uid, &team);
        let mut handles: Vec<InboxHandle> = serde_json::from_slice(&v.store.get(&key)?)?;
        let mut old = foks_proto::decode_team_inbox(&handles[0].row)?;
        let pending = s.client.team_invitation_inbox(
            &host,
            FederationCredential::Software(&account.credential),
            &team,
            None,
        )?;
        assert_eq!(pending.len(), 1);
        old[0].time = pending[0].time.saturating_sub(1);
        handles[0].row = foks_proto::encode_team_inbox(&old)?;
        v.store
            .put(&key, &Zeroizing::new(serde_json::to_vec(&handles)?))?;
        assert!(matches!(
            s.invitation_action(
                "owner",
                InvitationAction::Approve {
                    team_alias: "project".into(),
                    request_id: id.clone(),
                    role: TeamMemberRole::Member { visibility: 0 },
                },
                None,
                v,
                k,
            ),
            Err(Error::InvalidAccount(
                "request is no longer in the pending inbox; refresh it"
            ))
        ));
        // The same inclusive window must find the intended RSVP at its
        // upper boundary. This exercises the actual handle lookup path.
        old[0] = pending[0].clone();
        old[0].time = pending[0].time - 1;
        handles[0].row = foks_proto::encode_team_inbox(&old)?;
        v.store
            .put(&key, &Zeroizing::new(serde_json::to_vec(&handles)?))?;
        let found = s.invitation_inbox_handle(
            &host,
            FederationCredential::Software(&account.credential),
            &team,
            None,
            &id,
            v,
        )?;
        assert_eq!(found.rsvp, pending[0].rsvp);
        Ok(())
    });
    let inbox = action(
        &f,
        "owner",
        InvitationAction::Inbox {
            team_alias: "project".into(),
        },
    );
    let id = inbox["rows"][0]["request_id"].as_str().unwrap().to_owned();
    action(
        &f,
        "owner",
        InvitationAction::Approve {
            team_alias: "project".into(),
            request_id: id,
            role: TeamMemberRole::Member { visibility: 0 },
        },
    );
    assert!(action(
        &f,
        "owner",
        InvitationAction::Inbox {
            team_alias: "project".into()
        }
    )["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    f.run(|s, v, _| {
        assert_eq!(s.list_team_members("project", v)?.len(), 2);
        Ok(())
    });
}
#[test]
fn local_acceptance_lost_reply_stays_unknown_after_restart_without_replay() {
    let f = Fixture::start();
    f.run(|s, v, k| s.create_account("owner", "lostinviteowner", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_account("joiner", "lostinvitejoiner", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_named_team("owner", "project", "project", v, k));
    let p = action(
        &f,
        "owner",
        InvitationAction::Create {
            team_alias: "project".into(),
        },
    );
    let p = action(
        &f,
        "owner",
        InvitationAction::Attempt {
            operation_id: operation(&p),
        },
    );
    let p = action(
        &f,
        "joiner",
        InvitationAction::Accept {
            invite: p["invite"].as_str().unwrap().into(),
        },
    );
    f._environment
        .arm_fault(foks_server_testkit::TestFault::InvitationAfterCommitBeforeResponse);
    f.run(|s, v, k| {
        assert!(s
            .invitation_action(
                "joiner",
                InvitationAction::Attempt {
                    operation_id: operation(&p)
                },
                None,
                v,
                k
            )
            .is_err());
        Ok(())
    });
    for _ in 0..2 {
        assert_eq!(
            action(
                &f,
                "joiner",
                InvitationAction::Attempt {
                    operation_id: operation(&p)
                }
            )["state"],
            "submission-unknown"
        );
    }
    let inbox = action(
        &f,
        "owner",
        InvitationAction::Inbox {
            team_alias: "project".into(),
        },
    );
    assert_eq!(inbox["rows"].as_array().unwrap().len(), 1);
    f.run(|s, v, _| {
        let a = v.account("joiner")?;
        let ops = HardStateStore::open(&s.paths.hard_database)?.invitation_operations(
            s.pinned_host()?.host_id().as_bytes(),
            a.credential.uid.as_bytes(),
        )?;
        assert_eq!(ops[0].attempt_count, 1);
        Ok(())
    });
}

#[test]
fn remote_invitation_reopens_both_profiles_and_admits_with_verified_keys() {
    let mut f = Fixture::start();
    let remote_env = TestEnvironment::new().unwrap();
    let _server = remote_env.start_server().unwrap();
    let root = remote_env.client_path("remote-invite", "root.der").unwrap();
    remote_env.write_probe_root(&root).unwrap();
    f.registry
        .add(Profile {
            name: "remote".into(),
            label: None,
            probe: format!("localhost:{}", remote_env.addresses().unwrap().probe.port()),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let run = |profile: &str, alias: &str, action: Option<InvitationAction>| {
        let s = ProfileSession::open(&f.registry, profile).unwrap();
        let other_name = if profile == "local" {
            "remote"
        } else {
            "local"
        };
        let other = ProfileSession::open(&f.registry, other_name).unwrap();
        f.credentials
            .with_checked_sessions(&s, &other, |s, other| {
                let master = f.credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &s.paths.credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                if let Some(action) = action {
                    s.remote_invitation_action(other, alias, action, None, &mut vault, &master)
                } else {
                    s.probe_and_pin()?;
                    s.create_account(
                        alias,
                        "remoteinvitejoiner",
                        "laptop",
                        "",
                        "",
                        None,
                        &mut vault,
                        &master,
                    )?;
                    Ok(serde_json::Value::Null)
                }
            })
            .unwrap()
    };
    run("remote", "joiner", None);
    f.run(|s, v, k| s.create_account("owner", "remoteinviteowner", "laptop", "", "", None, v, k));
    let team = f.run(|s, v, k| s.create_named_team("owner", "project", "project", v, k));
    let p = action(
        &f,
        "owner",
        InvitationAction::Create {
            team_alias: "project".into(),
        },
    );
    let published = action(
        &f,
        "owner",
        InvitationAction::Attempt {
            operation_id: operation(&p),
        },
    );
    let p = run(
        "remote",
        "joiner",
        Some(InvitationAction::AcceptRemote {
            remote_profile: "local".into(),
            invite: published["invite"].as_str().unwrap().into(),
        }),
    );
    let done = run(
        "remote",
        "joiner",
        Some(InvitationAction::AttemptRemote {
            remote_profile: "local".into(),
            operation_id: operation(&p),
        }),
    );
    assert_eq!(done["delivery_acknowledged"], true);
    let inbox = action(
        &f,
        "owner",
        InvitationAction::Inbox {
            team_alias: "project".into(),
        },
    );
    assert_eq!(inbox["rows"].as_array().unwrap().len(), 1);
    let request_id = inbox["rows"][0]["request_id"].as_str().unwrap().to_owned();
    let inspection = run(
        "local",
        "owner",
        Some(InvitationAction::InspectRemote {
            remote_profile: "remote".into(),
            team_alias: "project".into(),
            request_id: request_id.clone(),
        }),
    );
    assert_eq!(inspection["verified"], true);
    for _ in 0..2 {
        let done = run(
            "local",
            "owner",
            Some(InvitationAction::ApproveRemote {
                remote_profile: "remote".into(),
                team_alias: "project".into(),
                request_id: request_id.clone(),
                role: TeamMemberRole::Member { visibility: 0 },
            }),
        );
        assert_eq!(done["state"], "complete");
    }
    let sync = run(
        "remote",
        "joiner",
        Some(InvitationAction::SyncRemote {
            remote_profile: "local".into(),
            team_id: team.team_id_hex,
            source_team_alias: None,
            source_role: None,
        }),
    );
    assert_eq!(sync["membership_verified"], true);
    assert!(sync["key_generations"].as_u64().unwrap() > 0);
    assert!(!inspection.to_string().contains("permission"));
}

#[test]
fn inbox_count_reports_the_inbox_size_without_expanding_or_storing_rows() {
    let f = Fixture::start();
    f.run(|s, v, k| s.create_account("owner", "countowner", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_account("joiner", "countjoiner", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_named_team("owner", "project", "project", v, k));
    let prepared = action(
        &f,
        "owner",
        InvitationAction::Create {
            team_alias: "project".into(),
        },
    );
    let published = action(
        &f,
        "owner",
        InvitationAction::Attempt {
            operation_id: operation(&prepared),
        },
    );
    let invite = published["invite"].as_str().unwrap().to_owned();
    let accepted = action(&f, "joiner", InvitationAction::Accept { invite });
    action(
        &f,
        "joiner",
        InvitationAction::Attempt {
            operation_id: operation(&accepted),
        },
    );
    let key = f.run(|s, v, _| {
        let host = s.pinned_host()?;
        let account = v.account("owner")?;
        let team = EntityId::from_bytes(v.team("project")?.team_id.clone())?;
        Ok(inbox_key(&host, &account.credential.uid, &team))
    });
    // Counting expands no row, so it writes no handle blob and hands back no
    // rows at all: only how many are waiting.
    let counted = action(
        &f,
        "owner",
        InvitationAction::InboxCount {
            team_alias: "project".into(),
        },
    );
    assert_eq!(counted["count"], 1);
    assert_eq!(counted["possibly_truncated"], false);
    assert!(counted.get("rows").is_none());
    f.run(|_, v, _| {
        assert!(v.store.get(&key).is_err());
        Ok(())
    });
    // The full inbox reports the same number, still verifies each row, and
    // writes the handles the decisions need.
    let inbox = action(
        &f,
        "owner",
        InvitationAction::Inbox {
            team_alias: "project".into(),
        },
    );
    assert_eq!(inbox["rows"].as_array().unwrap().len(), 1);
    assert_eq!(inbox["rows"][0]["verified"], true);
    assert_eq!(inbox["rows"][0]["joiner_kind"], "user");
    assert_eq!(inbox["rows"][0]["username"], "countjoiner");
    assert_eq!(inbox["possibly_truncated"], counted["possibly_truncated"]);
    f.run(|_, v, _| {
        assert!(v.store.get(&key).is_ok());
        Ok(())
    });
    // The Rust server can return 1000 rows of each kind. Every handle the
    // reader can store must also pass portability inventory validation.
    f.run(|s, v, _| {
        let original = v.store.get(&key)?;
        let stored: Vec<InboxHandle> = serde_json::from_slice(&original)?;
        let mut handles: Vec<InboxHandle> = (0..2000)
            .map(|n| InboxHandle {
                id: format!("{n:032x}"),
                row: stored[0].row.clone(),
            })
            .collect();
        let hard = HardStateStore::open(&s.paths.hard_database)?;
        let suffix = key.strip_prefix("invitation-inbox.").unwrap();
        v.store
            .put(&key, &Zeroizing::new(serde_json::to_vec(&handles)?))?;
        assert!(validate_inventory_record(
            v,
            "invitation-inbox",
            suffix,
            &hard
        )?);
        handles.push(InboxHandle {
            id: format!("{:032x}", 2000),
            row: stored[0].row.clone(),
        });
        v.store
            .put(&key, &Zeroizing::new(serde_json::to_vec(&handles)?))?;
        assert!(matches!(
            validate_inventory_record(v, "invitation-inbox", suffix, &hard),
            Err(Error::InvalidAccount(
                "invitation inbox inventory exceeds limit"
            ))
        ));
        v.store.put(&key, &original)?;
        Ok(())
    });
    // An alias this account does not own is refused before any request.
    f.run(|s, v, k| {
        assert!(matches!(
            s.invitation_action(
                "joiner",
                InvitationAction::InboxCount {
                    team_alias: "project".into()
                },
                None,
                v,
                k
            ),
            Err(Error::InvalidAccount("team belongs to another account"))
        ));
        Ok(())
    });
}

#[test]
fn a_preloaded_destination_team_must_be_the_team_the_row_names() {
    let f = Fixture::start();
    f.run(|s, v, k| s.create_account("owner", "preloadowner", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_account("joiner", "preloadjoiner", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_named_team("owner", "project", "project", v, k));
    f.run(|s, v, k| s.create_named_team("owner", "other", "other", v, k));
    let prepared = action(
        &f,
        "owner",
        InvitationAction::Create {
            team_alias: "project".into(),
        },
    );
    let published = action(
        &f,
        "owner",
        InvitationAction::Attempt {
            operation_id: operation(&prepared),
        },
    );
    let invite = published["invite"].as_str().unwrap().to_owned();
    let accepted = action(&f, "joiner", InvitationAction::Accept { invite });
    action(
        &f,
        "joiner",
        InvitationAction::Attempt {
            operation_id: operation(&accepted),
        },
    );
    f.run(|s, v, _| {
        let host = s.pinned_host()?;
        let account = v.account("owner")?;
        let credential = FederationCredential::Software(&account.credential);
        let project = EntityId::from_bytes(v.team("project")?.team_id.clone())?;
        let other = EntityId::from_bytes(v.team("other")?.team_id.clone())?;
        let rows = s
            .client
            .team_invitation_inbox(&host, credential, &project, None)?;
        assert_eq!(rows.len(), 1);
        let user = s
            .client
            .authenticate_credential_and_pin(&host, credential)?;
        let loaded = |team| {
            s.client.load_and_pin_team_with_credential(
                &host,
                credential,
                &user.verified,
                &user.puks,
                team,
            )
        };
        // The destination the caller loaded once is the one every row is
        // resolved against, so a mismatch is refused rather than resolved
        // against whatever the caller happened to pass.
        assert!(matches!(
            s.client.load_local_invitation_joiner_with_team(
                &host,
                credential,
                &project,
                &loaded(&other)?,
                &rows[0],
            ),
            Err(foks_client::Error::TeamBinding(
                "invitation destination team does not match the loaded team"
            ))
        ));
        // Loaded correctly, the loader supplied with the team resolves the row exactly as
        // the per-row loader does.
        let loaded_with_team = s.client.load_local_invitation_joiner_with_team(
            &host,
            credential,
            &project,
            &loaded(&project)?,
            &rows[0],
        )?;
        let per_row = s
            .client
            .load_local_invitation_joiner(&host, credential, &project, &rows[0])?;
        assert_eq!(loaded_with_team.uid(), per_row.uid());
        assert_eq!(loaded_with_team.username_utf8(), per_row.username_utf8());
        Ok(())
    });
}
