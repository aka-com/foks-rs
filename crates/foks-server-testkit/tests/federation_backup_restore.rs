use foks_client::NamedTeamSecrets;
use foks_proto::{EntityId, FqParty, SecretSeed, ENTITY_HOST, ENTITY_USER};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};

fn entity(kind: u8, fill: u8) -> EntityId {
    EntityId::from_bytes([vec![kind], vec![fill; 32]].concat()).unwrap()
}

#[test]
fn backup_restores_live_federation_bearers_and_their_key_generation() {
    let source = TestEnvironment::new().unwrap();
    let server = source.start_server().unwrap();
    let addresses = server.addresses();
    let owner = TestClient::new(&source, "federation-backup-owner").unwrap();
    let host = owner.probe_and_pin().unwrap().pinned;
    let account = owner
        .create_account(&host, &TestAccountSpec::new("fedbackup", 0x71))
        .unwrap();
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0x72; 32]),
        member: SecretSeed::new([0x73; 32]),
        admin: SecretSeed::new([0x74; 32]),
        owner: SecretSeed::new([0x75; 32]),
        removal_key: SecretSeed::new([0x76; 32]),
        team_name_commitment_key: [0x77; 16],
    };
    let team = owner
        .foks()
        .create_single_owner_named_team(&host, &account.credential, "fedbackupteam", &secrets)
        .unwrap();
    let viewer = FqParty::new(entity(ENTITY_USER, 0x78), entity(ENTITY_HOST, 0x79)).unwrap();
    let user_permission = owner
        .foks()
        .grant_remote_user_view(&host, &account.credential, viewer.clone())
        .unwrap();
    let team_permission = owner
        .foks()
        .grant_remote_team_view(&host, &account.credential, &team.team, viewer.clone())
        .unwrap();

    let reader = TestClient::new(&source, "federation-backup-reader").unwrap();
    reader.probe_and_pin().unwrap();
    server.shutdown().unwrap();
    let rotation = source.rotate_capability_key().unwrap();
    assert_eq!(rotation.retiring_generation_ids.len(), 1);
    let rotated_server = source.start_server().unwrap();
    let artifacts = rotated_server
        .backup_named("federation-live-grants")
        .unwrap();
    assert!(rotated_server.backup_is_valid(&artifacts).unwrap());
    rotated_server.shutdown().unwrap();

    let restored = TestEnvironment::restore_backup(&artifacts, addresses).unwrap();
    let restored_server = restored.start_server().unwrap();
    let reconstructed_owner = TestClient::new(&source, "federation-backup-owner").unwrap();
    let restored_owner_host = reconstructed_owner.pinned_host().unwrap();
    assert_eq!(
        reconstructed_owner
            .foks()
            .grant_remote_user_view(&restored_owner_host, &account.credential, viewer.clone(),)
            .unwrap(),
        user_permission
    );
    assert_eq!(
        reconstructed_owner
            .foks()
            .grant_remote_team_view(
                &restored_owner_host,
                &account.credential,
                &team.team,
                viewer,
            )
            .unwrap(),
        team_permission
    );
    let reconstructed = TestClient::new(&source, "federation-backup-reader").unwrap();
    let restored_host = reconstructed.pinned_host().unwrap();
    let user = reconstructed
        .foks()
        .load_remote_user_and_pin(&restored_host, &account.credential.uid, &user_permission)
        .unwrap();
    assert_eq!(user.verified.username(), b"fedbackup");
    let restored_team = reconstructed
        .foks()
        .load_remote_team_and_pin(&restored_host, &team.team, &team_permission)
        .unwrap();
    assert_eq!(restored_team.verified.team_name(), b"fedbackupteam");

    let metrics = restored_server.metrics();
    assert!(metrics.requests_started >= 4);
    assert!(metrics.responses_completed >= 4);
    assert!(metrics.handler_duration_observations >= 4);
    restored_server.shutdown().unwrap();
}
