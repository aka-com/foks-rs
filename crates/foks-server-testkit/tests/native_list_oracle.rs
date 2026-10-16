#![cfg(feature = "celld-proof")]
//! The qualification oracle must preserve the real public List path and status.
use foks_proto::{KvListResponse, Role};
use foks_rpc::{KvAuth, KvListCursor, RpcStatus, DEFAULT_MAX_FRAME_LENGTH};
use foks_server::{hosted_personal, hosted_personal::QualificationError};
use foks_server_db::Clock;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use std::{io::Cursor, io::Write, sync::Arc, time::Duration};

struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        Ok(self.0)
    }
}

#[test]
fn native_list_oracle_matches_raw_authenticated_rpc_bytes_and_errors() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "list-oracle").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("listoracle", 0x75))
        .unwrap();
    let root = account.kv_projection[0].root_directory_id;
    let mut protected = client.open_protected_store().unwrap();
    client
        .foks()
        .user_kv_write_session(
            &host,
            &account.credential,
            &account.authenticated.verified,
            &account.authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap()
        .put_file(
            root,
            "entry.txt",
            &mut Cursor::new(b"native List oracle"),
            foks_client::KvWriteOptions {
                read_role: Role::OWNER,
                write_role: Role::OWNER,
                overwrite: false,
                expected_version: None,
            },
        )
        .unwrap();

    let mut roots = rustls::RootCertStore::empty();
    for certificate in host.tls_ca_certificates() {
        roots
            .add(CertificateDer::from(certificate.clone()))
            .unwrap();
    }
    let key = foks_crypto::device_signing_key_pkcs8(&account.credential.seed).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_client_auth_cert(
        account
            .credential
            .certificate_chain
            .iter()
            .cloned()
            .map(CertificateDer::from)
            .collect(),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())),
    )
    .unwrap();
    let tcp = std::net::TcpStream::connect_timeout(
        &server.addresses().authenticated,
        Duration::from_secs(5),
    )
    .unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    tls.write_all(&foks_rpc::encode_kv_select_vhost_request(host.host_id()).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut tls, DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();

    let clock: Arc<dyn Clock> = Arc::new(FixedClock(environment.advance_clock(0)));
    let reader = environment.read_database().unwrap();
    let certificate = CertificateDer::from(account.credential.certificate_chain[0].as_slice());
    let mut sequence = 1;
    let mut compare = |request: Vec<u8>| {
        let call =
            foks_rpc::read_call(&mut Cursor::new(&request), DEFAULT_MAX_FRAME_LENGTH).unwrap();
        let result =
            hosted_personal::native_list(&certificate, &reader, &clock, call.argument()).unwrap();
        let expected = match &result {
            Ok(bytes) => foks_rpc::encode_success_response_at(bytes, call.sequence()),
            Err(status) => foks_rpc::encode_status_response_at(status, call.sequence()),
        }
        .unwrap();
        tls.write_all(&request).unwrap();
        let actual = foks_rpc::read_frame(&mut tls, DEFAULT_MAX_FRAME_LENGTH).unwrap();
        assert_eq!(
            actual,
            foks_rpc::read_frame(&mut Cursor::new(expected), DEFAULT_MAX_FRAME_LENGTH).unwrap(),
            "sequence {}",
            call.sequence()
        );
        result
    };

    let mut selected = None;
    for count in [0, 1, 4096, 4097] {
        for load_small in [false, true] {
            let bytes = compare(
                foks_rpc::encode_kv_list_request_at(
                    KvAuth::User,
                    &root,
                    KvListCursor::None,
                    count,
                    load_small,
                    sequence,
                )
                .unwrap(),
            )
            .unwrap();
            sequence += 1;
            let response = KvListResponse::decode(&bytes).unwrap();
            assert_eq!(response.entries.len(), 1);
            assert_eq!(response.final_page, count != 1);
            assert_eq!(response.extended.len(), usize::from(load_small));
            selected = Some(response.entries[0].clone());
        }
    }
    let selected = selected.unwrap();
    for (cursor, count) in [
        (KvListCursor::Mac(selected.name_mac), 0),
        (KvListCursor::Time(selected.creation_time), 1),
        (KvListCursor::Time(i64::MAX as u64), 0),
    ] {
        let bytes = compare(
            foks_rpc::encode_kv_list_request_at(KvAuth::User, &root, cursor, 4096, true, sequence)
                .unwrap(),
        )
        .unwrap();
        sequence += 1;
        let response = KvListResponse::decode(&bytes).unwrap();
        assert_eq!(response.entries.len(), count);
        assert!(response.final_page);
    }
    for time in [i64::MAX as u64 + 1, u64::MAX] {
        assert_eq!(
            compare(
                foks_rpc::encode_kv_list_request_at(
                    KvAuth::User,
                    &root,
                    KvListCursor::Time(time),
                    4096,
                    true,
                    sequence,
                )
                .unwrap(),
            ),
            Err(RpcStatus::TransactionRetry)
        );
        sequence += 1;
    }
    // Parent lookup precedes conversion of an otherwise well-shaped u64 time.
    assert_eq!(
        compare(
            foks_rpc::encode_kv_list_request_at(
                KvAuth::User,
                &[0xf1; 16],
                KvListCursor::Time(u64::MAX),
                4096,
                true,
                sequence,
            )
            .unwrap(),
        ),
        Err(RpcStatus::KvNoEnt)
    );
    sequence += 1;
    let valid = foks_rpc::encode_kv_list_request_at(
        KvAuth::User,
        &root,
        KvListCursor::None,
        1,
        false,
        sequence,
    )
    .unwrap();
    let call = foks_rpc::read_call(&mut Cursor::new(valid), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    let foks_snowpack::Value::Array(mut fields) = foks_snowpack::decode(call.argument()).unwrap()
    else {
        panic!("encoded List argument must be a struct")
    };
    fields[2] = foks_snowpack::Value::Null;
    assert!(matches!(
        compare(
            foks_rpc::encode_call(
                call.protocol_id(),
                call.method_position(),
                &foks_snowpack::encode(&foks_snowpack::Value::Array(fields)).unwrap(),
                sequence,
            )
            .unwrap()
        ),
        Err(RpcStatus::BadArguments(_))
    ));
    drop(tls);
    // Certificate failures are outer qualification errors, not invented List statuses.
    assert!(matches!(
        hosted_personal::native_list(
            &CertificateDer::from(&b"not DER"[..]),
            &reader,
            &clock,
            call.argument()
        ),
        Err(QualificationError::Certificate(_))
    ));
    server.shutdown().unwrap();
}
