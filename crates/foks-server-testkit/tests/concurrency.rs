use std::io::Cursor;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use foks_client::{KvWriteOptions, SoftwareAccountRequest, SoftwareAccountSecrets};
use foks_proto::{InviteCode, Role, SecretSeed, SoftwareSignupArgument};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestProfile};
use rustls::pki_types::{CertificateDer, ServerName};
use std::io::Write as _;

fn options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    }
}

#[test]
fn independently_signed_signups_from_one_merkle_root_both_commit() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "same-root-signups").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let first_request = software_account_request("samerootfirst");
    let second_request = software_account_request("samerootsecond");
    let first_secrets = SoftwareAccountSecrets::new(
        SecretSeed::new([0x21; 32]),
        SecretSeed::new([0x22; 32]),
        [0x23; 17],
    );
    let second_secrets = SoftwareAccountSecrets::new(
        SecretSeed::new([0x31; 32]),
        SecretSeed::new([0x32; 32]),
        [0x33; 17],
    );
    let first_reservation = client
        .foks()
        .reserve_username(&probe.pinned, &first_request.username_utf8)
        .unwrap();
    let second_reservation = client
        .foks()
        .reserve_username(&probe.pinned, &second_request.username_utf8)
        .unwrap();
    let (_, merkle) = client.foks().advance_merkle_root(&probe.pinned).unwrap();
    let first = client
        .foks()
        .prepare_software_account(
            &probe.pinned,
            &merkle,
            &first_request,
            first_reservation,
            &first_secrets,
        )
        .unwrap();
    let second = client
        .foks()
        .prepare_software_account(
            &probe.pinned,
            &merkle,
            &second_request,
            second_reservation,
            &second_secrets,
        )
        .unwrap();
    let first_frame = signup_frame(&first, &first_request, [0x23; 17]);
    let second_frame = signup_frame(&second, &second_request, [0x33; 17]);

    submit_signup(&environment, &probe.pinned, &first_frame);
    submit_signup(&environment, &probe.pinned, &second_frame);

    assert!(server
        .identity(first.eldest.uid.as_bytes())
        .unwrap()
        .is_some());
    assert!(server
        .identity(second.eldest.uid.as_bytes())
        .unwrap()
        .is_some());
    server.shutdown().unwrap();
}

#[test]
fn independent_users_make_progress_under_barrier_started_reads_and_writes() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let first_client = TestClient::new(&environment, "parallel-first").unwrap();
    let second_client = TestClient::new(&environment, "parallel-second").unwrap();
    let first_probe = first_client.probe_and_pin().unwrap();
    let second_probe = second_client.probe_and_pin().unwrap();
    let first = first_client
        .create_account(
            &first_probe.pinned,
            &TestAccountSpec::new("parallelfirst", 0x91),
        )
        .unwrap();
    let second = second_client
        .create_account(
            &second_probe.pinned,
            &TestAccountSpec::new("parallelsecond", 0x92),
        )
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let first_barrier = Arc::clone(&barrier);
        let first_client = &first_client;
        let first_probe = &first_probe.pinned;
        let first_account = &first;
        let first_thread = scope.spawn(move || {
            first_barrier.wait();
            write_and_read(first_client, first_probe, first_account, "first.txt", b'a')
        });
        let second_barrier = Arc::clone(&barrier);
        let second_client = &second_client;
        let second_probe = &second_probe.pinned;
        let second_account = &second;
        let second_thread = scope.spawn(move || {
            second_barrier.wait();
            write_and_read(
                second_client,
                second_probe,
                second_account,
                "second.txt",
                b'b',
            )
        });
        barrier.wait();
        first_thread.join().unwrap().unwrap();
        second_thread.join().unwrap().unwrap();
    });
    assert!(server
        .identity(first.credential.uid.as_bytes())
        .unwrap()
        .is_some());
    assert!(server
        .identity(second.credential.uid.as_bytes())
        .unwrap()
        .is_some());
    let metrics = server.metrics();
    assert!(metrics.requests_started >= 20);
    assert_eq!(metrics.requests_started, metrics.responses_completed);
    server.shutdown().unwrap();
}

#[test]
fn conflicting_same_directory_writes_have_one_winner_and_no_busy_error() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let first_client = TestClient::new(&environment, "conflict-first").unwrap();
    let first_probe = first_client.probe_and_pin().unwrap();
    let created = first_client
        .create_account(
            &first_probe.pinned,
            &TestAccountSpec::new("conflictuser", 0x93),
        )
        .unwrap();
    // This client first pins after signup advanced the Merkle root. Its
    // imported credential therefore exercises authenticated historical-root
    // discovery for the eldest link rather than sharing the creator's anchor.
    let second_client = TestClient::new(&environment, "conflict-second").unwrap();
    let second_probe = second_client.probe_and_pin().unwrap();
    let second_authenticated = second_client
        .foks()
        .authenticate_and_pin(&second_probe.pinned, &created.credential)
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let (first_result, second_result) = std::thread::scope(|scope| {
        let created_ref = &created;
        let first_barrier = Arc::clone(&barrier);
        let first_client = &first_client;
        let first_probe = &first_probe.pinned;
        let created = created_ref;
        let first = scope.spawn(move || {
            let mut protected = first_client.open_protected_store().unwrap();
            let mut session = first_client
                .foks()
                .user_kv_write_session(
                    first_probe,
                    &created.credential,
                    &created.authenticated.verified,
                    &created.authenticated.puks,
                    first_client.soft_state_path(),
                    &mut protected,
                )
                .unwrap();
            first_barrier.wait();
            session.put_file(
                created.kv_projection[0].root_directory_id,
                "winner.txt",
                &mut Cursor::new(b"first"),
                options(),
            )
        });
        let second_barrier = Arc::clone(&barrier);
        let second_client = &second_client;
        let second_probe = &second_probe.pinned;
        let created = created_ref;
        let second_authenticated = &second_authenticated;
        let second = scope.spawn(move || {
            let mut protected = second_client.open_protected_store().unwrap();
            let mut session = second_client
                .foks()
                .user_kv_write_session(
                    second_probe,
                    &created.credential,
                    &second_authenticated.verified,
                    &second_authenticated.puks,
                    second_client.soft_state_path(),
                    &mut protected,
                )
                .unwrap();
            second_barrier.wait();
            session.put_file(
                created.kv_projection[0].root_directory_id,
                "winner.txt",
                &mut Cursor::new(b"second"),
                options(),
            )
        });
        barrier.wait();
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(first_result.is_ok(), second_result.is_ok());
    let failure = if let Err(error) = first_result {
        error
    } else {
        second_result.unwrap_err()
    };
    assert!(!failure.to_string().to_ascii_lowercase().contains("busy"));
    let reconstructed = TestClient::new(&environment, "conflict-first").unwrap();
    let authenticated = reconstructed
        .foks()
        .authenticate_and_pin(&first_probe.pinned, &created.credential)
        .unwrap();
    let tree = reconstructed
        .foks()
        .sync_user_kv(
            &first_probe.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree[0].entries.len(), 1);
    assert!(matches!(
        tree[0].entries[0].content.as_deref(),
        Some(b"first") | Some(b"second")
    ));
    server.shutdown().unwrap();
}

#[test]
fn full_writer_queue_returns_rate_limit_and_recovers_after_drain() {
    let environment = TestEnvironment::with_profile(TestProfile::QueuePressure).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "queue-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let created = client
        .create_account(&probe.pinned, &TestAccountSpec::new("queueuser", 0x94))
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
    let pressure = server.saturate_writer_queue().unwrap();
    assert_eq!(pressure.metrics().pending, 2);
    let started = std::time::Instant::now();
    let error = session
        .acquire_lock(root, [0x44; 16], Duration::from_secs(30))
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(matches!(
        error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1012, .. })
    ));
    let metrics = pressure.drain().unwrap();
    assert_eq!(metrics.pending, 0);
    assert!(metrics.rejected >= 1);
    let token = session
        .acquire_lock(root, [0x44; 16], Duration::from_secs(30))
        .unwrap();
    session.release_lock(root, [0x44; 16], token).unwrap();
    server.shutdown().unwrap();
}

#[test]
fn bounded_mixed_workload_soak_preserves_progress_across_restarts() {
    const REPRODUCTION_SEED: u64 = 0x5eed_cafe;
    let started = std::time::Instant::now();
    let environment = TestEnvironment::new().unwrap();
    let mut server = environment.start_server().unwrap();
    let first_client = TestClient::new(&environment, "soak-first").unwrap();
    let second_client = TestClient::new(&environment, "soak-second").unwrap();
    let first_probe = first_client.probe_and_pin().unwrap();
    let second_probe = second_client.probe_and_pin().unwrap();
    let first = first_client
        .create_account(
            &first_probe.pinned,
            &TestAccountSpec::new("soakfirst", 0xb1),
        )
        .unwrap();
    let second = second_client
        .create_account(
            &second_probe.pinned,
            &TestAccountSpec::new("soaksecond", 0xb2),
        )
        .unwrap();
    drop(first_client);
    drop(second_client);

    let mut completed = 0_u64;
    for cycle in 0..4 {
        for (client_id, probe, account, byte) in [
            ("soak-first", &first_probe.pinned, &first, 0xc1),
            ("soak-second", &second_probe.pinned, &second, 0xc2),
        ] {
            let client = TestClient::new(&environment, client_id).unwrap();
            let authenticated = client
                .foks()
                .authenticate_and_pin(probe, &account.credential)
                .unwrap_or_else(|error| {
                    panic!("seed={REPRODUCTION_SEED:#x} cycle={cycle}: {error}")
                });
            let mut protected = client.open_protected_store().unwrap();
            let mut session = client
                .foks()
                .user_kv_write_session(
                    probe,
                    &account.credential,
                    &authenticated.verified,
                    &authenticated.puks,
                    client.soft_state_path(),
                    &mut protected,
                )
                .unwrap();
            session
                .put_file(
                    account.kv_projection[0].root_directory_id,
                    &format!("cycle-{cycle}.txt"),
                    &mut Cursor::new(vec![byte; 256 + cycle]),
                    options(),
                )
                .unwrap();
            assert_eq!(session.sync().unwrap()[0].entries.len(), cycle + 1);
            completed += 1;
        }
        assert!(server.metrics().responses_completed > 0);
        let storage = server.storage_report().unwrap();
        assert!(storage.database_bytes + storage.wal_bytes < 64 * 1024 * 1024);
        server.shutdown().unwrap();
        server = environment.start_server().unwrap();
    }
    assert_eq!(completed, 8);
    assert!(started.elapsed() < Duration::from_secs(30));
    server.shutdown().unwrap();
}

fn write_and_read(
    client: &TestClient,
    host: &foks_client::PinnedHost,
    account: &foks_client::CreatedSoftwareAccount,
    name: &str,
    byte: u8,
) -> foks_client::Result<()> {
    let root = account.kv_projection[0].root_directory_id;
    let mut protected = client
        .open_protected_store()
        .map_err(|error| foks_client::Error::ProtectedMaterial(error.to_string()))?;
    let mut session = client.foks().user_kv_write_session(
        host,
        &account.credential,
        &account.authenticated.verified,
        &account.authenticated.puks,
        client.soft_state_path(),
        &mut protected,
    )?;
    session.put_file(root, name, &mut Cursor::new(vec![byte; 64]), options())?;
    for _ in 0..8 {
        let tree = session.sync()?;
        assert_eq!(tree[0].entries.len(), 1);
    }
    Ok(())
}

fn software_account_request(username: &str) -> SoftwareAccountRequest {
    SoftwareAccountRequest {
        username_utf8: username.to_owned(),
        device_name: format!("{username} device"),
        invite_code: InviteCode::Empty,
        email: format!("{username}@example.test"),
        passphrase: None,
    }
}

fn signup_frame(
    prepared: &foks_client::PreparedSoftwareAccount,
    request: &SoftwareAccountRequest,
    self_token: [u8; 17],
) -> Vec<u8> {
    foks_rpc::encode_signup_request_at(
        &SoftwareSignupArgument {
            username_utf8: &prepared.username_utf8,
            reservation: &prepared.reservation,
            link: &prepared.eldest.link,
            puk_box: &prepared.puk_box,
            username_commitment_key: prepared.username_commitment_key,
            device_name: &prepared.device_name,
            next_tree_location: prepared.next_tree_location,
            invite_code: &request.invite_code,
            email: request.email.as_bytes(),
            subchain_tree_location: prepared.subchain_tree_location,
            self_token,
            puk_hepk: &prepared.eldest.puk.hepk,
            device_hepk: &prepared.eldest.device.hepk,
            passphrase: prepared.passphrase.as_ref(),
        },
        1,
    )
    .unwrap()
}

fn submit_signup(environment: &TestEnvironment, host: &foks_client::PinnedHost, signup: &[u8]) {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in host.tls_ca_certificates() {
        roots
            .add(CertificateDer::from(certificate.clone()))
            .unwrap();
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp =
        std::net::TcpStream::connect(environment.addresses().unwrap().public_services).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    tls.write_all(&foks_rpc::encode_registration_select_vhost_request(host.host_id()).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut tls, 8 * 1024 * 1024, 0).unwrap();
    tls.write_all(signup).unwrap();
    foks_rpc::read_void_response(&mut tls, 8 * 1024 * 1024, 1).unwrap();
}
