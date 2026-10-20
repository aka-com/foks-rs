use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use foks_proto::{
    KexActorType, KexReceiveArgument, KexSendArgument, KexWrapperMessage, SecretBox, SecretSeed,
};
use foks_rpc::{Error, DEFAULT_MAX_FRAME_LENGTH};
use foks_server_testkit::{InProcessServer, TestEnvironment, TestProfile};

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

fn connect(server: &InProcessServer) -> Tls {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(server.service_roots())
    .with_no_client_auth();
    let tcp = TcpStream::connect(server.addresses().public_services).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(connection, tcp)
}

fn packet(tag: u8) -> (KexSendArgument, KexReceiveArgument) {
    let seed = SecretSeed::new([0x31; 32]);
    let message = KexWrapperMessage {
        session_id: [tag; 32],
        sender: foks_crypto::derive_device_public(&seed).unwrap().id,
        sequence: 2,
        payload: SecretBox {
            nonce: [0x61; 16],
            ciphertext: vec![0x71; 32],
        },
    };
    let signature = foks_crypto::sign_kex_wrapper(&seed, &message).unwrap();
    let receive = KexReceiveArgument {
        session_id: message.session_id,
        receiver: foks_crypto::derive_device_public(&SecretSeed::new([0x41; 32]))
            .unwrap()
            .id,
        sequence: message.sequence,
        poll_wait_milliseconds: 60 * 60 * 1000,
        actor: KexActorType::Provisionee,
    };
    (
        KexSendArgument {
            message,
            signature,
            actor: KexActorType::Provisioner,
        },
        receive,
    )
}

fn send(tls: &mut Tls, request: &[u8]) {
    tls.write_all(request).unwrap();
    tls.flush().unwrap();
}

fn start_poll(server: &InProcessServer, argument: &KexReceiveArgument) -> Tls {
    let mut tls = connect(server);
    send(
        &mut tls,
        &foks_rpc::encode_kex_receive_request(argument).unwrap(),
    );
    tls
}

fn wait_metrics(
    server: &InProcessServer,
    predicate: impl Fn(foks_server::KexMetricsSnapshot) -> bool,
) -> foks_server::KexMetricsSnapshot {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let value = server.metrics().kex;
        if predicate(value) {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "KEX metrics failed to settle: {value:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn assert_closed(tls: &mut Tls) {
    let mut byte = [0];
    match tls.read(&mut byte) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
            ) => {}
        result => panic!("expected connection close, not timeout/data: {result:?}"),
    }
}

fn completed(value: foks_server::KexMetricsSnapshot) -> u64 {
    value.delivered + value.timed_out + value.disconnected + value.shutdown + value.failed
}

#[test]
fn saturated_anonymous_polls_preserve_public_requests_and_signed_pairing_progress() {
    let environment = TestEnvironment::with_profile(TestProfile::KexPollCapacity).unwrap();
    let server = environment.start_server().unwrap();
    // Four active public connections imply two admitted KEX receives despite
    // the default configured receive ceiling of32.
    for cycle in 0..3u8 {
        let (offer, receive) = packet(10 + cycle * 3);
        let mut legitimate = start_poll(&server, &receive);
        let (_, anonymous) = packet(11 + cycle * 3);
        let abandoned = start_poll(&server, &anonymous);
        wait_metrics(&server, |m| m.active == 2);
        let mut excess = connect(&server);
        let started = Instant::now();
        send(
            &mut excess,
            &foks_rpc::encode_call(
                foks_rpc::KEX_PROTOCOL_ID,
                foks_rpc::KEX_RECEIVE_METHOD_POSITION,
                &receive.encoded().unwrap(),
                73,
            )
            .unwrap(),
        );
        assert!(matches!(
            foks_rpc::read_bare_response(&mut excess, DEFAULT_MAX_FRAME_LENGTH, 73),
            Err(Error::RemoteStatus { code: 1012, .. })
        ));
        assert_closed(&mut excess);
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(excess);
        // This is the same public listener, not the separate probe listener.
        let mut ordinary = connect(&server);
        let probe = foks_proto::ProbeResponse::decode(&server.probe_response()).unwrap();
        let host = probe.hostchain[0].decode_change().unwrap().host;
        send(
            &mut ordinary,
            &foks_rpc::encode_get_current_merkle_root_request(&host, 0).unwrap(),
        );
        let root = foks_rpc::read_response(&mut ordinary, DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
        foks_proto::MerkleRoot::decode(&root).unwrap();
        drop(ordinary);
        let mut sender = connect(&server);
        send(
            &mut sender,
            &foks_rpc::encode_kex_send_request(&offer).unwrap(),
        );
        foks_rpc::read_bare_void_response(&mut sender, DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
        let response =
            foks_rpc::read_bare_response(&mut legitimate, DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
        assert_eq!(KexWrapperMessage::decode(&response).unwrap(), offer.message);
        drop(sender);
        drop(legitimate);
        wait_metrics(&server, |m| m.active == 1);
        abandoned.sock.shutdown(Shutdown::Both).unwrap();
        drop(abandoned);
        let before = wait_metrics(&server, |m| m.active == 0);
        assert_eq!(before.admitted, completed(before));
        // Reuse immediately after disconnect, rather than waiting for the hour poll.
        let (_, again) = packet(12 + cycle * 3);
        let reused = start_poll(&server, &again);
        wait_metrics(&server, |m| m.active == 1);
        reused.sock.shutdown(Shutdown::Both).unwrap();
        drop(reused);
        let after = wait_metrics(&server, |m| m.active == 0);
        assert_eq!(after.admitted, completed(after));
        assert_eq!(after.delivered, u64::from(cycle) + 1);
        assert_eq!(after.disconnected, 2 * (u64::from(cycle) + 1));
        assert_eq!(after.rejected, u64::from(cycle) + 1);
    }
    let mut metrics = TcpStream::connect(server.management_address()).unwrap();
    metrics
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    metrics
        .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut output = String::new();
    metrics.read_to_string(&mut output).unwrap();
    for expected in [
        "foks_kex_active_polls 0",
        "foks_kex_admitted_total 9",
        "foks_kex_rejected_total 3",
        "foks_kex_completed_total{reason=\"delivered\"} 3",
        "foks_kex_completed_total{reason=\"disconnected\"} 6",
    ] {
        assert!(
            output.lines().any(|line| line == expected),
            "missing {expected}"
        );
    }
    server.shutdown().unwrap();
}

#[test]
fn timeout_and_malformed_arguments_release_admission_and_shutdown_cancels_waits() {
    let environment = TestEnvironment::with_profile(TestProfile::KexPollCapacity).unwrap();
    let server = environment.start_server().unwrap();
    let (_, mut receive) = packet(80);
    receive.poll_wait_milliseconds = 40;
    let mut timed = start_poll(&server, &receive);
    assert!(matches!(
        foks_rpc::read_bare_response(&mut timed, DEFAULT_MAX_FRAME_LENGTH, 0),
        Err(Error::RemoteStatus { code: 1014, .. })
    ));
    drop(timed);
    wait_metrics(&server, |m| m.active == 0 && m.timed_out == 1);
    let mut malformed = connect(&server);
    send(
        &mut malformed,
        &foks_rpc::encode_call(
            foks_rpc::KEX_PROTOCOL_ID,
            foks_rpc::KEX_RECEIVE_METHOD_POSITION,
            &[0x91, 0x00],
            9,
        )
        .unwrap(),
    );
    assert!(matches!(
        foks_rpc::read_bare_response(&mut malformed, DEFAULT_MAX_FRAME_LENGTH, 9),
        Err(Error::RemoteStatus { code: 1030, .. })
    ));
    drop(malformed);
    let settled = wait_metrics(&server, |m| m.active == 0 && m.failed == 1);
    assert_eq!(settled.admitted, 2);
    assert_eq!(settled.admitted, completed(settled));
    receive.poll_wait_milliseconds = 60 * 60 * 1000;
    let mut first = start_poll(&server, &receive);
    let mut second = start_poll(&server, &receive);
    wait_metrics(&server, |m| m.active == 2);
    let started = Instant::now();
    server.shutdown().unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_closed(&mut first);
    assert_closed(&mut second);
}

#[test]
fn complete_software_pairing_succeeds_after_anonymous_budget_is_reclaimed() {
    use foks_client::KexProvisionOffer;
    use foks_proto::Role;
    use foks_server_testkit::{TestAccountSpec, TestClient};

    let environment = TestEnvironment::with_profile(TestProfile::KexPollCapacity).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "bounded-provisioner").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let created = client
        .create_account(&host, &TestAccountSpec::new("bounded-kex-user", 0x31))
        .unwrap();
    let (_, anonymous) = packet(95);
    let first = start_poll(&server, &anonymous);
    let second = start_poll(&server, &anonymous);
    wait_metrics(&server, |m| m.active == 2);
    first.sock.shutdown(Shutdown::Both).unwrap();
    second.sock.shutdown(Shutdown::Both).unwrap();
    drop(first);
    drop(second);
    wait_metrics(&server, |m| m.active == 0 && m.disconnected == 2);

    let provisionee = TestClient::new(&environment, "bounded-provisionee").unwrap();
    let provisionee_host = provisionee.probe_and_pin().unwrap().pinned;
    let offer = KexProvisionOffer::generate(Role::OWNER).unwrap();
    let phrase = offer.phrase().expose_joined();
    client
        .foks()
        .publish_kex_provision_offer(&host, &created.credential, &offer)
        .unwrap();
    let provisioner = client.foks().clone();
    let expected_user: [u8; 33] = created.credential.uid.as_bytes().try_into().unwrap();
    let mut protected = client.open_protected_store().unwrap();
    let finish = std::thread::spawn(move || {
        provisioner.finish_kex_provisioning(&host, &created.credential, &offer, &mut protected)
    });
    let accepted = provisionee
        .foks()
        .accept_kex_provisioning_for_user(
            &provisionee_host,
            &phrase,
            "paired laptop",
            2,
            SecretSeed::new([0x41; 32]),
            Some(&expected_user),
        )
        .unwrap();
    let finished = finish.join().unwrap().unwrap();
    assert_eq!(accepted.authenticated.verified.chain_seqno(), 2);
    assert_eq!(accepted.authenticated.verified.devices().len(), 2);
    assert_eq!(
        finished.device,
        foks_crypto::derive_device_public(&SecretSeed::new([0x41; 32])).unwrap()
    );
    let settled = wait_metrics(&server, |m| m.active == 0);
    assert_eq!(settled.admitted, completed(settled));
    assert!(settled.delivered > 0);
    assert_eq!(settled.rejected, 0);
    server.shutdown().unwrap();
}
