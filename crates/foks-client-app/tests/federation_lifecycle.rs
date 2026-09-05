use std::time::{SystemTime, UNIX_EPOCH};

use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend,
    FederationDestinationRole, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy,
    TeamMemberRole, TrustRoot, YubiProvisionInput,
};
use foks_client_db::{HardStateStore, ScheduledJob, ScheduledJobKind};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::{TestEnvironment, TestFault};
use foks_yubi::{MockYubiProvider, Pin, SlotId, YubiProvider as _};

#[test]
fn protected_product_workflow_admits_and_reconciles_a_remote_team() {
    let local_environment = TestEnvironment::new().unwrap();
    let remote_environment = TestEnvironment::new().unwrap();
    let _local_server = local_environment.start_server().unwrap();
    let _remote_server = remote_environment.start_server().unwrap();

    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state");
    let local_root = temporary.path().join("local-probe-root.der");
    let remote_root = temporary.path().join("remote-probe-root.der");
    local_environment.write_probe_root(&local_root).unwrap();
    remote_environment.write_probe_root(&remote_root).unwrap();
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();

    let mut registry = ProfileRegistry::open(&state).unwrap();
    add_profile(
        &mut registry,
        "local",
        local_environment.addresses().unwrap().probe.port(),
        local_root,
    );
    add_profile(
        &mut registry,
        "remote",
        remote_environment.addresses().unwrap().probe.port(),
        remote_root,
    );
    let local = ProfileSession::open(&registry, "local").unwrap();
    let remote = ProfileSession::open(&registry, "remote").unwrap();
    let credentials = ClientCredentials::open(&state).unwrap();

    credentials
        .with_checked_sessions(&local, &remote, |local, remote| {
            local.probe_and_pin()?;
            remote.probe_and_pin()?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    let master = credentials.master_key().unwrap();
    credentials
        .with_checked_session(&local, |local| {
            let mut store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            local.create_account(
                "local-owner",
                "localfedowner",
                "local federation owner",
                "local@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            local.create_named_team(
                "local-owner",
                "local-team",
                "localteam",
                &mut vault,
                &master,
            )?;
            local.create_account(
                "local-admin",
                "localfedadmin",
                "local federation admin",
                "local-admin@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            local.add_local_team_member(
                "local-team",
                "localfedadmin",
                TeamMemberRole::Admin,
                &mut vault,
                &master,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    credentials
        .with_checked_session(&remote, |remote| {
            let mut store = EncryptedFileSecretStore::open(
                &remote.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            remote.create_account(
                "remote-owner",
                "remotefedowner",
                "remote federation owner",
                "remote@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            remote.create_named_team(
                "remote-owner",
                "remote-team",
                "remoteteam",
                &mut vault,
                &master,
            )?;
            remote.create_account(
                "remote-member",
                "remotefedmember",
                "remote federation member",
                "remote-member@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            remote.add_local_team_member(
                "remote-team",
                "remotefedmember",
                TeamMemberRole::Admin,
                &mut vault,
                &master,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    let remote_admin = credentials
        .with_checked_sessions(&local, &remote, |local, remote| {
            let mut local_store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut remote_store = EncryptedFileSecretStore::open(
                &remote.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut local_vault = AccountVault::new(&mut local_store);
            let mut remote_vault = AccountVault::new(&mut remote_store);
            local.admit_federated_team(
                remote,
                "local-team",
                "remote-team",
                FederationDestinationRole::Admin,
                &mut local_vault,
                &mut remote_vault,
                &master,
            )
        })
        .unwrap_err();
    assert!(matches!(
        remote_admin,
        foks_client_app::Error::InvalidAccount(
            "federated parties cannot be team administrators or owners"
        )
    ));

    let remote_fault_hits =
        remote_environment.arm_fault(TestFault::FederationGrantTeamAfterCommitBeforeResponse);
    let ambiguous_grant = credentials
        .with_checked_sessions(&local, &remote, |local, remote| {
            let mut local_store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut remote_store = EncryptedFileSecretStore::open(
                &remote.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut local_vault = AccountVault::new(&mut local_store);
            let mut remote_vault = AccountVault::new(&mut remote_store);
            local.admit_federated_team(
                remote,
                "local-team",
                "remote-team",
                FederationDestinationRole::Member { visibility: 0 },
                &mut local_vault,
                &mut remote_vault,
                &master,
            )
        })
        .unwrap_err();
    assert!(matches!(ambiguous_grant, foks_client_app::Error::Client(_)));
    assert_eq!(remote_environment.fault_hits(), remote_fault_hits + 1);

    credentials
        .with_checked_session(&local, |local| {
            let mut store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            let memberships = local.list_federated_memberships("local-team", &mut vault)?;
            assert_eq!(memberships.len(), 1);
            assert!(!memberships[0].active);
            assert!(memberships[0].operation_id_hex.is_none());
            assert!(local
                .list_team_members("local-team", &mut vault)?
                .iter()
                .all(|member| member.scoped_host_id_hex.is_none()));
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    // The failed permission response happened after both signed metadata
    // transitions. Their authenticated hard-state projections persist, while
    // no scoped roster row was created before the ranges became disjoint.
    let local_database = rusqlite::Connection::open(&local.paths().hard_database).unwrap();
    let (local_seqno, local_low, local_high_infinity): (i64, Vec<u8>, bool) = local_database
        .query_row(
            "SELECT chain_seqno, index_low_base, index_high_infinity FROM teams",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(local_seqno, 3);
    assert_eq!(local_low, [0x80]);
    assert!(local_high_infinity);
    let remote_database = rusqlite::Connection::open(&remote.paths().hard_database).unwrap();
    let (remote_seqno, remote_low, remote_high): (i64, Vec<u8>, Vec<u8>) = remote_database
        .query_row(
            "SELECT chain_seqno, index_low_base, index_high_base FROM teams",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(remote_seqno, 3);
    assert_eq!(remote_low, [1]);
    assert_eq!(remote_high, [0x20]);

    let local_fault_hits =
        local_environment.arm_fault(TestFault::FederationTeamEditAfterCommitBeforeResponse);
    let first = credentials
        .with_checked_sessions(&local, &remote, |local, remote| {
            let mut local_store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut remote_store = EncryptedFileSecretStore::open(
                &remote.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut local_vault = AccountVault::new(&mut local_store);
            let mut remote_vault = AccountVault::new(&mut remote_store);
            let report = local.admit_federated_team(
                remote,
                "local-team",
                "remote-team",
                FederationDestinationRole::Member { visibility: 0 },
                &mut local_vault,
                &mut remote_vault,
                &master,
            )?;
            let memberships = local.list_federated_memberships("local-team", &mut local_vault)?;
            assert_eq!(memberships.len(), 1);
            assert!(memberships[0].active);
            assert_eq!(
                memberships[0].operation_id_hex.as_deref(),
                Some(report.operation_id_hex.as_str())
            );
            Ok::<_, foks_client_app::Error>(report)
        })
        .unwrap();
    assert_eq!(local_environment.fault_hits(), local_fault_hits + 1);
    assert!(first.active);

    let repeated = credentials
        .with_checked_sessions(&remote, &local, |remote, local| {
            let mut local_store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut remote_store = EncryptedFileSecretStore::open(
                &remote.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut local_vault = AccountVault::new(&mut local_store);
            let mut remote_vault = AccountVault::new(&mut remote_store);
            local.admit_federated_team(
                remote,
                "local-team",
                "remote-team",
                FederationDestinationRole::Member { visibility: 0 },
                &mut local_vault,
                &mut remote_vault,
                &master,
            )
        })
        .unwrap();
    assert_eq!(repeated.operation_id_hex, first.operation_id_hex);
    assert_eq!(repeated.scheduled_job_id_hex, first.scheduled_job_id_hex);

    // A local user remains actionable by its authenticated party ID even
    // though the same roster now includes a host-scoped admitted team. The
    // demotion must use the authenticated mixed-roster path; the scoped party
    // remains present and authenticated even though its Member role does not
    // receive the rotated Admin key.
    credentials
        .with_checked_session(&local, |local| {
            let mut store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            let members = local.list_team_members("local-team", &mut vault)?;
            let party_id = members
                .iter()
                .find(|member| member.username.as_deref() == Some("localfedadmin"))
                .map(|member| member.party_id_hex.clone())
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "local administrator is missing from the authenticated mixed roster",
                ))?;
            let scoped_party_id = members
                .iter()
                .find(|member| member.scoped_host_id_hex.is_some())
                .map(|member| member.party_id_hex.clone())
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "admitted team is missing from the authenticated mixed roster",
                ))?;
            assert!(matches!(
                local
                    .remove_local_team_member("local-team", &scoped_party_id, &mut vault, &master,),
                Err(foks_client_app::Error::Protocol(_))
            ));
            let demoted = local.demote_local_team_member_in_authenticated_roster(
                "local-team",
                &party_id,
                TeamMemberRole::Member { visibility: 0 },
                &mut vault,
                &registry,
                &credentials,
                &master,
            )?;
            assert_eq!(
                demoted.destination_role,
                Some(TeamMemberRole::Member { visibility: 0 })
            );
            assert!(local
                .list_team_members("local-team", &mut vault)?
                .iter()
                .any(|member| member.scoped_host_id_hex.is_some()));
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    let original_remote_generation = credentials
        .with_checked_session(&local, |local| {
            let mut store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            let members = local.list_team_members("local-team", &mut vault)?;
            members
                .into_iter()
                .find(|member| member.scoped_host_id_hex.is_some())
                .map(|member| member.generation)
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "federated roster row is missing",
                ))
        })
        .unwrap();
    credentials
        .with_checked_session(&remote, |remote| {
            let mut store = EncryptedFileSecretStore::open(
                &remote.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            let party_id = remote
                .list_team_members("remote-team", &mut vault)?
                .into_iter()
                .find(|member| member.username.as_deref() == Some("remotefedmember"))
                .map(|member| member.party_id_hex)
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "remote member is missing from the authenticated roster",
                ))?;
            remote.demote_local_team_member(
                "remote-team",
                &party_id,
                TeamMemberRole::Member { visibility: 0 },
                &mut vault,
                &master,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    let due = micros_now().saturating_add(10 * 60 * 1_000_000);
    let forged_job_id = [0xaa; 16];
    credentials
        .with_checked_session(&local, |local| {
            let host = local.pinned_host()?;
            HardStateStore::open(&local.paths().hard_database)?.register_scheduled_job(
                &ScheduledJob {
                    job_id: forged_job_id,
                    kind: ScheduledJobKind::FederationRefresh,
                    host_id: host.host_id().as_bytes().to_vec(),
                    scope_id: br#"{"local_team_alias":"local-team","remote_profile":"remote","remote_team_alias":"remote-team","destination":{"member":{"visibility":0}}}"#.to_vec(),
                    interval_micros: 24 * 60 * 60 * 1_000_000,
                    next_run_at: due,
                    failure_count: 0,
                    lease_until: None,
                    last_completed_at: None,
                    last_error: None,
                    updated_at: due,
                },
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    let scheduled = credentials
        .with_checked_session(&local, |local| {
            let mut store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            local.run_due_jobs_with_federation(due, &mut vault, &registry, &credentials, &master)
        })
        .unwrap();
    assert_eq!(scheduled.runs.len(), 2);
    let real = scheduled
        .runs
        .iter()
        .find(|run| run.job_id_hex == first.scheduled_job_id_hex)
        .unwrap();
    assert!(
        real.completed,
        "federation refresh failed: {:?}",
        real.error
    );
    credentials
        .with_checked_session(&local, |local| {
            let mut store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            let members = local.list_team_members("local-team", &mut vault)?;
            let refreshed = members
                .into_iter()
                .find(|member| member.scoped_host_id_hex.is_some())
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "refreshed federated roster row is missing",
                ))?;
            assert!(refreshed.generation > original_remote_generation);
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    let forged = scheduled
        .runs
        .iter()
        .find(|run| run.job_id_hex == "aa".repeat(16))
        .unwrap();
    assert!(!forged.completed);
    assert!(forged
        .error
        .as_deref()
        .unwrap()
        .contains("untrusted public scope"));

    // Expulsion uses the admission-time removal key retained only in the
    // encrypted vault. The real Rust server validates the removal proof and
    // rotated PTKs; no fake range or roster override participates.
    _remote_server.shutdown().unwrap();
    // Neither revocation nor recovery may depend on the expelled host. Fail
    // the local binding cleanup after the chain commits, leaving the intent
    // and stale active binding exactly as a crash would.
    let interrupted = credentials
        .with_checked_session(&local, |local| {
            let store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut store = FailExpulsionCleanup(store);
            let mut vault = AccountVault::new(&mut store);
            let binding = local
                .list_federated_memberships("local-team", &mut vault)?
                .into_iter()
                .next()
                .unwrap();
            local.expel_federated_team(
                "local-team",
                &binding.remote_host_id_hex,
                &binding.remote_team_id_hex,
                &mut vault,
                &registry,
                &credentials,
                &master,
            )
        })
        .unwrap_err();
    assert!(matches!(interrupted, foks_client_app::Error::Keystore(
        foks_keystore::Error::Io(ref error)
    ) if error.to_string() == "injected expulsion cleanup failure"));
    let expelled = credentials
        .with_checked_session(&local, |local| {
            let mut store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            let binding = local
                .list_federated_memberships("local-team", &mut vault)?
                .into_iter()
                .next()
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "federated membership disappeared before retained-key expulsion",
                ))?;
            local.expel_federated_team(
                "local-team",
                &binding.remote_host_id_hex,
                &binding.remote_team_id_hex,
                &mut vault,
                &registry,
                &credentials,
                &master,
            )
        })
        .unwrap();
    assert!(!expelled.active);
    let scheduled_job_id: [u8; 16] = decode_hex(&first.scheduled_job_id_hex)
        .unwrap()
        .try_into()
        .unwrap();
    assert!(HardStateStore::open(&local.paths().hard_database)
        .unwrap()
        .scheduled_job(&scheduled_job_id)
        .unwrap()
        .is_none());
    // Local expulsion has no post-commit dependency on the remote profile and
    // does not reinterpret the protocol's grant RPC as an explicit revocation.
    // The remote grant therefore keeps its normal state; exact viewer removal
    // is invalidated atomically only when it occurs in the granting team edit.
    let remote_database = rusqlite::Connection::open(remote_environment.database_path()).unwrap();
    let permission_state: i64 = remote_database
        .query_row(
            "SELECT state FROM federation_team_view_permissions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(permission_state, 1);
    credentials
        .with_checked_session(&local, |local| {
            let mut store = EncryptedFileSecretStore::open(
                &local.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            assert!(local
                .list_federated_memberships("local-team", &mut vault)?
                .is_empty());
            assert!(local
                .list_team_members("local-team", &mut vault)?
                .iter()
                .all(|member| member.scoped_host_id_hex.is_none()));
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
}

/// A federation graph can be deeper than one hop: A federates a team to B,
/// which federates its own team to C. When C's roster advances, B's exported
/// projection goes stale, and A cannot rekey against it until B has run its
/// own responder. The refresh therefore has to reach through B into C in a
/// single pass rather than reporting a stale recipient and giving up.
#[test]
fn a_federated_refresh_cascades_through_an_intermediate_profile() {
    let environments = [
        TestEnvironment::new().unwrap(),
        TestEnvironment::new().unwrap(),
        TestEnvironment::new().unwrap(),
    ];
    let _servers = environments
        .iter()
        .map(|environment| environment.start_server().unwrap())
        .collect::<Vec<_>>();
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state");
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    for (index, profile) in ["root", "remote-b", "remote-c"].iter().enumerate() {
        let root = temporary.path().join(format!("{profile}-root.der"));
        environments[index].write_probe_root(&root).unwrap();
        add_profile(
            &mut registry,
            profile,
            environments[index].addresses().unwrap().probe.port(),
            root,
        );
    }
    let root = ProfileSession::open(&registry, "root").unwrap();
    let remote_b = ProfileSession::open(&registry, "remote-b").unwrap();
    let remote_c = ProfileSession::open(&registry, "remote-c").unwrap();
    let credentials = ClientCredentials::open(&state).unwrap();
    for session in [&root, &remote_b, &remote_c] {
        credentials
            .with_checked_session(session, |session| session.probe_and_pin())
            .unwrap();
    }
    let master = credentials.master_key().unwrap();
    let pin = Pin::new("123456").unwrap();
    let provider = MockYubiProvider::with_card("cascade-root-card", 75001, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);

    // The root administers through hardware, so the cascade also has to carry
    // an unlocked credential across the nested profiles.
    credentials
        .with_checked_session(&root, |root| {
            let mut store = EncryptedFileSecretStore::open(
                &root.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            root.create_account(
                "owner",
                "cascaderoot",
                "cascade root",
                "root@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            root.create_named_team("owner", "team", "cascaderootteam", &mut vault, &master)?;
            root.provision_yubi_device(
                YubiProvisionInput {
                    source_alias: "owner".to_owned(),
                    target_alias: "hardware".to_owned(),
                    device_name: "cascade root Yubi".to_owned(),
                    serial: 2,
                    card,
                    signing_slot: SlotId::new(0x82)?,
                    pq_slot: SlotId::new(0x83)?,
                    retry_configuration: None,
                },
                Pin::new("123456")?,
                &provider,
                &mut vault,
                &master,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    for (session, username, team_name, member_name) in [
        (&remote_b, "cascadeb", "cascadebteam", "cascadebmember"),
        (&remote_c, "cascadec", "cascadecteam", "cascadecmember"),
    ] {
        credentials
            .with_checked_session(session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.create_account(
                    "owner",
                    username,
                    username,
                    &format!("{username}@example.test"),
                    "",
                    None,
                    &mut vault,
                    &master,
                )?;
                session.create_named_team("owner", "team", team_name, &mut vault, &master)?;
                session.create_account(
                    "member",
                    member_name,
                    member_name,
                    &format!("{member_name}@example.test"),
                    "",
                    None,
                    &mut vault,
                    &master,
                )?;
                session.add_local_team_member(
                    "team",
                    member_name,
                    TeamMemberRole::Admin,
                    &mut vault,
                    &master,
                )?;
                Ok::<_, foks_client_app::Error>(())
            })
            .unwrap();
    }
    for (left, right) in [(&root, &remote_b), (&remote_b, &remote_c)] {
        credentials
            .with_checked_sessions(left, right, |left, right| {
                let mut left_store = EncryptedFileSecretStore::open(
                    &left.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut right_store = EncryptedFileSecretStore::open(
                    &right.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                left.admit_federated_team(
                    right,
                    "team",
                    "team",
                    FederationDestinationRole::Member { visibility: 0 },
                    &mut AccountVault::new(&mut left_store),
                    &mut AccountVault::new(&mut right_store),
                    &master,
                )?;
                Ok::<_, foks_client_app::Error>(())
            })
            .unwrap();
    }

    let federated_generation = |session: &ProfileSession, missing| {
        credentials
            .with_checked_session(session, |checked| {
                let mut store = EncryptedFileSecretStore::open(
                    &checked.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                checked
                    .list_team_members("team", &mut AccountVault::new(&mut store))?
                    .into_iter()
                    .find(|member| member.scoped_host_id_hex.is_some())
                    .map(|member| member.generation)
                    .ok_or(foks_client_app::Error::InvalidAccount(missing))
            })
            .unwrap()
    };
    let root_before = federated_generation(&root, "root-to-B roster row is missing");
    let b_before = federated_generation(&remote_b, "B-to-C roster row is missing");
    let (b_host_id, b_team_id) = credentials
        .with_checked_session(&remote_b, |remote_b| {
            let mut store = EncryptedFileSecretStore::open(
                &remote_b.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let team = remote_b
                .list_teams(&mut AccountVault::new(&mut store))?
                .into_iter()
                .find(|team| team.alias == "team")
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "B team summary is missing",
                ))?;
            Ok::<_, foks_client_app::Error>((
                remote_b.pinned_host()?.host_id().as_bytes().to_vec(),
                decode_hex(&team.team_id_hex)?,
            ))
        })
        .unwrap();
    let b_sequence_before = credentials
        .with_checked_session(&remote_b, |remote_b| {
            HardStateStore::open(&remote_b.paths().hard_database)?
                .team_for_host(&b_host_id, &b_team_id)?
                .map(|team| team.chain_seqno)
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "B has no local team projection",
                ))
        })
        .unwrap();

    // Revoke at the far end of the chain. Only C can rekey its own team, and
    // only B can then re-export a projection A is able to use.
    credentials
        .with_checked_session(&remote_c, |remote_c| {
            let mut store = EncryptedFileSecretStore::open(
                &remote_c.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            let party_id = remote_c
                .list_team_members("team", &mut vault)?
                .into_iter()
                .find(|member| member.username.as_deref() == Some("cascadecmember"))
                .map(|member| member.party_id_hex)
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "cascade member is missing from the authenticated roster",
                ))?;
            remote_c.remove_local_team_member("team", &party_id, &mut vault, &master)?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    let report = credentials
        .with_checked_session(&root, |root| {
            let mut store = EncryptedFileSecretStore::open(
                &root.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            root.with_unlocked_yubi(
                "hardware",
                &Pin::new("123456")?,
                &provider,
                &mut vault,
                |actor, vault| {
                    root.refresh_federated_security_with_unlocked_yubi(
                        "team",
                        &[actor],
                        vault,
                        &registry,
                        &credentials,
                        &master,
                    )
                },
            )
        })
        .unwrap();
    assert!(
        report.refreshed && report.deferred.is_none(),
        "the cascade must complete rather than defer: {report:?}"
    );

    let b_after = federated_generation(&remote_b, "refreshed B-to-C roster row is missing");
    let root_after = federated_generation(&root, "refreshed root-to-B roster row is missing");
    let b_sequence_after = credentials
        .with_checked_session(&remote_b, |remote_b| {
            HardStateStore::open(&remote_b.paths().hard_database)?
                .team_for_host(&b_host_id, &b_team_id)?
                .map(|team| team.chain_seqno)
                .ok_or(foks_client_app::Error::InvalidAccount(
                    "B lost its local team projection",
                ))
        })
        .unwrap();
    assert!(
        b_after > b_before,
        "B must rotate its C recipient after C advances"
    );
    assert!(
        b_sequence_after > b_sequence_before,
        "B's nested C refresh must commit a new B chain link"
    );
    // B reboxes its existing PTK to C's new recipient key, which does not
    // rotate B's own source PTK, so the root authenticates the refreshed B
    // chain without an unnecessary local mutation of its own.
    assert!(root_after >= root_before);
}

fn decode_hex(value: &str) -> foks_client_app::Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(foks_client_app::Error::InvalidAccount("invalid test hex"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char)
                .to_digit(16)
                .ok_or(foks_client_app::Error::InvalidAccount("invalid test hex"))?;
            let low = (pair[1] as char)
                .to_digit(16)
                .ok_or(foks_client_app::Error::InvalidAccount("invalid test hex"))?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

struct FailExpulsionCleanup(EncryptedFileSecretStore);

impl foks_keystore::SecretStore for FailExpulsionCleanup {
    fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()> {
        if key == "team.local-team"
            && serde_json::from_slice::<serde_json::Value>(value).unwrap()["federated_members"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            return Err(std::io::Error::other("injected expulsion cleanup failure").into());
        }
        self.0.put(key, value)
    }

    fn get(&mut self, key: &str) -> foks_keystore::Result<zeroize::Zeroizing<Vec<u8>>> {
        self.0.get(key)
    }

    fn remove(&mut self, key: &str) -> foks_keystore::Result<bool> {
        self.0.remove(key)
    }

    fn keys(&mut self) -> foks_keystore::Result<Vec<String>> {
        self.0.keys()
    }
}

fn add_profile(
    registry: &mut ProfileRegistry,
    name: &str,
    probe_port: u16,
    root: std::path::PathBuf,
) {
    registry
        .add(Profile {
            name: name.to_owned(),
            probe: format!("localhost:{probe_port}"),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
}

fn micros_now() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros(),
    )
    .unwrap()
}
