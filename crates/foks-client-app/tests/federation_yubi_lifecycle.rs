//! Integration tests for the hardware-backed federated security responder.
//! Verifies that when a software administrator device is revoked, a hardware
//! YubiKey administrator can rekey and maintain member keys for the federated
//! team.

use std::time::{SystemTime, UNIX_EPOCH};

use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend,
    FederationDestinationRole, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy,
    TeamMemberRole, TrustRoot, YubiProvisionInput,
};
use foks_keystore::{EncryptedFileSecretStore, SecretStore as _};
use foks_server_testkit::TestEnvironment;
use foks_yubi::{MockYubiProvider, Pin, SlotId, YubiProvider};

#[test]
fn yubi_only_administrator_defers_unattended_and_runs_the_explicit_federation_responder() {
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
    // Every phase below opens a fresh session, exactly as a separate CLI or
    // agent invocation would. A single long-lived client would otherwise
    // carry pooled sockets across the multi-second hardware enrollment and
    // hit the server's idle timeout, which has nothing to do with federation.
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

    let local = ProfileSession::open(&registry, "local").unwrap();
    credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            local.create_account(
                "local-owner",
                "yubifedlocalowner",
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
                "yubilocalteam",
                &mut vault,
                &master,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    let remote = ProfileSession::open(&registry, "remote").unwrap();
    credentials
        .with_checked_session(&remote, |remote| {
            let mut store = open_store(remote.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            remote.create_account(
                "remote-owner",
                "yubifedremoteowner",
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
                "yubiremoteteam",
                &mut vault,
                &master,
            )?;
            remote.create_account(
                "remote-member",
                "yubifedremotemember",
                "remote federation member",
                "remote-member@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            remote.add_local_team_member(
                "remote-team",
                "yubifedremotemember",
                TeamMemberRole::Admin,
                &mut vault,
                &master,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    let local = ProfileSession::open(&registry, "local").unwrap();
    let remote = ProfileSession::open(&registry, "remote").unwrap();
    let admitted = credentials
        .with_checked_sessions(&local, &remote, |local, remote| {
            let mut local_store = open_store(local.paths(), &master)?;
            let mut remote_store = open_store(remote.paths(), &master)?;
            local.admit_federated_team(
                remote,
                "local-team",
                "remote-team",
                FederationDestinationRole::Member { visibility: 0 },
                &mut AccountVault::new(&mut local_store),
                &mut AccountVault::new(&mut remote_store),
                &master,
            )
        })
        .unwrap();
    assert!(admitted.active);

    // Enroll a YubiKey for the local owner, then delete the software account
    // record. The user chain still lists the software device, but this profile
    // can no longer act with it, which is exactly the post-revocation state a
    // hardware-only administrator is left in.
    let pin = Pin::new("123456").unwrap();
    let provider = MockYubiProvider::with_card("federation-yubikey", 74001, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let local = ProfileSession::open(&registry, "local").unwrap();
    credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            local.provision_yubi_device(
                YubiProvisionInput {
                    source_alias: "local-owner".to_owned(),
                    target_alias: "local-hardware".to_owned(),
                    device_name: "federation owner key".to_owned(),
                    serial: 3,
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
    let local = ProfileSession::open(&registry, "local").unwrap();
    credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            assert!(store.remove("account.local-owner")?);
            let mut vault = AccountVault::new(&mut store);
            assert!(vault.aliases()?.is_empty());
            assert_eq!(vault.yubi_aliases()?, vec!["local-hardware".to_owned()]);
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    // Demote the remote administrator so the federated roster genuinely needs
    // a new generation. Without the responder this stale key would keep
    // appearing in the local team's federated material.
    let original_generation = federated_generation(&local);
    let remote = ProfileSession::open(&registry, "remote").unwrap();
    credentials
        .with_checked_session(&remote, |remote| {
            let mut store = open_store(remote.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            let party_id = remote
                .list_team_members("remote-team", &mut vault)?
                .into_iter()
                .find(|member| member.username.as_deref() == Some("yubifedremotemember"))
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

    // The unattended scheduler must never prompt for a PIN. It defers the
    // federation job with an action-required reason instead of failing it,
    // so the job is not backed off and the operator can see what is needed.
    let due = micros_now().saturating_add(20 * 60 * 1_000_000);
    let local = ProfileSession::open(&registry, "local").unwrap();
    let deferred = credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            local.run_due_jobs_with_federation(
                due,
                &mut AccountVault::new(&mut store),
                &registry,
                &credentials,
                &master,
            )
        })
        .unwrap();
    let federation_run = deferred
        .runs
        .iter()
        .find(|run| run.job_id_hex == admitted.scheduled_job_id_hex)
        .expect("the federation job is due");
    assert!(
        federation_run.completed && federation_run.error.is_none(),
        "a locked YubiKey must not fail the job: {federation_run:?}"
    );
    let reason = federation_run
        .deferred
        .as_deref()
        .expect("a hardware-only administrator must report an action-required deferral");
    assert!(
        reason.contains("local-hardware"),
        "the deferral names the key to unlock: {reason}"
    );
    assert_eq!(
        federated_generation(&local),
        original_generation,
        "a deferred run must not change federated material"
    );

    // With the same key unlocked, the established Yubi security-sync flow
    // performs the refresh the unattended scheduler correctly refused to
    // attempt. This drives the whole chain: the unlock helper, the per-binding
    // responder, and the generalized cross-profile refresh underneath it.
    let local = ProfileSession::open(&registry, "local").unwrap();
    let synced = credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            local.sync_yubi_account_with_federation(
                "local-hardware",
                Pin::new("123456")?,
                &provider,
                &mut AccountVault::new(&mut store),
                &registry,
                &credentials,
                &master,
            )
        })
        .unwrap();
    assert_eq!(synced.federation.len(), 1);
    assert_eq!(synced.federation[0].local_team_alias, "local-team");
    assert_eq!(synced.federation[0].local_profile, "local");
    assert!(
        synced.federation[0].refreshed && synced.federation[0].deferred.is_none(),
        "unexpected deferral: {:?}",
        synced.federation[0]
    );
    assert!(
        federated_generation(&local) > original_generation,
        "the hardware responder must advance the federated roster"
    );

    // An unlocked key offered for another profile must not be accepted as
    // this profile's administrator.
    let local = ProfileSession::open(&registry, "local").unwrap();
    let foreign = credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            local.with_unlocked_yubi(
                "local-hardware",
                &Pin::new("123456")?,
                &provider,
                &mut vault,
                |actor, vault| {
                    let mislabeled = foks_client_app::UnlockedYubiActor {
                        profile: "remote",
                        alias: actor.alias,
                        credential: actor.credential,
                    };
                    local.refresh_federated_security_with_unlocked_yubi(
                        "local-team",
                        &[mislabeled],
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
        !foreign.refreshed && foreign.deferred.is_some(),
        "a credential bound to another profile must not act here: {foreign:?}"
    );
}

/// Reads the generation of the federated (host-scoped) roster row straight
/// from the authenticated local projection. The vault-level listing needs a
/// software account, which this scenario has deliberately removed.
fn federated_generation(session: &ProfileSession) -> i64 {
    let database = rusqlite::Connection::open_with_flags(
        &session.paths().hard_database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    database
        .query_row(
            "SELECT generation FROM team_members WHERE length(scoped_host_id) = 33",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
}

fn open_store(
    paths: &foks_client_app::ProfilePaths,
    master: &[u8; 32],
) -> Result<EncryptedFileSecretStore, foks_client_app::Error> {
    Ok(EncryptedFileSecretStore::open(
        &paths.credential_store,
        derive_vault_key(master),
    )?)
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
            label: None,
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

/// The far side of a federation can also be hardware-only. An unattended
/// scheduler can never unlock two YubiKeys, so this is the explicit workflow
/// the CLI and agent expose: each profile's durable Yubi record is read under
/// its own short-lived checked session, the devices are opened by the caller,
/// and only then is the refresh driven under the local session. Holding both
/// operation locks across the refresh would make the remote side, which the
/// refresh re-acquires without waiting, look permanently busy.
#[test]
fn two_hardware_only_profiles_refresh_through_the_explicit_two_key_workflow() {
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
    let credentials = ClientCredentials::open(&state).unwrap();
    {
        let local = ProfileSession::open(&registry, "local").unwrap();
        let remote = ProfileSession::open(&registry, "remote").unwrap();
        credentials
            .with_checked_sessions(&local, &remote, |local, remote| {
                local.probe_and_pin()?;
                remote.probe_and_pin()?;
                Ok::<_, foks_client_app::Error>(())
            })
            .unwrap();
    }
    let master = credentials.master_key().unwrap();

    let local = ProfileSession::open(&registry, "local").unwrap();
    credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            local.create_account(
                "local-owner",
                "twokeylocalowner",
                "local owner",
                "local@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            local.create_named_team(
                "local-owner",
                "local-team",
                "twokeylocalteam",
                &mut vault,
                &master,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    let remote = ProfileSession::open(&registry, "remote").unwrap();
    credentials
        .with_checked_session(&remote, |remote| {
            let mut store = open_store(remote.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            remote.create_account(
                "remote-owner",
                "twokeyremoteowner",
                "remote owner",
                "remote@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            remote.create_named_team(
                "remote-owner",
                "remote-team",
                "twokeyremoteteam",
                &mut vault,
                &master,
            )?;
            remote.create_account(
                "remote-member",
                "twokeyremotemember",
                "remote member",
                "remote-member@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            remote.add_local_team_member(
                "remote-team",
                "twokeyremotemember",
                TeamMemberRole::Admin,
                &mut vault,
                &master,
            )?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();

    let local = ProfileSession::open(&registry, "local").unwrap();
    let remote = ProfileSession::open(&registry, "remote").unwrap();
    credentials
        .with_checked_sessions(&local, &remote, |local, remote| {
            let mut local_store = open_store(local.paths(), &master)?;
            let mut remote_store = open_store(remote.paths(), &master)?;
            local.admit_federated_team(
                remote,
                "local-team",
                "remote-team",
                FederationDestinationRole::Member { visibility: 0 },
                &mut AccountVault::new(&mut local_store),
                &mut AccountVault::new(&mut remote_store),
                &master,
            )
        })
        .unwrap();

    // Demote the remote administrator so the federated roster genuinely needs
    // a new generation, then take both profiles hardware-only.
    let remote = ProfileSession::open(&registry, "remote").unwrap();
    credentials
        .with_checked_session(&remote, |remote| {
            let mut store = open_store(remote.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            let party_id = remote
                .list_team_members("remote-team", &mut vault)?
                .into_iter()
                .find(|member| member.username.as_deref() == Some("twokeyremotemember"))
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
    let original_generation = federated_generation(&local);

    let pin = Pin::new("123456").unwrap();
    let local_provider = MockYubiProvider::with_card("two-key-local", 75001, &pin).unwrap();
    let local_card = local_provider.cards().unwrap().remove(0);
    let remote_provider = MockYubiProvider::with_card("two-key-remote", 75002, &pin).unwrap();
    let remote_card = remote_provider.cards().unwrap().remove(0);
    for (profile, card, provider, source, target) in [
        (
            "local",
            local_card,
            &local_provider as &dyn YubiProvider,
            "local-owner",
            "local-hardware",
        ),
        (
            "remote",
            remote_card,
            &remote_provider as &dyn YubiProvider,
            "remote-owner",
            "remote-hardware",
        ),
    ] {
        let session = ProfileSession::open(&registry, profile).unwrap();
        credentials
            .with_checked_session(&session, |session| {
                let mut store = open_store(session.paths(), &master)?;
                let mut vault = AccountVault::new(&mut store);
                session.provision_yubi_device(
                    YubiProvisionInput {
                        source_alias: source.to_owned(),
                        target_alias: target.to_owned(),
                        device_name: format!("{profile} owner key"),
                        serial: 4,
                        card,
                        signing_slot: SlotId::new(0x82)?,
                        pq_slot: SlotId::new(0x83)?,
                        retry_configuration: None,
                    },
                    Pin::new("123456")?,
                    provider,
                    &mut vault,
                    &master,
                )?;
                assert!(store.remove(&format!("account.{source}"))?);
                Ok::<_, foks_client_app::Error>(())
            })
            .unwrap();
    }

    // Read the remote profile's durable Yubi record under its own short-lived
    // session and release that session before taking the local one.
    let remote = ProfileSession::open(&registry, "remote").unwrap();
    let remote_loaded = credentials
        .with_checked_session(&remote, |remote| {
            let mut store = open_store(remote.paths(), &master)?;
            AccountVault::new(&mut store).yubi_account("remote-hardware")
        })
        .unwrap();
    let remote_device = remote_provider
        .open(&remote_loaded.locator, Some(&Pin::new("123456").unwrap()))
        .unwrap();
    let remote_credential = remote_loaded.credential(remote_device.as_ref());
    let remote_actor = foks_client_app::UnlockedYubiActor {
        profile: "remote",
        alias: "remote-hardware",
        credential: &remote_credential,
    };

    let local = ProfileSession::open(&registry, "local").unwrap();
    let report = credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            local.with_unlocked_yubi(
                "local-hardware",
                &Pin::new("123456")?,
                &local_provider,
                &mut vault,
                |local_actor, vault| {
                    local.refresh_federated_security_with_unlocked_yubi(
                        "local-team",
                        &[local_actor, remote_actor],
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
        "a two-key refresh must not defer once both keys are unlocked: {report:?}"
    );
    assert!(
        federated_generation(&local) > original_generation,
        "the two-key responder must advance the federated roster"
    );

    // Without the remote key the same request must defer rather than fail,
    // because the remote side has no software administrator left either.
    let local = ProfileSession::open(&registry, "local").unwrap();
    let deferred = credentials
        .with_checked_session(&local, |local| {
            let mut store = open_store(local.paths(), &master)?;
            let mut vault = AccountVault::new(&mut store);
            local.with_unlocked_yubi(
                "local-hardware",
                &Pin::new("123456")?,
                &local_provider,
                &mut vault,
                |local_actor, vault| {
                    local.refresh_federated_security_with_unlocked_yubi(
                        "local-team",
                        &[local_actor],
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
        !deferred.refreshed,
        "a locked remote key must defer: {deferred:?}"
    );
    let reason = deferred.deferred.as_deref().expect("an explicit deferral");
    assert!(
        reason.contains("remote-hardware"),
        "the deferral names the remote key to unlock: {reason}"
    );
}
