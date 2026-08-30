use std::time::Duration;

use foks_server_testkit::{TestAccountSpec, TestClient};

use crate::support::Fixture;

#[test]
fn lock_tokens_expiry_and_user_scope_are_enforced() {
    let fixture = Fixture::start("kv-lock-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvlocks", 0x45))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let target = [0x44; 16];
    let mut protected = fixture.client.open_protected_store().unwrap();
    let mut session = fixture
        .client
        .foks()
        .user_kv_write_session(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let zero = session.acquire_lock(root, target, Duration::ZERO).unwrap();
    let zero_replacement = session.acquire_lock(root, target, Duration::ZERO).unwrap();
    assert_ne!(zero_replacement, zero);
    let timed_out_release = session.release_lock(root, target, zero).unwrap_err();
    assert!(matches!(
        timed_out_release,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 8015, .. })
    ));
    session
        .release_lock(root, target, zero_replacement)
        .unwrap();
    let long = session
        .acquire_lock(root, target, Duration::from_secs(24 * 60 * 60 + 1))
        .unwrap();
    session.release_lock(root, target, long).unwrap();
    let first = session
        .acquire_lock(root, target, Duration::from_secs(30))
        .unwrap();
    let contention = session
        .acquire_lock(root, target, Duration::from_secs(30))
        .unwrap_err();
    assert!(matches!(
        contention,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 8014, .. })
    ));
    let wrong_release = session.release_lock(root, target, [0x99; 16]).unwrap_err();
    assert!(matches!(
        wrong_release,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 8015, .. })
    ));
    fixture.environment.advance_clock(30 * 1_000_000 + 1);
    let replacement = session
        .acquire_lock(root, target, Duration::from_secs(30))
        .unwrap();
    assert_ne!(replacement, first);
    assert!(session.release_lock(root, target, first).is_err());
    session.release_lock(root, target, replacement).unwrap();
    drop(session);
    drop(protected);

    let other_client = TestClient::new(&fixture.environment, "other-lock-client").unwrap();
    let other_probe = other_client.probe_and_pin().unwrap();
    let other = other_client
        .create_account(
            &other_probe.pinned,
            &TestAccountSpec::new("otherlocks", 0x55),
        )
        .unwrap();
    let mut other_protected = other_client.open_protected_store().unwrap();
    let mut other_session = other_client
        .foks()
        .user_kv_write_session(
            &other_probe.pinned,
            &other.credential,
            &other.authenticated.verified,
            &other.authenticated.puks,
            other_client.soft_state_path(),
            &mut other_protected,
        )
        .unwrap();
    let cross_user = other_session
        .acquire_lock(root, target, Duration::from_secs(30))
        .unwrap_err();
    assert!(matches!(
        cross_user,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 8016, .. })
    ));
}
