use std::time::Duration;

use foks_client::{FoksClient, ProbeTarget};
use foks_proto::{ProbeResponse, Signature};
use foks_server_testkit::{TestClient, TestEnvironment};

#[test]
fn a_different_valid_server_cannot_replace_an_existing_pin() {
    let trusted_environment = TestEnvironment::new().unwrap();
    let trusted_server = trusted_environment.start_server().unwrap();
    let trusted_client = TestClient::new(&trusted_environment, "trust-client").unwrap();
    let trusted = trusted_client.probe_and_pin().unwrap();

    let other_environment = TestEnvironment::new().unwrap();
    let other_server = other_environment.start_server().unwrap();
    let mut roots = trusted_environment.probe_roots();
    roots.extend(other_environment.probe_roots().roots);
    let mut client = FoksClient::with_roots(roots);
    client.set_timeout(Duration::from_secs(5));
    let target = ProbeTarget::parse(&format!(
        "localhost:{}",
        other_server.addresses().probe.port()
    ))
    .unwrap();
    assert!(matches!(
        client.probe_and_pin(&target, trusted_client.hard_state_path()),
        Err(foks_client::Error::Database(
            foks_client_db::Error::HostIdentityChanged { .. }
        ))
    ));
    let retained = trusted_client.pinned_host().unwrap();
    assert_eq!(retained.host_id(), trusted.pinned.host_id());
    trusted_server.shutdown().unwrap();
    other_server.shutdown().unwrap();
}

#[test]
fn wrong_probe_root_or_hostname_never_creates_a_pin() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();

    let missing_root_path = environment
        .client_path("wrong-root", "hard.sqlite")
        .unwrap();
    let mut missing_root = FoksClient::with_roots(rustls::RootCertStore::empty());
    missing_root.set_timeout(Duration::from_secs(5));
    let localhost =
        ProbeTarget::parse(&format!("localhost:{}", server.addresses().probe.port())).unwrap();
    assert!(missing_root
        .probe_and_pin(&localhost, &missing_root_path)
        .is_err());
    assert!(!missing_root_path.exists());

    let wrong_name_path = environment
        .client_path("wrong-name", "hard.sqlite")
        .unwrap();
    let mut wrong_name = FoksClient::with_roots(environment.probe_roots());
    wrong_name.set_timeout(Duration::from_secs(5));
    let ip_name =
        ProbeTarget::parse(&format!("127.0.0.1:{}", server.addresses().probe.port())).unwrap();
    assert!(wrong_name
        .probe_and_pin(&ip_name, &wrong_name_path)
        .is_err());
    assert!(!wrong_name_path.exists());
    server.shutdown().unwrap();
}

#[test]
fn a_tampered_public_zone_cannot_modify_an_existing_pin() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "tampered-zone").unwrap();
    let trusted = client.probe_and_pin().unwrap();
    let mut probe = ProbeResponse::decode(&server.probe_response()).unwrap();
    let Signature::Ed25519(signature) = &mut probe.public_zone.signature else {
        panic!("test bootstrap did not use Ed25519")
    };
    signature[0] ^= 0x80;
    let tampered = probe.encoded().unwrap();
    server.shutdown().unwrap();

    let override_server = environment.start_probe_override(tampered).unwrap();
    assert!(matches!(
        client.probe_and_pin(),
        Err(foks_client::Error::Verify(_))
    ));
    assert_eq!(
        client.pinned_host().unwrap().host_id(),
        trusted.pinned.host_id()
    );
    override_server.shutdown().unwrap();
}
