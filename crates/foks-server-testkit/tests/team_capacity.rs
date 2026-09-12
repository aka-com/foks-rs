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
fn repeated_team_views_roll_over_same_member_capabilities() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "team-view-rollover-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let account = client
        .create_account(&probe.pinned, &TestAccountSpec::new("teamviewer", 0xb1))
        .unwrap();
    let team = client
        .foks()
        .create_single_owner_adhoc_team(&probe.pinned, &account.credential, &secrets(0x21))
        .unwrap();

    let mut view_tokens = Vec::new();
    for attempt in 0..40 {
        let loaded = client.foks().load_and_pin_team(
            &probe.pinned,
            &account.credential,
            &account.authenticated.verified,
            &account.authenticated.puks,
            &team.team,
        );
        match loaded {
            Ok(loaded) => view_tokens.push(loaded.view_token),
            Err(error) => panic!("team view {attempt} failed: {error}"),
        }
    }
    let database = rusqlite::Connection::open(environment.database_path()).unwrap();
    let tokens: i64 = database
        .query_row(
            "SELECT count(*) FROM team_view_tokens WHERE team_id = ?1 AND member_id = ?2",
            rusqlite::params![team.team.as_bytes(), account.credential.uid.as_bytes()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tokens, 32);
    const TEAM_VIEW_TOKEN_TYPE_ID: u64 = 0x6d10_7e4a_464f_4b53;
    let stored = |token: &[u8; 16]| -> i64 {
        database
            .query_row(
                "SELECT count(*) FROM team_view_tokens WHERE token_hash = ?1",
                [foks_crypto::prefixed_hash(TEAM_VIEW_TOKEN_TYPE_ID, token)],
                |row| row.get(0),
            )
            .unwrap()
    };
    let retained = view_tokens.iter().map(&stored).sum::<i64>();
    assert!((31..=32).contains(&retained));
    assert!(
        view_tokens
            .iter()
            .filter(|token| stored(token) == 0)
            .count()
            >= 8
    );
    assert_eq!(stored(view_tokens.last().unwrap()), 1);
    server.shutdown().unwrap();
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
