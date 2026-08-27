use foks_client::AdHocTeamSecrets;
use foks_proto::SecretSeed;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestProfile};

fn secrets(seed: u8) -> AdHocTeamSecrets {
    AdHocTeamSecrets {
        member_min: SecretSeed::new([seed; 32]),
        member: SecretSeed::new([seed.wrapping_add(1); 32]),
        admin: SecretSeed::new([seed.wrapping_add(2); 32]),
        owner: SecretSeed::new([seed.wrapping_add(3); 32]),
    }
}

#[test]
fn exact_team_limit_succeeds_and_plus_one_has_no_partial_team() {
    let environment = TestEnvironment::with_profile(TestProfile::SmallTeamCapacity).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "team-capacity-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let account = client
        .create_account(&probe.pinned, &TestAccountSpec::new("teamcapacity", 0xc1))
        .unwrap();
    let first = client
        .foks()
        .create_single_owner_adhoc_team(&probe.pinned, &account.credential, &secrets(0x31))
        .unwrap();
    let second = client
        .foks()
        .create_single_owner_adhoc_team(&probe.pinned, &account.credential, &secrets(0x41))
        .unwrap();
    let error = match client.foks().create_single_owner_adhoc_team(
        &probe.pinned,
        &account.credential,
        &secrets(0x51),
    ) {
        Ok(_) => panic!("plus-one team unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1060, .. })
    ));
    for team in [&first.team, &second.team] {
        assert!(client
            .foks()
            .load_and_pin_team(
                &probe.pinned,
                &account.credential,
                &account.authenticated.verified,
                &account.authenticated.puks,
                team,
            )
            .is_ok());
    }
    server.shutdown().unwrap();
}
