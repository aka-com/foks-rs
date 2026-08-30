use foks_client::KexProvisionOffer;
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient};

use crate::support::Fixture;

#[test]
pub(crate) fn interactive_software_device_pairing() {
    let fixture = Fixture::start("kex-provisioner");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kex-user", 0x31))
        .unwrap();
    let provisionee = TestClient::new(&fixture.environment, "kex-provisionee").unwrap();
    let provisionee_host = provisionee.probe_and_pin().unwrap().pinned;

    let offer = KexProvisionOffer::generate(Role::OWNER).unwrap();
    let phrase = offer.phrase().expose_joined();
    fixture
        .client
        .foks()
        .publish_kex_provision_offer(fixture.host(), &created.credential, &offer)
        .unwrap();

    let provisioner = fixture.client.foks().clone();
    let provisioner_host = fixture.host().clone();
    let existing = created.credential;
    let mut protected = fixture.client.open_protected_store().unwrap();
    let finish = std::thread::spawn(move || {
        let first = provisioner
            .finish_kex_provisioning(&provisioner_host, &existing, &offer, &mut protected)
            .unwrap();
        // Model a provisioner crash after authenticated remote verification
        // but before the application finalized its protected mutation record.
        let resumed = provisioner
            .finish_kex_provisioning(&provisioner_host, &existing, &offer, &mut protected)
            .unwrap();
        (first, resumed)
    });

    let accepted = provisionee
        .foks()
        .accept_kex_provisioning(
            &provisionee_host,
            &phrase,
            "paired laptop",
            2,
            SecretSeed::new([0x41; 32]),
        )
        .unwrap();
    let (finished, resumed) = finish.join().unwrap();

    assert_eq!(accepted.authenticated.verified.chain_seqno(), 2);
    assert_eq!(accepted.authenticated.verified.devices().len(), 2);
    assert_eq!(
        finished.device,
        foks_crypto::derive_device_public(&SecretSeed::new([0x41; 32])).unwrap()
    );
    assert!(finished.operation_id.is_some());
    assert_eq!(resumed.operation_id, finished.operation_id);
    assert_eq!(resumed.device, finished.device);
}
