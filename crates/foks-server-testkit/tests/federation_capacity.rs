use foks_client::NamedTeamSecrets;
use foks_proto::{EntityId, FqParty, SecretSeed, ENTITY_HOST, ENTITY_USER};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestProfile};

fn entity(kind: u8, fill: u8) -> EntityId {
    EntityId::from_bytes([vec![kind], vec![fill; 32]].concat()).unwrap()
}

fn viewer(fill: u8) -> FqParty {
    FqParty::new(
        entity(ENTITY_USER, fill),
        entity(ENTITY_HOST, fill.wrapping_add(0x40)),
    )
    .unwrap()
}

fn secrets(seed: u8) -> NamedTeamSecrets {
    NamedTeamSecrets {
        member_min: SecretSeed::new([seed; 32]),
        member: SecretSeed::new([seed.wrapping_add(1); 32]),
        admin: SecretSeed::new([seed.wrapping_add(2); 32]),
        owner: SecretSeed::new([seed.wrapping_add(3); 32]),
        removal_key: SecretSeed::new([seed.wrapping_add(4); 32]),
        team_name_commitment_key: [seed.wrapping_add(5); 16],
    }
}

#[test]
fn federation_scope_quotas_are_atomic_and_expired_grants_release_capacity() {
    let environment = TestEnvironment::with_profile(TestProfile::SmallFederationCapacity).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "federation-capacity").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("fedcapacity", 0x21))
        .unwrap();
    let team = client
        .foks()
        .create_single_owner_named_team(
            &host,
            &account.credential,
            "fedcapacityteam",
            &secrets(0x31),
        )
        .unwrap();
    let second_client = TestClient::new(&environment, "federation-capacity-second").unwrap();
    let second_host = second_client.probe_and_pin().unwrap().pinned;
    let second_account = second_client
        .create_account(&second_host, &TestAccountSpec::new("fedcapacitytwo", 0x41))
        .unwrap();
    let second_team = second_client
        .foks()
        .create_single_owner_named_team(
            &second_host,
            &second_account.credential,
            "fedcapacityteamtwo",
            &secrets(0x42),
        )
        .unwrap();

    let first_user = client
        .foks()
        .grant_remote_user_view(&host, &account.credential, viewer(0x51))
        .unwrap();
    client
        .foks()
        .grant_remote_user_view(&host, &account.credential, viewer(0x52))
        .unwrap();
    assert_eq!(
        client
            .foks()
            .grant_remote_user_view(&host, &account.credential, viewer(0x51))
            .unwrap(),
        first_user
    );
    let user_error = client
        .foks()
        .grant_remote_user_view(&host, &account.credential, viewer(0x53))
        .unwrap_err();
    assert!(matches!(
        user_error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1012, .. })
    ));
    second_client
        .foks()
        .grant_remote_user_view(&second_host, &second_account.credential, viewer(0x54))
        .unwrap();
    let global_user_error = second_client
        .foks()
        .grant_remote_user_view(&second_host, &second_account.credential, viewer(0x55))
        .unwrap_err();
    assert!(matches!(
        global_user_error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1012, .. })
    ));

    let first_team = client
        .foks()
        .grant_remote_team_view(&host, &account.credential, &team.team, viewer(0x61))
        .unwrap();
    client
        .foks()
        .grant_remote_team_view(&host, &account.credential, &team.team, viewer(0x62))
        .unwrap();
    assert_eq!(
        client
            .foks()
            .grant_remote_team_view(&host, &account.credential, &team.team, viewer(0x61))
            .unwrap(),
        first_team
    );
    let team_error = client
        .foks()
        .grant_remote_team_view(&host, &account.credential, &team.team, viewer(0x63))
        .unwrap_err();
    assert!(matches!(
        team_error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1012, .. })
    ));
    second_client
        .foks()
        .grant_remote_team_view(
            &second_host,
            &second_account.credential,
            &second_team.team,
            viewer(0x64),
        )
        .unwrap();
    let global_team_error = second_client
        .foks()
        .grant_remote_team_view(
            &second_host,
            &second_account.credential,
            &second_team.team,
            viewer(0x65),
        )
        .unwrap_err();
    assert!(matches!(
        global_team_error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1012, .. })
    ));

    server.shutdown().unwrap();
    let now = environment.advance_clock(0);
    let database = rusqlite::Connection::open(environment.database_path()).unwrap();
    let user_count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM federation_user_view_permissions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let team_count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM federation_team_view_permissions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((user_count, team_count), (3, 3));
    database
        .execute(
            "UPDATE federation_user_view_permissions
             SET issued_at = 0, updated_at = 0, expires_at = ?1",
            [i64::try_from(now.saturating_sub(1)).unwrap()],
        )
        .unwrap();
    database
        .execute(
            "UPDATE federation_team_view_permissions
             SET issued_at = 0, updated_at = 0, expires_at = ?1",
            [i64::try_from(now.saturating_sub(1)).unwrap()],
        )
        .unwrap();
    drop(database);

    let restarted = environment.start_server().unwrap();
    let user_retry = TestClient::new(&environment, "federation-capacity-user-retry").unwrap();
    let user_retry_host = user_retry.probe_and_pin().unwrap().pinned;
    user_retry
        .foks()
        .grant_remote_user_view(&user_retry_host, &account.credential, viewer(0x53))
        .unwrap();
    let team_retry = TestClient::new(&environment, "federation-capacity-team-retry").unwrap();
    let team_retry_host = team_retry.probe_and_pin().unwrap().pinned;
    team_retry
        .foks()
        .grant_remote_team_view(
            &team_retry_host,
            &account.credential,
            &team.team,
            viewer(0x63),
        )
        .unwrap();
    restarted.shutdown().unwrap();
}
