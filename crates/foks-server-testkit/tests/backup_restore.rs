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
fn matching_backup_restores_under_the_existing_pin_and_credential() {
    let source = TestEnvironment::new().unwrap();
    let server = source.start_server().unwrap();
    let addresses = server.addresses();
    let client = TestClient::new(&source, "restore-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let created = client
        .create_account(&probe.pinned, &TestAccountSpec::new("restoreuser", 0x71))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .user_kv_write_session(
            &probe.pinned,
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    session
        .put_file(
            root,
            "before-backup.txt",
            &mut Cursor::new(b"durable backup content"),
            options(),
        )
        .unwrap();
    drop(session);
    let team = client
        .foks()
        .create_single_owner_named_team(
            &probe.pinned,
            &created.credential,
            "restoreteam",
            &NamedTeamSecrets {
                member_min: SecretSeed::new([0x31; 32]),
                member: SecretSeed::new([0x32; 32]),
                admin: SecretSeed::new([0x33; 32]),
                owner: SecretSeed::new([0x34; 32]),
                removal_key: SecretSeed::new([0x35; 32]),
                team_name_commitment_key: [0x36; 16],
            },
        )
        .unwrap();
    let mut team_session = client
        .foks()
        .team_kv_write_session(
            &probe.pinned,
            &created.credential,
            &team.authenticated,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let team_tree = team_session.ensure_root(Role::OWNER, Role::OWNER).unwrap();
    let team_root = team_tree[0].root_directory_id;
    team_session
        .put_file(
            team_root,
            "team-before-backup.txt",
            &mut Cursor::new(b"durable team backup content"),
            options(),
        )
        .unwrap();
    drop(team_session);
    drop(protected);
    let artifacts = server.backup_named("matching").unwrap();
    assert!(server.backup_is_valid(&artifacts).unwrap());
    let original_host = probe.pinned.host_id().clone();
    server.shutdown().unwrap();

    let restored = TestEnvironment::restore_backup(&artifacts, addresses).unwrap();
    let restored_server = restored.start_server().unwrap();
    assert_eq!(restored_server.addresses(), addresses);
    let reconstructed = TestClient::new(&source, "restore-client").unwrap();
    let pinned = reconstructed.pinned_host().unwrap();
    assert_eq!(pinned.host_id(), &original_host);
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
    assert_eq!(
        tree[0].entries[0].content.as_deref(),
        Some(b"durable backup content".as_slice())
    );
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
    assert_eq!(
        team_tree[0].entries[0].content.as_deref(),
        Some(b"durable team backup content".as_slice())
    );
    restored_server.shutdown().unwrap();
}

#[test]
fn backup_manifest_database_and_key_failures_are_closed() {
    let source = TestEnvironment::new().unwrap();
    let server = source.start_server().unwrap();
    let addresses = server.addresses();
    let client = TestClient::new(&source, "failure-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    client
        .create_account(&probe.pinned, &TestAccountSpec::new("failureuser", 0x72))
        .unwrap();
    let wrong_root = server.backup_named("wrong-root").unwrap();
    let missing_key = server.backup_named("missing-key").unwrap();
    let modified_key = server.backup_named("modified-key").unwrap();
    let swapped_key = server.backup_named("swapped-key").unwrap();
    let bad_manifest = server.backup_named("bad-manifest").unwrap();
    let truncated = server.backup_named("truncated").unwrap();
    let donor_environment = TestEnvironment::new().unwrap();
    let donor_server = donor_environment.start_server().unwrap();
    let donor = donor_server.backup_named("donor").unwrap();
    donor_server.shutdown().unwrap();
    server.shutdown().unwrap();

    let wrong =
        TestEnvironment::restore_backup_with_root_key(&wrong_root, addresses, [0x99; 32]).unwrap();
    let error = expect_server_error(wrong.start_server());
    assert_bounded_non_secret(&error);
    assert_bounded_non_secret(&expect_server_error(wrong.start_server()));

    std::fs::remove_file(missing_key.key_directory.join("host.key")).unwrap();
    let error = expect_environment_error(TestEnvironment::restore_backup(&missing_key, addresses));
    assert_bounded_non_secret(&error);

    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .append(true)
        .open(modified_key.key_directory.join("merkle.key"))
        .unwrap()
        .write_all(b"corrupt")
        .unwrap();
    let modified = TestEnvironment::restore_backup(&modified_key, addresses).unwrap();
    let error = expect_server_error(modified.start_server());
    assert_bounded_non_secret(&error);

    std::fs::copy(
        donor.key_directory.join("host.key"),
        swapped_key.key_directory.join("host.key"),
    )
    .unwrap();
    let swapped = TestEnvironment::restore_backup(&swapped_key, addresses).unwrap();
    let error = expect_server_error(swapped.start_server());
    assert_bounded_non_secret(&error);

    std::fs::OpenOptions::new()
        .append(true)
        .open(&bad_manifest.key_manifest)
        .unwrap()
        .write_all(b"unexpected\n")
        .unwrap();
    let error = expect_environment_error(TestEnvironment::restore_backup(&bad_manifest, addresses));
    assert_bounded_non_secret(&error);

    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&truncated.database)
        .unwrap();
    file.set_len(1024).unwrap();
    let error = expect_environment_error(TestEnvironment::restore_backup(&truncated, addresses));
    assert_bounded_non_secret(&error);
}

#[test]
fn online_backup_is_coherent_while_kv_writes_continue() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let addresses = server.addresses();
    let client = TestClient::new(&environment, "concurrent-backup-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let created = client
        .create_account(
            &probe.pinned,
            &TestAccountSpec::new("concurrentbackup", 0x73),
        )
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let barrier = std::sync::Barrier::new(2);
    let artifacts = std::thread::scope(|scope| {
        let backup = scope.spawn(|| {
            barrier.wait();
            server.operator_backup_named("concurrent")
        });
        let mut protected = client.open_protected_store().unwrap();
        let mut session = client
            .foks()
            .user_kv_write_session(
                &probe.pinned,
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                client.soft_state_path(),
                &mut protected,
            )
            .unwrap();
        barrier.wait();
        for index in 0..12 {
            session
                .put_file(
                    root,
                    &format!("concurrent-{index:02}.txt"),
                    &mut Cursor::new(format!("content-{index}")),
                    options(),
                )
                .unwrap();
        }
        drop(session);
        backup.join().unwrap().unwrap()
    });
    assert!(server.backup_is_valid(&artifacts).unwrap());
    server.shutdown().unwrap();

    let restored = TestEnvironment::restore_backup(&artifacts, addresses).unwrap();
    let restored_server = restored.start_server().unwrap();
    let reconstructed = TestClient::new_with_fresh_soft_state(
        &environment,
        "concurrent-backup-client",
        "concurrent-backup-restored-cache",
    )
    .unwrap();
    let pinned = reconstructed.pinned_host().unwrap();
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
    assert_eq!(tree.len(), 1);
    assert!(tree[0].entries.len() <= 12);
    for (index, entry) in tree[0].entries.iter().enumerate() {
        assert_eq!(entry.name, format!("concurrent-{index:02}.txt").as_bytes());
        assert_eq!(
            entry.content.as_deref(),
            Some(format!("content-{index}").as_bytes())
        );
    }
    restored_server.shutdown().unwrap();
}

fn assert_bounded_non_secret(error: &foks_server::Error) {
    let diagnostic = error.to_string();
    assert!(
        diagnostic.len() <= 240,
        "unbounded diagnostic: {diagnostic}"
    );
    assert!(!diagnostic.contains("failureuser"));
    assert!(!diagnostic.contains("[153, 153"));
}

fn expect_server_error(
    result: foks_server::Result<foks_server_testkit::InProcessServer>,
) -> foks_server::Error {
    match result {
        Ok(_) => panic!("server unexpectedly started"),
        Err(error) => error,
    }
}

fn expect_environment_error(result: foks_server::Result<TestEnvironment>) -> foks_server::Error {
    match result {
        Ok(_) => panic!("restore unexpectedly succeeded"),
        Err(error) => error,
    }
}
