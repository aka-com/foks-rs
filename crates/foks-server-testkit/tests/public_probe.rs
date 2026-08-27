use std::time::Duration;
use std::{io::Write as _, net::TcpStream, sync::Arc};

use foks_client::{FoksClient, ProbeTarget};
use foks_client_db::Acceptance;
use foks_server_testkit::IsolatedTestServer;

#[test]
fn public_client_verifies_and_pins_the_real_tls_probe() {
    let server = IsolatedTestServer::start().unwrap();
    let mut client = FoksClient::with_roots(server.probe_roots());
    client.set_timeout(Duration::from_secs(5));
    let target =
        ProbeTarget::parse(&format!("localhost:{}", server.addresses().probe.port())).unwrap();
    let database = server.root().join("client-hard-state.sqlite");
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let client_tls = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(server.probe_roots())
            .with_no_client_auth(),
    );
    let tcp = TcpStream::connect(server.addresses().probe).unwrap();
    let connection = rustls::ClientConnection::new(
        client_tls,
        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    if let Err(error) =
        tls.write_all(&foks_rpc::encode_probe_request("localhost", 0, None).unwrap())
    {
        std::thread::sleep(Duration::from_millis(100));
        panic!("probe TLS write failed: {error}");
    }
    tls.flush().unwrap();
    let content = foks_rpc::read_frame(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH).unwrap();
    foks_rpc::decode_probe_response(&content, 0).unwrap();

    let first = client.probe_and_pin(&target, &database).unwrap();
    let second = client.probe_and_pin(&target, &database).unwrap();
    assert_eq!(first.acceptance, Acceptance::Inserted);
    assert_eq!(second.acceptance, Acceptance::Unchanged);
    assert_eq!(
        first.verified.snapshot.host_id(),
        second.verified.snapshot.host_id()
    );
    assert_eq!(
        first.verified.public_zone.services.probe,
        format!("localhost:{}", server.addresses().probe.port())
    );
    assert_eq!(
        first.verified.public_zone.services.registration,
        format!("localhost:{}", server.addresses().public_services.port())
    );
    assert_eq!(
        first.verified.public_zone.services.user,
        format!("localhost:{}", server.addresses().authenticated.port())
    );
    let (acceptance, advanced) = client.advance_merkle_root(&first.pinned).unwrap();
    assert_eq!(acceptance, Acceptance::Unchanged);
    assert_eq!(advanced.root().epoch, 0);
    server.shutdown().unwrap();
}
