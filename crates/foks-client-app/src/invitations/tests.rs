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
    let rejection = action(
        &f,
        "owner",
        InvitationAction::Reject {
            team_alias: "project".into(),
            request_id: id,
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
