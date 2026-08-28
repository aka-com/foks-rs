use foks_proto::InviteCode;
use foks_server_db::InviteRegime;
use foks_server_testkit::{TestAccountSpec, TestClient};
use std::sync::{Arc, Barrier};

use crate::support::Fixture;

#[test]
pub(crate) fn signup_invites_success() {
    let fixture = Fixture::start("invite-control-client");

    let standard = fixture.environment.issue_standard_invite(None).unwrap();
    let standard_code = InviteCode::from_user_input(&standard.code, false).unwrap();
    fixture
        .client
        .foks()
        .check_invite_code(fixture.host(), &standard_code)
        .unwrap();
    let standard_client = TestClient::new(&fixture.environment, "standard-invite-client").unwrap();
    standard_client.probe_and_pin().unwrap();
    standard_client
        .create_account(
            fixture.host(),
            &TestAccountSpec::new("standardinvite", 0x81).with_invite(standard_code.clone()),
        )
        .unwrap();
    assert_bad_invite(
        fixture
            .client
            .foks()
            .check_invite_code(fixture.host(), &standard_code),
    );

    fixture
        .environment
        .set_invite_regime(InviteRegime::Required)
        .unwrap();
    assert_bad_invite(
        fixture
            .client
            .foks()
            .check_invite_code(fixture.host(), &InviteCode::Empty),
    );

    let multiuse = fixture
        .environment
        .issue_multiuse_invite("Small-Team+Launch", Some(2), None)
        .unwrap();
    let multiuse_code = InviteCode::from_user_input(&multiuse.code, false).unwrap();
    for (id, username, seed) in [
        ("multiuse-one", "multiuseone", 0x91),
        ("multiuse-two", "multiusetwo", 0xa1),
    ] {
        let client = TestClient::new(&fixture.environment, id).unwrap();
        client.probe_and_pin().unwrap();
        client
            .create_account(
                fixture.host(),
                &TestAccountSpec::new(username, seed).with_invite(multiuse_code.clone()),
            )
            .unwrap();
    }
    assert_bad_invite(
        fixture
            .client
            .foks()
            .check_invite_code(fixture.host(), &multiuse_code),
    );

    let disabled = fixture
        .environment
        .issue_multiuse_invite("disabled-code", None, None)
        .unwrap();
    fixture.environment.advance_clock(1);
    assert!(fixture.environment.disable_invite(&disabled.code).unwrap());
    assert_bad_invite(fixture.client.foks().check_invite_code(
        fixture.host(),
        &InviteCode::from_user_input(&disabled.code, false).unwrap(),
    ));

    let expires_at = fixture.environment.advance_clock(10);
    let expiring = fixture
        .environment
        .issue_multiuse_invite("expiring-code", None, Some(expires_at + 10))
        .unwrap();
    fixture.environment.advance_clock(11);
    assert_bad_invite(fixture.client.foks().check_invite_code(
        fixture.host(),
        &InviteCode::from_user_input(&expiring.code, false).unwrap(),
    ));

    let contested = fixture.environment.issue_standard_invite(None).unwrap();
    let contested = InviteCode::from_user_input(&contested.code, false).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let handles = [
        ("invite-racer-one", "inviteracerone", 0xb1),
        ("invite-racer-two", "inviteracertwo", 0xc1),
    ]
    .into_iter()
    .map(|(client_id, username, seed)| {
        let environment = fixture.environment.clone();
        let barrier = Arc::clone(&barrier);
        let code = contested.clone();
        std::thread::spawn(move || {
            let client = TestClient::new(&environment, client_id).unwrap();
            let host = client.probe_and_pin().unwrap().pinned;
            barrier.wait();
            client.create_account(
                &host,
                &TestAccountSpec::new(username, seed).with_invite(code),
            )
        })
    })
    .collect::<Vec<_>>();
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|result| result.is_err()).count(), 1);
    assert_bad_invite(
        fixture
            .client
            .foks()
            .check_invite_code(fixture.host(), &contested),
    );
}

fn assert_bad_invite(result: foks_client::Result<()>) {
    assert!(matches!(
        result,
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1019,
            ..
        }))
    ));
}
