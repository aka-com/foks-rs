use std::io::Cursor;
use std::sync::{Arc, Barrier};

use foks_client::{AdHocTeamSecrets, KvWriteOptions};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};

fn secrets(seed: u8) -> AdHocTeamSecrets {
    AdHocTeamSecrets {
        member_min: SecretSeed::new([seed; 32]),
        member: SecretSeed::new([seed.wrapping_add(1); 32]),
        admin: SecretSeed::new([seed.wrapping_add(2); 32]),
        owner: SecretSeed::new([seed.wrapping_add(3); 32]),
    }
}

fn options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    }
}

#[test]
fn independent_team_reads_and_kv_writes_progress_under_one_writer() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let owner = TestClient::new(&environment, "team-concurrency-owner").unwrap();
    let owner_probe = owner.probe_and_pin().unwrap();
    let account = owner
        .create_account(
            &owner_probe.pinned,
            &TestAccountSpec::new("teamconcurrency", 0xe1),
        )
        .unwrap();
    let first_team = owner
        .foks()
        .create_single_owner_adhoc_team(&owner_probe.pinned, &account.credential, &secrets(0x51))
        .unwrap();
    let second_team = owner
        .foks()
        .create_single_owner_adhoc_team(&owner_probe.pinned, &account.credential, &secrets(0x61))
        .unwrap();

    let first_client = TestClient::new(&environment, "team-concurrency-first").unwrap();
    let first_probe = first_client.probe_and_pin().unwrap();
    let first_user = first_client
        .foks()
        .authenticate_and_pin(&first_probe.pinned, &account.credential)
        .unwrap();
    let first_loaded = first_client
        .foks()
        .load_and_pin_team(
            &first_probe.pinned,
            &account.credential,
            &first_user.verified,
            &first_user.puks,
            &first_team.team,
        )
        .unwrap();
    let second_client = TestClient::new(&environment, "team-concurrency-second").unwrap();
    let second_probe = second_client.probe_and_pin().unwrap();
    let second_user = second_client
        .foks()
        .authenticate_and_pin(&second_probe.pinned, &account.credential)
        .unwrap();
    let second_loaded = second_client
        .foks()
        .load_and_pin_team(
            &second_probe.pinned,
            &account.credential,
            &second_user.verified,
            &second_user.puks,
            &second_team.team,
        )
        .unwrap();

    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let first_barrier = Arc::clone(&barrier);
        let first_credential = &account.credential;
        let first_client_ref = &first_client;
        let first_host = &first_probe.pinned;
        let first_loaded_ref = &first_loaded;
        let first = scope.spawn(move || {
            first_barrier.wait();
            write_team_file(
                first_client_ref,
                first_host,
                first_credential,
                first_loaded_ref,
                "first.txt",
            )
        });
        let second_barrier = Arc::clone(&barrier);
        let second_credential = &account.credential;
        let second_client_ref = &second_client;
        let second_host = &second_probe.pinned;
        let second_loaded_ref = &second_loaded;
        let second = scope.spawn(move || {
            second_barrier.wait();
            write_team_file(
                second_client_ref,
                second_host,
                second_credential,
                second_loaded_ref,
                "second.txt",
            )
        });
        barrier.wait();
        first.join().unwrap();
        second.join().unwrap();
    });
    assert!(server.metrics().responses_completed > 0);
    server.shutdown().unwrap();
}

fn write_team_file(
    client: &TestClient,
    host: &foks_client::PinnedHost,
    credential: &foks_client::DeviceCredential,
    team: &foks_client::AuthenticatedTeamOutcome,
    name: &str,
) {
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .team_kv_write_session(
            host,
            credential,
            team,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let tree = session.ensure_root(Role::OWNER, Role::OWNER).unwrap();
    session
        .put_file(
            tree[0].root_directory_id,
            name,
            &mut Cursor::new(name.as_bytes()),
            options(),
        )
        .unwrap();
    assert_eq!(session.sync().unwrap()[0].entries.len(), 1);
}
