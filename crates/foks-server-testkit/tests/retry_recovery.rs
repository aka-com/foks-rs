use std::io::Cursor;

use foks_client::{KvWriteOptions, Passphrase, SoftwareAccountRequest, SoftwareAccountSecrets};
use foks_client_db::HardStateStore;
use foks_proto::{EntityId, InviteCode, Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestFault};

fn account_request(username: &str) -> SoftwareAccountRequest {
    SoftwareAccountRequest {
        username_utf8: username.to_owned(),
        device_name: format!("{username} device"),
        invite_code: InviteCode::Empty,
        email: format!("{username}@example.test"),
        passphrase: None,
    }
}

fn account_secrets(seed: u8) -> SoftwareAccountSecrets {
    SoftwareAccountSecrets::new(
        SecretSeed::new([seed; 32]),
        SecretSeed::new([seed.wrapping_add(1); 32]),
        [seed.wrapping_add(2); 17],
    )
}

#[test]
fn disconnect_before_signup_commit_leaves_no_identity_or_receipt() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "before-commit-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let hit_before = environment.arm_fault(TestFault::SignupBeforeCommit);
    let mut protected = client.open_protected_store().unwrap();
    let error = expect_account_error(client.foks().create_software_account(
        &probe.pinned,
        account_request("beforecommit"),
        account_secrets(0x81),
        client.soft_state_path(),
        &mut protected,
    ));
    assert!(matches!(error, foks_client::Error::Rpc(_)));
    assert_eq!(environment.fault_hits(), hit_before + 1);
    let operation = only_pending_operation(&client, &probe.pinned);
    assert!(server.identity(&operation.subject_id).unwrap().is_none());
    assert!(server
        .request_receipt(&[0x83; 17], &operation.request_hash)
        .unwrap()
        .is_none());
    drop(protected);
    server.shutdown().unwrap();
}

#[test]
fn signup_commit_with_lost_or_partial_response_reconciles_without_replay() {
    for (client_id, fault, seed) in [
        (
            "after-commit-client",
            TestFault::SignupAfterCommitBeforeResponse,
            0x83,
        ),
        (
            "partial-response-client",
            TestFault::SignupDuringResponseWrite,
            0x84,
        ),
    ] {
        let environment = TestEnvironment::new().unwrap();
        let server = environment.start_server().unwrap();
        let client = TestClient::new(&environment, client_id).unwrap();
        let probe = client.probe_and_pin().unwrap();
        let hit_before = environment.arm_fault(fault);
        let mut protected = client.open_protected_store().unwrap();
        let error = expect_account_error(client.foks().create_software_account(
            &probe.pinned,
            account_request(client_id),
            account_secrets(seed),
            client.soft_state_path(),
            &mut protected,
        ));
        assert!(matches!(error, foks_client::Error::Rpc(_)));
        assert_eq!(environment.fault_hits(), hit_before + 1);
        let operation = only_pending_operation(&client, &probe.pinned);
        assert!(server.identity(&operation.subject_id).unwrap().is_some());
        assert!(matches!(
            server.request_receipt(&[seed.wrapping_add(2); 17], &operation.request_hash),
            Ok(Some(_)) | Err(foks_server_db::Error::ReceiptConflict)
        ));
        assert_eq!(server.current_root().unwrap().unwrap().epoch, 2);
        drop(protected);
        drop(client);

        let reconstructed = TestClient::new(&environment, client_id).unwrap();
        let uid = EntityId::from_bytes(operation.subject_id).unwrap();
        let device_id = EntityId::from_bytes(operation.scope_id).unwrap();
        let mut reopened = reconstructed.open_protected_store().unwrap();
        let recovered = reconstructed
            .foks()
            .resume_software_account_for_credential(
                &probe.pinned,
                &uid,
                &device_id,
                reconstructed.soft_state_path(),
                &mut reopened,
            )
            .unwrap();
        assert_eq!(recovered.credential.uid, uid);
        assert_eq!(server.current_root().unwrap().unwrap().epoch, 2);
        assert!(HardStateStore::open(reconstructed.hard_state_path())
            .unwrap()
            .pending_mutations(probe.pinned.host_id().as_bytes())
            .unwrap()
            .is_empty());
        server.shutdown().unwrap();
    }
}

#[test]
fn passphrase_updates_reconcile_lost_or_partial_responses_by_exact_readback() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "passphrase-fault-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let created = client
        .create_account(
            &probe.pinned,
            &TestAccountSpec::new("passphrasefault", 0x86),
        )
        .unwrap();

    let first = Passphrase::new("fault injected first passphrase").unwrap();
    let hit_before = environment.arm_fault(TestFault::PassphraseSetAfterCommitBeforeResponse);
    let enrolled = client
        .foks()
        .set_passphrase(&probe.pinned, &created.credential, &first)
        .unwrap();
    assert_eq!(enrolled.generation, 1);
    assert_eq!(environment.fault_hits(), hit_before + 1);
    assert_eq!(
        client
            .foks()
            .verify_passphrase(&probe.pinned, &created.credential, &first)
            .unwrap()
            .generation,
        1
    );

    let second = Passphrase::new("fault injected second passphrase").unwrap();
    let hit_before = environment.arm_fault(TestFault::PassphraseChangeDuringResponseWrite);
    let changed = client
        .foks()
        .change_passphrase(&probe.pinned, &created.credential, &second)
        .unwrap();
    assert_eq!(changed.generation, 2);
    assert_eq!(environment.fault_hits(), hit_before + 1);
    assert_eq!(
        client
            .foks()
            .verify_passphrase(&probe.pinned, &created.credential, &second)
            .unwrap()
            .generation,
        2
    );
    assert!(client
        .foks()
        .verify_passphrase(&probe.pinned, &created.credential, &first)
        .is_err());
    server.shutdown().unwrap();
}

#[test]
fn disconnect_between_large_file_chunks_is_invisible_and_retryable() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "chunk-fault-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let created = client
        .create_account(&probe.pinned, &TestAccountSpec::new("chunkfault", 0x85))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let options = KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    };
    let hit_before = environment.arm_fault(TestFault::BetweenLargeFileChunks);
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
    let bytes = vec![0x5a; 4 * 1024 * 1024 + 2];
    assert!(session
        .put_file(root, "chunked.bin", &mut Cursor::new(&bytes), options)
        .is_err());
    assert_eq!(environment.fault_hits(), hit_before + 1);
    drop(session);
    drop(protected);
    drop(client);

    environment.advance_clock(24 * 60 * 60 * 1_000_000 + 1);
    let (maintenance, _) = server.run_maintenance().unwrap();
    assert_eq!(maintenance.uploads, 1);

    let reconstructed = TestClient::new(&environment, "chunk-fault-client").unwrap();
    let authenticated = reconstructed
        .foks()
        .authenticate_and_pin(&probe.pinned, &created.credential)
        .unwrap();
    assert!(reconstructed
        .foks()
        .sync_user_kv(
            &probe.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
        )
        .unwrap()[0]
        .entries
        .is_empty());
    let mut reopened = reconstructed.open_protected_store().unwrap();
    let mut retry = reconstructed
        .foks()
        .user_kv_write_session(
            &probe.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
            &mut reopened,
        )
        .unwrap();
    retry
        .put_file(root, "chunked.bin", &mut Cursor::new(&bytes), options)
        .unwrap();
    let tree = retry.sync().unwrap();
    assert_eq!(tree[0].entries.len(), 1);
    assert_eq!(tree[0].entries[0].large_file_size, Some(bytes.len() as u64));
    drop(retry);
    environment.advance_clock(24 * 60 * 60 * 1_000_000 + 1);
    let (maintenance, _) = server.run_maintenance().unwrap();
    assert_eq!(maintenance.uploads, 0);
    let tree = reconstructed
        .foks()
        .sync_user_kv(
            &probe.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree[0].entries[0].large_file_size, Some(bytes.len() as u64));
    server.shutdown().unwrap();
}

fn only_pending_operation(
    client: &TestClient,
    host: &foks_client::PinnedHost,
) -> foks_client_db::MutationOperation {
    let pending = HardStateStore::open(client.hard_state_path())
        .unwrap()
        .pending_mutations(host.host_id().as_bytes())
        .unwrap();
    assert_eq!(pending.len(), 1);
    pending.into_iter().next().unwrap()
}

fn expect_account_error(
    result: foks_client::Result<foks_client::CreatedSoftwareAccount>,
) -> foks_client::Error {
    match result {
        Ok(_) => panic!("account creation unexpectedly succeeded"),
        Err(error) => error,
    }
}
