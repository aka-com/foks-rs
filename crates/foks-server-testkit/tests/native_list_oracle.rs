#![cfg(feature = "celld-proof")]
//! The qualification oracle must preserve the real public List path and status.
use foks_proto::{KvListResponse, Role};
use foks_rpc::{KvAuth, KvListCursor, RpcStatus, DEFAULT_MAX_FRAME_LENGTH};
use foks_server::{hosted_personal, hosted_personal::QualificationError};
use foks_server_db::Clock;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use std::{
    io::Cursor,
    io::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

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
    // Both native direct and request-header forms use the same authority parser.
    let mut wrapped = fields.clone();
    wrapped[0] = foks_snowpack::Value::Array(vec![wrapped[0].clone(), foks_snowpack::Value::Null]);
    assert!(compare(
        foks_rpc::encode_call(
            call.protocol_id(),
            call.method_position(),
            &foks_snowpack::encode(&foks_snowpack::Value::Array(wrapped)).unwrap(),
            sequence
        )
        .unwrap()
    )
    .is_ok());
    sequence += 1;
    for wrapped in [false, true] {
        let team = foks_snowpack::Value::Array(vec![
            foks_snowpack::Value::Unsigned(1),
            foks_snowpack::Value::Variant(Some((
                b"1".to_vec(),
                Box::new(foks_snowpack::Value::Binary(vec![0x57; 16])),
            ))),
        ]);
        let mut changed = fields.clone();
        changed[0] = if wrapped {
            foks_snowpack::Value::Array(vec![team, foks_snowpack::Value::Null])
        } else {
            team
        };
        // A well-shaped unknown team token reaches team SQL; it is not personal.
        assert_eq!(
            compare(
                foks_rpc::encode_call(
                    call.protocol_id(),
                    call.method_position(),
                    &foks_snowpack::encode(&foks_snowpack::Value::Array(changed)).unwrap(),
                    sequence
                )
                .unwrap()
            ),
            Err(RpcStatus::Expired)
        );
        sequence += 1;
    }
    let mut malformed = Vec::new();
    for (token, message) in [
        (foks_snowpack::Value::Null, "team KV token is not binary"),
        (
            foks_snowpack::Value::Binary(vec![0; 15]),
            "team KV token has the wrong width",
        ),
        (
            foks_snowpack::Value::Binary(vec![0; 17]),
            "team KV token has the wrong width",
        ),
    ] {
        for wrapped in [false, true] {
            let auth = foks_snowpack::Value::Array(vec![
                foks_snowpack::Value::Unsigned(1),
                foks_snowpack::Value::Variant(Some((b"1".to_vec(), Box::new(token.clone())))),
            ]);
            let mut changed = fields.clone();
            changed[0] = if wrapped {
                foks_snowpack::Value::Array(vec![auth, foks_snowpack::Value::Null])
            } else {
                auth
            };
            changed[2] = foks_snowpack::Value::Null;
            malformed.push((changed, message));
        }
    }
    let mut changed = fields.clone();
    changed[0] = foks_snowpack::Value::Null;
    changed[2] = foks_snowpack::Value::Null;
    malformed.push((changed, "KV authentication is not a struct"));

    let mut changed = fields.clone();
    changed[2] = foks_snowpack::Value::Null;
    malformed.push((changed, "KV pagination is not a struct"));
    let mut changed = fields.clone();
    changed[1] = foks_snowpack::Value::Null;
    changed[2] = foks_snowpack::Value::Null;
    malformed.push((changed, "KV directory ID is not binary"));
    let mut changed = fields.clone();
    changed[1] = foks_snowpack::Value::Binary(vec![0; 15]);
    malformed.push((changed, "KV directory ID has the wrong width"));
    let mut changed = fields.clone();
    changed.push(foks_snowpack::Value::Null);
    malformed.push((changed, "KV argument has the wrong shape"));
    for (cursor, count, load, message) in [
        (
            foks_snowpack::Value::Null,
            foks_snowpack::Value::Negative(-1),
            foks_snowpack::Value::Bool(false),
            "KV pagination has the wrong shape",
        ),
        (
            foks_snowpack::Value::Null,
            foks_snowpack::Value::Unsigned(1),
            foks_snowpack::Value::Bool(false),
            "KV list cursor is not a struct",
        ),
        (
            foks_snowpack::Value::Array(vec![
                foks_snowpack::Value::Unsigned(0),
                foks_snowpack::Value::Null,
            ]),
            foks_snowpack::Value::Unsigned(1),
            foks_snowpack::Value::Bool(false),
            "unsupported KV list cursor",
        ),
    ] {
        let mut changed = fields.clone();
        changed[2] = foks_snowpack::Value::Array(vec![cursor, count, load]);
        malformed.push((changed, message));
    }
    // Authority parsing still runs first; the extracted field decoder must not
    // turn this into the competing pagination error.
    fields[0] = foks_snowpack::Value::Array(vec![
        foks_snowpack::Value::Unsigned(0),
        foks_snowpack::Value::Null,
    ]);
    fields[2] = foks_snowpack::Value::Null;
    malformed.push((fields, "unsupported KV authentication"));
    for (fields, message) in malformed {
        assert_eq!(
            compare(
                foks_rpc::encode_call(
                    call.protocol_id(),
                    call.method_position(),
                    &foks_snowpack::encode(&foks_snowpack::Value::Array(fields)).unwrap(),
                    sequence,
                )
                .unwrap()
            ),
            Err(RpcStatus::BadArguments(message.into()))
        );
        sequence += 1;
    }
    // The first clock sample authenticates the certificate in the oracle. The
    // second is the real resolve_authority boundary; failures there must beat
    // variant/token errors but not earlier shape/decode failures.
    struct FailingAuthorityClock {
        calls: AtomicUsize,
        now: u64,
    }
    impl Clock for FailingAuthorityClock {
        fn now_micros(&self) -> foks_server_db::Result<u64> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(self.now)
            } else {
                Err(foks_server_db::Error::Invalid(
                    "injected authority clock failure",
                ))
            }
        }
    }
    let bad = |message: &str| RpcStatus::BadArguments(message.to_owned());
    let post_clock = [
        foks_snowpack::Value::Array(vec![
            foks_snowpack::Value::Unsigned(0),
            foks_snowpack::Value::Null,
        ]),
        foks_snowpack::Value::Array(vec![
            foks_snowpack::Value::Unsigned(1),
            foks_snowpack::Value::Variant(Some((
                b"1".to_vec(),
                Box::new(foks_snowpack::Value::Null),
            ))),
        ]),
        foks_snowpack::Value::Array(vec![
            foks_snowpack::Value::Unsigned(1),
            foks_snowpack::Value::Variant(Some((
                b"1".to_vec(),
                Box::new(foks_snowpack::Value::Binary(vec![0; 15])),
            ))),
        ]),
    ];
    let mut clock_cases = vec![
        (
            foks_snowpack::Value::Null,
            1,
            bad("KV argument is not a struct"),
        ),
        (
            foks_snowpack::Value::Array(vec![foks_snowpack::Value::Null]),
            1,
            bad("KV authentication is not a struct"),
        ),
    ];
    for auth in post_clock {
        for wrapped in [false, true] {
            let auth = if wrapped {
                foks_snowpack::Value::Array(vec![auth.clone(), foks_snowpack::Value::Null])
            } else {
                auth.clone()
            };
            clock_cases.push((
                foks_snowpack::Value::Array(vec![auth, foks_snowpack::Value::Null]),
                2,
                RpcStatus::TransactionRetry,
            ));
        }
    }
    for (argument, count, expected) in clock_cases {
        let observed = Arc::new(FailingAuthorityClock {
            calls: AtomicUsize::new(0),
            now: environment.advance_clock(0),
        });
        let clock: Arc<dyn Clock> = observed.clone();
        assert_eq!(
            hosted_personal::native_list(
                &certificate,
                &reader,
                &clock,
                &foks_snowpack::encode(&argument).unwrap()
            )
            .unwrap(),
            Err(expected)
        );
        assert_eq!(observed.calls.load(Ordering::SeqCst), count);
    }
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
