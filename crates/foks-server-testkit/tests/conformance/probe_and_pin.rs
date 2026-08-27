use foks_client_db::Acceptance;

use crate::support::Fixture;

#[test]
pub(crate) fn probe_and_pin_success() {
    let fixture = Fixture::start("probe-client");
    assert_eq!(fixture.probe.acceptance, Acceptance::Inserted);
    let repeated = fixture.client.probe_and_pin().unwrap();
    assert_eq!(repeated.acceptance, Acceptance::Unchanged);
    assert_eq!(
        repeated.verified.snapshot.host_id(),
        fixture.probe.verified.snapshot.host_id()
    );
    assert_eq!(
        repeated.verified.public_zone.services.probe,
        format!("localhost:{}", fixture.server.addresses().probe.port())
    );
    assert_eq!(
        repeated.verified.public_zone.services.registration,
        format!(
            "localhost:{}",
            fixture.server.addresses().public_services.port()
        )
    );
    assert_eq!(
        repeated.verified.public_zone.services.user,
        format!(
            "localhost:{}",
            fixture.server.addresses().authenticated.port()
        )
    );

    let reservation = fixture
        .client
        .foks()
        .reserve_username(fixture.host(), "probeuser")
        .unwrap();
    assert_eq!(reservation.sequence, 1);
    let collision = fixture
        .client
        .foks()
        .reserve_username(fixture.host(), "probeuser")
        .unwrap_err();
    assert!(matches!(
        collision,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1023, .. })
    ));
    fixture.environment.advance_clock(10 * 60 * 1_000_000 + 1);
    let replacement = fixture
        .client
        .foks()
        .reserve_username(fixture.host(), "probeuser")
        .unwrap();
    assert_ne!(replacement.token, reservation.token);
    let (acceptance, root) = fixture
        .client
        .foks()
        .advance_merkle_root(fixture.host())
        .unwrap();
    assert_eq!(acceptance, Acceptance::Unchanged);
    assert_eq!(root.root().epoch, 1);
}
