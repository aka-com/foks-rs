use std::io::{Cursor, Read as _, Write as _};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use foks_client::{KvWriteOptions, NamedTeamSecrets};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{BinaryServer, TestAccountSpec, TestClient, TestEnvironment};

#[test]
fn explicit_binary_passes_serve_backup_restore_and_crash_restart_scenario() {
    let Some(binary) = std::env::var_os("FOKS_SERVER_BIN").map(PathBuf::from) else {
        return;
    };
    let source = TestEnvironment::new().unwrap();
    let server = BinaryServer::start(&source, &binary).unwrap();
    let addresses = server.addresses();
    exercise_invalid_tls(addresses.probe);
    let client = TestClient::new(&source, "binary-release-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let repeat_seed = std::env::var("FOKS_TEST_SEED")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    let account_seed = 0xd0_u8.wrapping_add(repeat_seed as u8);
    let created = client
        .create_account(
            &probe.pinned,
            &TestAccountSpec::new("binaryrelease", account_seed),
        )
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    write_file(
        &client,
        &probe.pinned,
        &created.credential,
        &created.authenticated,
        root,
        "before-backup.txt",
        b"binary secret content",
    );
    let member_min = SecretSeed::new([account_seed.wrapping_add(1); 32]);
    let member = SecretSeed::new([account_seed.wrapping_add(2); 32]);
    let admin = SecretSeed::new([account_seed.wrapping_add(3); 32]);
    let owner = SecretSeed::new([account_seed.wrapping_add(4); 32]);
    let removal = SecretSeed::new([account_seed.wrapping_add(5); 32]);
    let team = client
        .foks()
        .create_single_owner_named_team(
            &probe.pinned,
            &created.credential,
            "binaryteam",
            &NamedTeamSecrets {
                member_min,
                member,
                admin,
                owner,
                removal_key: removal,
                team_name_commitment_key: [account_seed.wrapping_add(6); 16],
            },
        )
        .unwrap();
    let mut team_protected = client.open_protected_store().unwrap();
    let mut team_session = client
        .foks()
        .team_kv_write_session(
            &probe.pinned,
            &created.credential,
            &team.authenticated,
            client.soft_state_path(),
            &mut team_protected,
        )
        .unwrap();
    let team_tree = team_session.ensure_root(Role::OWNER, Role::OWNER).unwrap();
    let team_root = team_tree[0].root_directory_id;
    team_session
        .put_file(
            team_root,
            "team-before-backup.txt",
            &mut Cursor::new(b"binary team secret content"),
            KvWriteOptions {
                read_role: Role::OWNER,
                write_role: Role::OWNER,
                overwrite: false,
                expected_version: None,
            },
        )
        .unwrap();
    drop(team_session);
    drop(team_protected);
    let backup = server.backup_named("binary-release").unwrap();
    let first_exit = server.graceful_shutdown().unwrap();
    assert!(first_exit.status.success());
    assert!(String::from_utf8_lossy(&first_exit.stderr).contains("FOKS server ready"));

    let restored =
        TestEnvironment::restore_backup_with_binary(&binary, &backup, addresses).unwrap();
    let restored_server = BinaryServer::start(&restored, &binary).unwrap();
    let reconstructed = TestClient::new(&source, "binary-release-client").unwrap();
    let pinned = reconstructed.pinned_host().unwrap();
    assert_eq!(pinned.host_id(), probe.pinned.host_id());
    let authenticated = reconstructed
        .foks()
        .authenticate_and_pin(&pinned, &created.credential)
        .unwrap();
    let tree = reconstructed
        .foks()
        .sync_user_kv(
            &pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree[0].root_directory_id, root);
    assert_eq!(tree[0].entries.len(), 1);
    assert_eq!(tree[0].entries[0].name, b"before-backup.txt");
    let restored_team = reconstructed
        .foks()
        .load_and_pin_team(
            &pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            &team.team,
        )
        .unwrap();
    let team_tree = reconstructed
        .foks()
        .sync_team_kv(
            &pinned,
            &created.credential,
            &restored_team,
            reconstructed.soft_state_path(),
        )
        .unwrap();
    assert_eq!(team_tree[0].root_directory_id, team_root);
    assert_eq!(team_tree[0].entries[0].name, b"team-before-backup.txt");
    write_file(
        &reconstructed,
        &pinned,
        &created.credential,
        &authenticated,
        root,
        "before-crash.txt",
        b"committed before forced termination",
    );
    let forced_exit = restored_server.force_shutdown().unwrap();
    assert!(!forced_exit.status.success());

    let restarted = BinaryServer::start(&restored, &binary).unwrap();
    let after_crash = TestClient::new(&source, "binary-release-client").unwrap();
    let pinned = after_crash.pinned_host().unwrap();
    let authenticated = after_crash
        .foks()
        .authenticate_and_pin(&pinned, &created.credential)
        .unwrap();
    let tree = after_crash
        .foks()
        .sync_user_kv(
            &pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            after_crash.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree[0].entries.len(), 2);
    let final_exit = restarted.graceful_shutdown().unwrap();
    assert!(final_exit.status.success());

    let mut logs = Vec::new();
    logs.extend_from_slice(&first_exit.stdout);
    logs.extend_from_slice(&first_exit.stderr);
    logs.extend_from_slice(&forced_exit.stdout);
    logs.extend_from_slice(&forced_exit.stderr);
    logs.extend_from_slice(&final_exit.stdout);
    logs.extend_from_slice(&final_exit.stderr);
    assert!(!contains_bytes(&logs, created.credential.seed.as_slice()));
    for certificate in &created.credential.certificate_chain {
        assert!(!contains_bytes(&logs, certificate));
    }
    let logs = String::from_utf8_lossy(&logs);
    assert!(logs.contains("connection_id="));
    assert!(logs.contains("class="));
    let account_seed_marker = format!("{account_seed:02x}").repeat(8);
    for secret in [
        "binary secret content".to_owned(),
        "binary team secret content".to_owned(),
        "committed before forced termination".to_owned(),
        account_seed_marker,
        "5151515151515151".to_owned(),
    ] {
        assert!(
            !logs.contains(&secret),
            "binary logs leaked test secret marker"
        );
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn exercise_invalid_tls(address: std::net::SocketAddr) {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    stream.write_all(b"not a TLS client hello").unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    assert!(response.len() <= 1024);
}

fn write_file(
    client: &TestClient,
    host: &foks_client::PinnedHost,
    credential: &foks_client::DeviceCredential,
    authenticated: &foks_client::AuthenticatedUserOutcome,
    root: [u8; 16],
    name: &str,
    content: &[u8],
) {
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .user_kv_write_session(
            host,
            credential,
            &authenticated.verified,
            &authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    session
        .put_file(
            root,
            name,
            &mut Cursor::new(content),
            KvWriteOptions {
                read_role: Role::OWNER,
                write_role: Role::OWNER,
                overwrite: false,
                expected_version: None,
            },
        )
        .unwrap();
}
