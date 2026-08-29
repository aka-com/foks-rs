use std::time::{SystemTime, UNIX_EPOCH};

use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend,
    FederationDestinationRole, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_client_db::{HardStateStore, ScheduledJob, ScheduledJobKind};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::{TestEnvironment, TestFault};

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
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

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
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

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

    let due = micros_now().saturating_add(10 * 60 * 1_000_000);
    let forged_job_id = [0xaa; 16];
    credentials
        .with_checked_session(&local, |local| {
            let host = local.pinned_host()?;
            HardStateStore::open(&local.paths().hard_database)?.register_scheduled_job(
                &ScheduledJob {
                    job_id: forged_job_id,
                    kind: ScheduledJobKind::FederationReconcile,
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
    assert!(real.completed);
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
