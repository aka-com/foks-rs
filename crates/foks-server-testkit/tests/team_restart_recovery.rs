use std::io::Cursor;

use foks_client::{KvWriteOptions, NamedTeamSecrets};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};

fn options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    }
}

#[test]
fn named_team_chain_ptks_and_kv_survive_server_restart() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let addresses = server.addresses();
    let client = TestClient::new(&environment, "team-restart-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let account = client
        .create_account(&probe.pinned, &TestAccountSpec::new("teamrestart", 0xd1))
        .unwrap();
    let team = client
        .foks()
        .create_single_owner_named_team(
            &probe.pinned,
            &account.credential,
            "restartteam",
            &NamedTeamSecrets {
                member_min: SecretSeed::new([0x41; 32]),
                member: SecretSeed::new([0x42; 32]),
                admin: SecretSeed::new([0x43; 32]),
                owner: SecretSeed::new([0x44; 32]),
                removal_key: SecretSeed::new([0x45; 32]),
                team_name_commitment_key: [0x46; 16],
            },
        )
        .unwrap();
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .team_kv_write_session(
            &probe.pinned,
            &account.credential,
            &team.authenticated,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let tree = session.ensure_root(Role::OWNER, Role::OWNER).unwrap();
    let root = tree[0].root_directory_id;
    session
        .put_file(
            root,
            "before-restart.txt",
            &mut Cursor::new(b"durable team content"),
            options(),
        )
        .unwrap();
    drop(session);
    drop(protected);
    server.shutdown().unwrap();

    let restarted = environment.start_server().unwrap();
    assert_eq!(restarted.addresses(), addresses);
    let reconstructed = TestClient::new_with_fresh_soft_state(
        &environment,
        "team-restart-client",
        "team-restart-fresh-cache",
    )
    .unwrap();
    let pinned = reconstructed.pinned_host().unwrap();
    let authenticated = reconstructed
        .foks()
        .authenticate_and_pin(&pinned, &account.credential)
        .unwrap();
    let loaded = reconstructed
        .foks()
        .load_and_pin_team(
            &pinned,
            &account.credential,
            &authenticated.verified,
            &authenticated.puks,
            &team.team,
        )
        .unwrap();
    assert_eq!(loaded.verified.team_name(), b"restartteam");
    assert_eq!(loaded.ptks.len(), 4);
    let tree = reconstructed
        .foks()
        .sync_team_kv(
            &pinned,
            &account.credential,
            &loaded,
            reconstructed.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree[0].root_directory_id, root);
    assert_eq!(tree[0].entries[0].name, b"before-restart.txt");
    assert_eq!(
        tree[0].entries[0].content.as_deref(),
        Some(b"durable team content".as_slice())
    );
    restarted.shutdown().unwrap();
}
