use std::io::Write as _;
use std::sync::Arc;

use foks_client::{
    AddLocalTeamMemberRequest, ChangeTeamMemberRequest, FederatedTeamAdmissionRequest,
    NamedTeamSecrets, TeamMemberSelector, TeamPtkRotationSeed, VerifiedMemberParty,
};
use foks_proto::{EntityId, FqParty, PermissionToken, Role, SecretSeed, ENTITY_HOST, ENTITY_USER};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use rustls::pki_types::ServerName;

use crate::support::Fixture;

fn entity(kind: u8, fill: u8) -> EntityId {
    EntityId::from_bytes([vec![kind], vec![fill; 32]].concat()).unwrap()
}

#[test]
pub(crate) fn federation_lifecycle() {
    let fixture = Fixture::start("federation-owner");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("federateduser", 0xa1))
        .unwrap();
    let viewer = FqParty::new(entity(ENTITY_USER, 0xb1), entity(ENTITY_HOST, 0xb2)).unwrap();
    let token = fixture
        .client
        .foks()
        .grant_remote_user_view(fixture.host(), &created.credential, viewer.clone())
        .unwrap();
    let repeated = fixture
        .client
        .foks()
        .grant_remote_user_view(fixture.host(), &created.credential, viewer)
        .unwrap();
    assert_eq!(token, repeated);

    let remote = TestClient::new(&fixture.environment, "federation-remote").unwrap();
    let remote_host = remote.probe_and_pin().unwrap().pinned;
    let loaded = remote
        .foks()
        .load_remote_user_and_pin(&remote_host, &created.credential.uid, &token)
        .unwrap();
    assert_eq!(loaded.verified.username(), b"federateduser");

    let mut invalid = [0xc1; 17];
    invalid[0] = 54;
    let error = remote
        .foks()
        .load_remote_user_and_pin(
            &remote_host,
            &created.credential.uid,
            &PermissionToken::new(invalid),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1013, .. })
    ));

    let database = rusqlite::Connection::open_with_flags(
        fixture.environment.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (ciphertext, hash): (Vec<u8>, Vec<u8>) = database
        .query_row(
            "SELECT token_ciphertext, token_hash
             FROM federation_user_view_permissions WHERE target_user_id = ?1",
            [created.credential.uid.as_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(ciphertext.len(), 33);
    assert_eq!(hash.len(), 32);
    assert_ne!(ciphertext.as_slice(), token.expose());
    assert!(!ciphertext
        .windows(token.expose().len())
        .any(|window| window == token.expose()));
}

#[test]
pub(crate) fn remote_team_membership_and_ptk_tokens() {
    let remote = Fixture::start("federation-remote-team-owner");
    let remote_account = remote
        .client
        .create_account(
            remote.host(),
            &TestAccountSpec::new("remoteteamowner", 0xd1),
        )
        .unwrap();
    let remote_secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0xd2; 32]),
        member: SecretSeed::new([0xd3; 32]),
        admin: SecretSeed::new([0xd4; 32]),
        owner: SecretSeed::new([0xd5; 32]),
        removal_key: SecretSeed::new([0xd6; 32]),
        team_name_commitment_key: [0xd7; 16],
    };
    let remote_team = remote
        .client
        .foks()
        .create_single_owner_named_team(
            remote.host(),
            &remote_account.credential,
            "remotealpha",
            &remote_secrets,
        )
        .unwrap();

    let local = Fixture::start("federation-local-team-owner");
    let local_account = local
        .client
        .create_account(local.host(), &TestAccountSpec::new("localteamowner", 0xe1))
        .unwrap();
    let local_secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0xe2; 32]),
        member: SecretSeed::new([0xe3; 32]),
        admin: SecretSeed::new([0xe4; 32]),
        owner: SecretSeed::new([0xe5; 32]),
        removal_key: SecretSeed::new([0xe6; 32]),
        team_name_commitment_key: [0xe7; 16],
    };
    let local_team = local
        .client
        .foks()
        .create_single_owner_named_team(
            local.host(),
            &local_account.credential,
            "localbeta",
            &local_secrets,
        )
        .unwrap();

    let viewer = FqParty::new(local_team.team.clone(), local.host().host_id().clone()).unwrap();
    let permission = remote
        .client
        .foks()
        .grant_remote_team_view(
            remote.host(),
            &remote_account.credential,
            &remote_team.team,
            viewer,
        )
        .unwrap();
    let removal_key = SecretSeed::new([0xe8; 32]);
    let admission_request = FederatedTeamAdmissionRequest {
        remote_host: remote.host(),
        remote_credential: &remote_account.credential,
        remote_team: &remote_team.team,
        local_host: local.host(),
        local_credential: &local_account.credential,
        local_team: &local_team.team,
        destination_role: Role::member(0),
        removal_key: &removal_key,
    };
    let mut protected = local.client.open_protected_store().unwrap();
    let admitted = local
        .client
        .foks()
        .admit_remote_team_to_named_team(&admission_request, &mut protected)
        .unwrap();
    assert_eq!(admitted.remote.verified.team_name(), b"remotealpha");
    let repeated = local
        .client
        .foks()
        .admit_remote_team_to_named_team(&admission_request, &mut protected)
        .unwrap();
    assert_eq!(repeated.operation_id, admitted.operation_id);
    let remote_member = admitted
        .added
        .authenticated
        .verified
        .members()
        .iter()
        .find(|member| member.party == remote_team.team)
        .unwrap();
    assert_eq!(
        remote_member.scoped_host.as_ref(),
        Some(remote.host().host_id())
    );

    let recovered = local
        .client
        .foks()
        .load_remote_member_view_permissions(
            local.host(),
            &local_account.credential,
            &local_team.team,
            &[FqParty::new(remote_team.team.clone(), remote.host().host_id().clone()).unwrap()],
        )
        .unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].permission, permission);
    let reader = TestClient::new(&remote.environment, "remote-team-public-reader").unwrap();
    let reader_host = reader.probe_and_pin().unwrap().pinned;
    let reloaded = reader
        .foks()
        .load_remote_team_and_pin(&reader_host, &remote_team.team, &recovered[0].permission)
        .unwrap();
    assert_eq!(reloaded.verified.team(), &remote_team.team);

    // An unrelated local-member removal rotates the member-floor PTKs. The
    // federation bearer box stays bound to its authenticated historical PTK
    // generation and must remain recoverable through the PTK seed chain.
    let departing_client =
        TestClient::new(&local.environment, "federation-departing-member").unwrap();
    let departing_host = departing_client.probe_and_pin().unwrap().pinned;
    let departing = departing_client
        .create_account(
            &departing_host,
            &TestAccountSpec::new("federationdeparting", 0xea),
        )
        .unwrap();
    let departing_removal_key = SecretSeed::new([0xeb; 32]);
    local
        .client
        .foks()
        .add_local_user_to_named_team(
            local.host(),
            &local_account.credential,
            &local_team.team,
            &AddLocalTeamMemberRequest {
                target_user: &departing.authenticated.verified,
                destination_role: Role::member(0),
                removal_key: &departing_removal_key,
            },
        )
        .unwrap();
    let rotated_min = SecretSeed::new([0xec; 32]);
    let rotated_member = SecretSeed::new([0xed; 32]);
    let rotations = [
        TeamPtkRotationSeed {
            role: Role::member(-0x4000),
            seed: &rotated_min,
        },
        TeamPtkRotationSeed {
            role: Role::member(0),
            seed: &rotated_member,
        },
    ];
    let remaining = [VerifiedMemberParty::Team(&admitted.remote.verified)];
    local
        .client
        .foks()
        .change_team_member_and_rotate_ptks(
            local.host(),
            &local_account.credential,
            &local_team.team,
            &ChangeTeamMemberRequest {
                target: TeamMemberSelector {
                    party: departing.authenticated.verified.uid(),
                    host: None,
                    source_role: Role::OWNER,
                },
                destination_role: Role::NONE,
                replacement: None,
                rotations: &rotations,
                remaining_parties: &remaining,
            },
        )
        .unwrap();
    let recovered_after_rotation = local
        .client
        .foks()
        .load_remote_member_view_permissions(
            local.host(),
            &local_account.credential,
            &local_team.team,
            &[FqParty::new(remote_team.team.clone(), remote.host().host_id().clone()).unwrap()],
        )
        .unwrap();
    assert_eq!(recovered_after_rotation[0].permission, permission);

    let client_database = rusqlite::Connection::open_with_flags(
        local.client.hard_state_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (state, permission_hash): (i64, Vec<u8>) = client_database
        .query_row(
            "SELECT state, permission_hash FROM federation_saga_operations
             WHERE operation_id = ?1",
            [admitted.operation_id.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, 5);
    assert_eq!(permission_hash.len(), 32);
    let client_bytes = std::fs::read(local.client.hard_state_path()).unwrap();
    assert!(!client_bytes
        .windows(permission.expose().len())
        .any(|window| window == permission.expose()));

    let database = rusqlite::Connection::open_with_flags(
        local.environment.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let exact_box: Vec<u8> = database
        .query_row(
            "SELECT exact_secret_box FROM team_remote_member_view_tokens
             WHERE target_team_id = ?1 AND member_party_id = ?2",
            rusqlite::params![local_team.team.as_bytes(), remote_team.team.as_bytes()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exact_box
        .windows(permission.expose().len())
        .any(|window| window == permission.expose()));
}

#[test]
pub(crate) fn remote_team_permission_renewal_preserves_the_embedded_bearer() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "federation-renewal-owner").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("renewalowner", 0xf1))
        .unwrap();
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0xf2; 32]),
        member: SecretSeed::new([0xf3; 32]),
        admin: SecretSeed::new([0xf4; 32]),
        owner: SecretSeed::new([0xf5; 32]),
        removal_key: SecretSeed::new([0xf6; 32]),
        team_name_commitment_key: [0xf7; 16],
    };
    let team = client
        .foks()
        .create_single_owner_named_team(&host, &account.credential, "renewable", &secrets)
        .unwrap();
    let viewer = FqParty::new(entity(ENTITY_USER, 0xf8), entity(ENTITY_HOST, 0xf9)).unwrap();
    let first = client
        .foks()
        .grant_remote_team_view(&host, &account.credential, &team.team, viewer.clone())
        .unwrap();

    server.shutdown().unwrap();
    let now = environment.advance_clock(0);
    let database = rusqlite::Connection::open(environment.database_path()).unwrap();
    database
        .execute(
            "UPDATE federation_team_view_permissions
             SET expires_at = ?4
             WHERE target_team_id = ?1 AND viewer_party_id = ?2 AND viewer_host_id = ?3",
            rusqlite::params![
                team.team.as_bytes(),
                viewer.party.as_bytes(),
                viewer.host.as_bytes(),
                i64::try_from(now + 24 * 60 * 60 * 1_000_000).unwrap(),
            ],
        )
        .unwrap();
    drop(database);

    let _server = environment.start_server().unwrap();
    let renewed_client = TestClient::new(&environment, "federation-renewal-retry").unwrap();
    let renewed_host = renewed_client.probe_and_pin().unwrap().pinned;
    let renewed = renewed_client
        .foks()
        .grant_remote_team_view(&renewed_host, &account.credential, &team.team, viewer)
        .unwrap();
    assert_eq!(renewed, first);

    let database = rusqlite::Connection::open_with_flags(
        environment.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (token_hash, expires_at): (Vec<u8>, i64) = database
        .query_row(
            "SELECT token_hash, expires_at FROM federation_team_view_permissions
             WHERE target_team_id = ?1",
            [team.team.as_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        token_hash,
        foks_crypto::federation_permission_token_hash(&first)
    );
    assert!(u64::try_from(expires_at).unwrap() > now + 29 * 24 * 60 * 60 * 1_000_000);
}

#[test]
pub(crate) fn unsupported_federation_routes() {
    let fixture = Fixture::start("unsupported-federation");
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    for certificate in fixture.host().tls_ca_certificates() {
        roots
            .add(rustls::pki_types::CertificateDer::from(certificate.clone()))
            .unwrap();
    }
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    for route in foks_server::rpc::ROUTES
        .iter()
        .filter(|route| route.protocol == "Beacon" && !route.supported)
    {
        let tcp = std::net::TcpStream::connect(fixture.server.addresses().public_services).unwrap();
        let connection = rustls::ClientConnection::new(
            Arc::new(config.clone()),
            ServerName::try_from("localhost".to_owned()).unwrap(),
        )
        .unwrap();
        let mut tls = rustls::StreamOwned::new(connection, tcp);
        let argument = foks_snowpack::encode(&foks_snowpack::Value::Null).unwrap();
        let request =
            foks_rpc::encode_call(route.protocol_id, route.position, &argument, 0).unwrap();
        tls.write_all(&request).unwrap();
        let error = foks_rpc::read_response(&mut tls, 4096, 0).unwrap_err();
        assert!(matches!(
            error,
            foks_rpc::Error::RemoteStatus { code: 1020, .. }
        ));
    }
}
