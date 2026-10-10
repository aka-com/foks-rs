use std::io::Write as _;
use std::sync::Arc;

use foks_server_testkit::{TestProfile, WAITLIST_TEST_ENTRIES};
use foks_snowpack::{decode, Value};
use rustls::pki_types::{CertificateDer, ServerName};

use crate::support::Fixture;

const STATUS_RATE_LIMITED: u64 = 1012;
const STATUS_UNSUPPORTED: u64 = 1020;

#[test]
pub(crate) fn go_client_waitlist_works() {
    let fixture = Fixture::start_with_profile("go-client-waitlist", TestProfile::Waitlist);
    let waitlist_id = join(&fixture, b"go-client@example.com").unwrap();
    assert_eq!(waitlist_id.len(), 13);
    assert_eq!(waitlist_id[0], 1);
    // A repeated address, under ASCII case folding, returns the stored identifier.
    assert_eq!(
        join(&fixture, b"Go-Client@Example.com").unwrap(),
        waitlist_id
    );
    assert_eq!(entry_count(&fixture), 1);
}

#[test]
fn waitlist_is_unsupported_unless_enabled() {
    let fixture = Fixture::start("waitlist-disabled");
    assert!(matches!(
        join(&fixture, b"disabled@example.com"),
        Err(foks_rpc::Error::RemoteStatus {
            code: STATUS_UNSUPPORTED,
            ..
        })
    ));
    assert_eq!(entry_count(&fixture), 0);
}

#[test]
fn waitlist_cap_refuses_joins_and_bounds_the_table() {
    let fixture = Fixture::start_with_profile("waitlist-cap", TestProfile::Waitlist);
    for index in 0..WAITLIST_TEST_ENTRIES {
        join(&fixture, format!("user-{index}@example.com").as_bytes()).unwrap();
    }
    // A listed address gets the same answer as a new one at the cap.
    for email in [&b"one-more@example.com"[..], b"user-0@example.com"] {
        assert!(matches!(
            join(&fixture, email),
            Err(foks_rpc::Error::RemoteStatus {
                code: STATUS_RATE_LIMITED,
                ..
            })
        ));
    }
    assert_eq!(entry_count(&fixture), WAITLIST_TEST_ENTRIES as usize);
}

fn join(fixture: &Fixture, email: &[u8]) -> foks_rpc::Result<Vec<u8>> {
    let mut tls = public_stream(fixture);
    tls.write_all(&foks_rpc::encode_join_waitlist_request_at(email, 0).unwrap())
        .unwrap();
    let response = foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0)?;
    let Value::Binary(waitlist_id) = decode(&response).unwrap() else {
        panic!("waitlist response was not a binary identifier");
    };
    Ok(waitlist_id)
}

fn entry_count(fixture: &Fixture) -> usize {
    foks_server_db::ReadDatabase::open(
        fixture.environment.database_path(),
        foks_server_db::Config::default(),
    )
    .unwrap()
    .waitlist_entries()
    .unwrap()
    .len()
}

fn public_stream(
    fixture: &Fixture,
) -> rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in fixture.host().tls_ca_certificates() {
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
    let tcp = std::net::TcpStream::connect(fixture.server.addresses().public_services).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(connection, tcp)
}
