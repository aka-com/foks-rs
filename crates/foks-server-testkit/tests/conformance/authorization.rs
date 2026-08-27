use foks_proto::{EntityId, SecretSeed, ENTITY_USER};
use foks_server_testkit::{TestAccountSpec, TestClient};

use crate::support::Fixture;

#[test]
pub(crate) fn authorization_and_unsupported_success() {
    let fixture = Fixture::start("authorization-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("authuser", 0x81))
        .unwrap();
    let other_client = TestClient::new(&fixture.environment, "other-auth-client").unwrap();
    let other_probe = other_client.probe_and_pin().unwrap();
    let other = other_client
        .create_account(
            &other_probe.pinned,
            &TestAccountSpec::new("otherauth", 0x91),
        )
        .unwrap();
    let refreshed = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &created.credential)
        .unwrap();
    assert_eq!(
        refreshed.merkle_acceptance,
        foks_client_db::Acceptance::Advanced
    );
    assert_eq!(refreshed.verified.username(), b"authuser");
    assert_eq!(refreshed.puks[0].seed.as_slice(), &[0x82; 32]);

    let foks_client::DeviceCredential {
        seed,
        certificate_chain,
        ..
    } = other.credential;
    let unbound = foks_client::DeviceCredential {
        uid: created.credential.uid.clone(),
        seed,
        certificate_chain,
    };
    let unbound_error = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &unbound)
        .unwrap_err();
    assert!(
        matches!(
            unbound_error,
            foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1013, .. })
        ),
        "unexpected unbound-device error: {unbound_error:?}"
    );

    let mut unknown_uid = vec![0x22; 33];
    unknown_uid[0] = ENTITY_USER;
    let unknown_uid = EntityId::from_bytes(unknown_uid).unwrap();
    let unknown_error = fixture
        .client
        .foks()
        .fetch_device_certificate_chain(fixture.host(), &unknown_uid, &SecretSeed::new([0x23; 32]))
        .unwrap_err();
    assert!(matches!(
        unknown_error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1049, .. })
    ));

    let unsupported = fixture
        .client
        .foks()
        .host_config(fixture.host(), &created.credential);
    assert!(
        matches!(
            &unsupported,
            Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: 1020,
                ..
            }))
        ),
        "unexpected unsupported-method response: {unsupported:?}"
    );
}
