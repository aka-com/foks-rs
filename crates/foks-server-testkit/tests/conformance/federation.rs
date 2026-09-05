use std::io::Write as _;
use std::sync::Arc;

use foks_client::{
    AddLocalTeamMemberRequest, ChangeTeamMemberRequest, FederatedTeamAdmissionRequest,
    FederatedTeamRefreshRequest, FederationCredential, NamedTeamSecrets, NewYubiDeviceSecrets,
    RetainedTeamMemberRemovalRequest, TeamMemberSelector, TeamPtkRotationSeed, VerifiedMemberParty,
    YubiCredential, YubiDeviceProvisionRequest,
};
use foks_proto::{
    EntityId, FqParty, PermissionToken, Role, SecretSeed, YubiSlotAndPqKeyId, ENTITY_HOST,
    ENTITY_USER,
};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use foks_yubi::{MockYubiProvider, Pin, PivPolicy, SlotId, YubiProvider as _};
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
    let recipient = loaded.verified_recipient().unwrap();
    assert_eq!(recipient.verified().uid(), &created.credential.uid);

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
    let mut remote_protected = remote.client.open_protected_store().unwrap();
    let remote_metadata_fault = remote
        .environment
        .arm_fault(foks_server_testkit::TestFault::FederationTeamEditAfterCommitBeforeResponse);
    let allocated = local
        .client
        .foks()
        .allocate_federated_team_index_ranges(
            &admission_request,
            &mut remote_protected,
            &mut protected,
        )
        .unwrap();
    assert_eq!(remote.environment.fault_hits(), remote_metadata_fault + 1);
    assert_eq!(allocated.child.high.base, [0x20]);
    assert_eq!(allocated.parent.low.base, [0x80]);
    let admitted = local
        .client
        .foks()
        .admit_remote_team_to_named_team(&admission_request, &mut protected)
        .unwrap();
    assert_eq!(admitted.remote.verified.team_name(), b"remotealpha");
    assert_eq!(admitted.remote.verified.chain_seqno(), 2);
    assert_eq!(admitted.added.authenticated.verified.chain_seqno(), 3);
    assert!(matches!(
        admitted
            .remote
            .verified
            .group_change_at(2)
            .unwrap()
            .metadata
            .as_slice(),
        [foks_proto::ChangeMetadata::TeamIndexRange(_)]
    ));
    assert!(matches!(
        admitted
            .added
            .authenticated
            .verified
            .group_change_at(2)
            .unwrap()
            .metadata
            .as_slice(),
        [foks_proto::ChangeMetadata::TeamIndexRange(_)]
    ));
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
    let stale_remote_direct = [VerifiedMemberParty::User(
        &remote_account.authenticated.verified,
    )];
    assert!(admitted
        .remote
        .verified_recipient(&stale_remote_direct)
        .is_err());
    let current_remote_user = remote
        .client
        .foks()
        .authenticate_and_pin(remote.host(), &remote_account.credential)
        .unwrap();
    let remote_direct = [VerifiedMemberParty::User(&current_remote_user.verified)];
    let remote_recipient = admitted.remote.verified_recipient(&remote_direct).unwrap();
    let remaining = [VerifiedMemberParty::Team(&remote_recipient)];
    let mut protected = local.client.open_protected_store().unwrap();
    let after_rotation = local
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
            &mut protected,
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

    // Expel the exact host-scoped team with its admission-time removal key.
    // Exercise exact-selector, retained-key, and actor failures before the
    // successful edit, then prove that the edit transaction itself invalidates
    // a bearer granted to the removed exact party+host.
    let target = after_rotation
        .authenticated
        .verified
        .members()
        .iter()
        .find(|member| {
            member.party == remote_team.team
                && member.scoped_host.as_ref() == Some(remote.host().host_id())
        })
        .unwrap();
    let selector = TeamMemberSelector {
        party: &remote_team.team,
        host: Some(remote.host().host_id()),
        source_role: target.source_role,
    };
    let roles = local
        .client
        .foks()
        .team_member_rotation_roles(&after_rotation.authenticated, selector, Role::NONE, None)
        .unwrap();
    let rotation_seeds = roles
        .iter()
        .enumerate()
        .map(|(index, _)| SecretSeed::new([0x90 + index as u8; 32]))
        .collect::<Vec<_>>();
    let expulsion_rotations = roles
        .iter()
        .zip(&rotation_seeds)
        .map(|(role, seed)| TeamPtkRotationSeed { role: *role, seed })
        .collect::<Vec<_>>();
    let remaining = [];
    for wrong_target in [
        TeamMemberSelector {
            party: &remote_team.team,
            host: Some(&entity(ENTITY_HOST, 0x91)),
            source_role: target.source_role,
        },
        TeamMemberSelector {
            party: &entity(foks_proto::ENTITY_NAMED_TEAM, 0x92),
            host: Some(remote.host().host_id()),
            source_role: target.source_role,
        },
    ] {
        let error = local
            .client
            .foks()
            .retained_team_member_removal_operation_id(
                &local_account.credential.uid,
                &local_team.team,
                &after_rotation.authenticated,
                &RetainedTeamMemberRemovalRequest {
                    target: wrong_target,
                    removal_key: &removal_key,
                    rotations: &expulsion_rotations,
                    remaining_parties: &remaining,
                },
            )
            .unwrap_err();
        assert!(matches!(error, foks_client::Error::TeamRequest(_)));
    }
    let wrong_removal_key = SecretSeed::new([0x93; 32]);
    let error = local
        .client
        .foks()
        .retained_team_member_removal_operation_id(
            &local_account.credential.uid,
            &local_team.team,
            &after_rotation.authenticated,
            &RetainedTeamMemberRemovalRequest {
                target: selector,
                removal_key: &wrong_removal_key,
                rotations: &expulsion_rotations,
                remaining_parties: &remaining,
            },
        )
        .unwrap_err();
    assert!(matches!(error, foks_client::Error::KeyBinding(_)));

    let mut unauthorized_protected = local.client.open_protected_store().unwrap();
    let error = match local
        .client
        .foks()
        .remove_retained_team_member_and_rotate_ptks(
            local.host(),
            &departing.credential,
            &local_team.team,
            &RetainedTeamMemberRemovalRequest {
                target: selector,
                removal_key: &removal_key,
                rotations: &expulsion_rotations,
                remaining_parties: &remaining,
            },
            &mut unauthorized_protected,
        ) {
        Ok(_) => panic!("removed member unexpectedly authorized a federated expulsion"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1013, .. })
            | foks_client::Error::KeyBinding(_)
            | foks_client::Error::TeamBinding(_)
    ));

    let expelled_viewer =
        FqParty::new(remote_team.team.clone(), remote.host().host_id().clone()).unwrap();
    let expelled_bearer = local
        .client
        .foks()
        .grant_remote_team_view(
            local.host(),
            &local_account.credential,
            &local_team.team,
            expelled_viewer,
        )
        .unwrap();
    let local_reader = TestClient::new(&local.environment, "expelled-team-public-reader").unwrap();
    let local_reader_host = local_reader.probe_and_pin().unwrap().pinned;
    local_reader
        .foks()
        .load_remote_team_and_pin(&local_reader_host, &local_team.team, &expelled_bearer)
        .unwrap();

    let mut protected = local.client.open_protected_store().unwrap();
    local
        .client
        .foks()
        .remove_retained_team_member_and_rotate_ptks(
            local.host(),
            &local_account.credential,
            &local_team.team,
            &RetainedTeamMemberRemovalRequest {
                target: selector,
                removal_key: &removal_key,
                rotations: &expulsion_rotations,
                remaining_parties: &remaining,
            },
            &mut protected,
        )
        .unwrap();
    let old_bearer = local_reader
        .foks()
        .load_remote_team_and_pin(&local_reader_host, &local_team.team, &expelled_bearer)
        .unwrap_err();
    assert!(matches!(
        old_bearer,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1013, .. })
    ));
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
    assert_eq!(expires_at, i64::MAX);
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

/// One YubiKey enrolled onto an existing software account, kept as owned parts
/// so the borrowing [`YubiCredential`] can be rebuilt by each caller.
struct EnrolledYubi {
    prepared: foks_yubi::PreparedYubiDevice,
    subkey_seed: SecretSeed,
    certificate_chain: Vec<Vec<u8>>,
    uid: EntityId,
}

impl EnrolledYubi {
    fn credential(&self) -> YubiCredential<'_> {
        YubiCredential {
            uid: self.uid.clone(),
            parent: self.prepared.device.as_ref(),
            subkey_seed: SecretSeed::new(*self.subkey_seed.as_bytes()),
            certificate_chain: self.certificate_chain.clone(),
        }
    }
}

fn provision_owner_yubi(
    fixture: &Fixture,
    existing: &foks_client::DeviceCredential,
    card_name: &str,
    serial: u32,
    subkey_fill: u8,
) -> EnrolledYubi {
    let pin = Pin::new("123456").unwrap();
    let provider = MockYubiProvider::with_card(card_name, serial, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let prepared = provider
        .prepare(
            &card,
            SlotId::new(0x82).unwrap(),
            SlotId::new(0x83).unwrap(),
            &pin,
            None,
            PivPolicy::Once,
            PivPolicy::Never,
        )
        .unwrap();
    let subkey_seed = SecretSeed::new([subkey_fill; 32]);
    let mut protected = fixture.client.open_protected_store().unwrap();
    let provisioned = fixture
        .client
        .foks()
        .provision_yubi_device(
            fixture.host(),
            existing,
            prepared.device.as_ref(),
            YubiDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: format!("{card_name} owner key"),
                serial: u64::from(serial),
                pq_hint: YubiSlotAndPqKeyId {
                    slot: 0x83,
                    id: prepared.locator.pq_key_id,
                },
            },
            NewYubiDeviceSecrets::new(
                SecretSeed::new(*subkey_seed.as_bytes()),
                [subkey_fill ^ 0x5a; 17],
            ),
            &mut protected,
        )
        .unwrap();
    let certificate_chain = provisioned.credential.certificate_chain.clone();
    let uid = provisioned.credential.uid.clone();
    drop(provisioned);
    EnrolledYubi {
        prepared,
        subkey_seed,
        certificate_chain,
        uid,
    }
}

/// Two federated hosts whose owning administrators each hold both a software
/// device and an enrolled YubiKey.
struct FederatedYubiPair {
    remote: Fixture,
    remote_account: foks_client::CreatedSoftwareAccount,
    remote_team: EntityId,
    remote_yubi: EnrolledYubi,
    local: Fixture,
    local_account: foks_client::CreatedSoftwareAccount,
    local_team: EntityId,
    local_yubi: EnrolledYubi,
}

fn federated_yubi_pair(tag: &str, fill: u8) -> FederatedYubiPair {
    let remote = Fixture::start(&format!("federation-yubi-remote-{tag}"));
    let remote_account = remote
        .client
        .create_account(
            remote.host(),
            &TestAccountSpec::new(format!("fedyubirem{tag}"), fill),
        )
        .unwrap();
    let remote_secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([fill ^ 0x11; 32]),
        member: SecretSeed::new([fill ^ 0x12; 32]),
        admin: SecretSeed::new([fill ^ 0x13; 32]),
        owner: SecretSeed::new([fill ^ 0x14; 32]),
        removal_key: SecretSeed::new([fill ^ 0x15; 32]),
        team_name_commitment_key: [fill ^ 0x16; 16],
    };
    let remote_team = remote
        .client
        .foks()
        .create_single_owner_named_team(
            remote.host(),
            &remote_account.credential,
            &format!("remyubi{tag}"),
            &remote_secrets,
        )
        .unwrap()
        .team;

    let local = Fixture::start(&format!("federation-yubi-local-{tag}"));
    let local_account = local
        .client
        .create_account(
            local.host(),
            &TestAccountSpec::new(format!("fedyubiloc{tag}"), fill ^ 0x20),
        )
        .unwrap();
    let local_secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([fill ^ 0x21; 32]),
        member: SecretSeed::new([fill ^ 0x22; 32]),
        admin: SecretSeed::new([fill ^ 0x23; 32]),
        owner: SecretSeed::new([fill ^ 0x24; 32]),
        removal_key: SecretSeed::new([fill ^ 0x25; 32]),
        team_name_commitment_key: [fill ^ 0x26; 16],
    };
    let local_team = local
        .client
        .foks()
        .create_single_owner_named_team(
            local.host(),
            &local_account.credential,
            &format!("locyubi{tag}"),
            &local_secrets,
        )
        .unwrap()
        .team;

    let removal_key = SecretSeed::new([fill ^ 0x27; 32]);
    let mut protected = local.client.open_protected_store().unwrap();
    let mut remote_protected = remote.client.open_protected_store().unwrap();
    let admission = FederatedTeamAdmissionRequest {
        remote_host: remote.host(),
        remote_credential: &remote_account.credential,
        remote_team: &remote_team,
        local_host: local.host(),
        local_credential: &local_account.credential,
        local_team: &local_team,
        destination_role: Role::member(0),
        removal_key: &removal_key,
    };
    local
        .client
        .foks()
        .allocate_federated_team_index_ranges(&admission, &mut remote_protected, &mut protected)
        .unwrap();
    local
        .client
        .foks()
        .admit_remote_team_to_named_team(&admission, &mut protected)
        .unwrap();
    drop(protected);

    let local_yubi = provision_owner_yubi(
        &local,
        &local_account.credential,
        &format!("local-federation-yubikey-{tag}"),
        73_001,
        fill ^ 0x31,
    );
    let remote_yubi = provision_owner_yubi(
        &remote,
        &remote_account.credential,
        &format!("remote-federation-yubikey-{tag}"),
        73_002,
        fill ^ 0x32,
    );
    FederatedYubiPair {
        remote,
        remote_account,
        remote_team,
        remote_yubi,
        local,
        local_account,
        local_team,
        local_yubi,
    }
}

/// The federated security responder must run for every combination of
/// software- and hardware-backed administrator on the two sides. A Yubi-only
/// administrator that could not renew the remote bearer would leave a revoked
/// member key inside future federated team material indefinitely.
#[test]
pub(crate) fn federated_refresh_accepts_software_and_yubi_credentials_on_both_sides() {
    let pair = federated_yubi_pair("combos", 0xf1);
    let local_yubi = pair.local_yubi.credential();
    let remote_yubi = pair.remote_yubi.credential();
    let combinations: [(FederationCredential<'_, '_>, FederationCredential<'_, '_>); 4] = [
        (
            FederationCredential::Software(&pair.remote_account.credential),
            FederationCredential::Software(&pair.local_account.credential),
        ),
        (
            FederationCredential::Software(&pair.remote_account.credential),
            FederationCredential::Yubi(&local_yubi),
        ),
        (
            FederationCredential::Yubi(&remote_yubi),
            FederationCredential::Software(&pair.local_account.credential),
        ),
        (
            FederationCredential::Yubi(&remote_yubi),
            FederationCredential::Yubi(&local_yubi),
        ),
    ];
    for (index, (remote_credential, local_credential)) in combinations.into_iter().enumerate() {
        // Each combination starts from fresh sockets. A pooled connection the
        // server has already timed out would otherwise surface as a transport
        // error and hide the credential behaviour under test.
        pair.local.client.foks().clear_connection_pool().unwrap();
        let refreshed = pair
            .local
            .client
            .foks()
            .refresh_federated_team_capability(&FederatedTeamRefreshRequest {
                remote_host: pair.remote.host(),
                remote_credential,
                remote_team: &pair.remote_team,
                local_host: pair.local.host(),
                local_credential,
                local_team: &pair.local_team,
            })
            .unwrap_or_else(|error| panic!("credential combination {index} failed: {error}"));
        assert_eq!(refreshed.verified.team(), &pair.remote_team);
    }
}

/// Generalizing the refresh over credential forms must not let a caller
/// substitute one identity for another. Each rejection below would otherwise
/// let a party that does not hold the hardware drive the responder.
#[test]
pub(crate) fn federated_refresh_rejects_mismatched_identity_host_and_authority() {
    let pair = federated_yubi_pair("binding", 0x71);
    let local_yubi = pair.local_yubi.credential();

    // A valid local transport paired with the wrong hardware parent. Only the
    // parent differs: the UID, subkey, and certificate chain are the enrolled
    // local ones, so a check that trusted the transport alone would accept it.
    let impostor = YubiCredential {
        uid: local_yubi.uid.clone(),
        parent: pair.remote_yubi.prepared.device.as_ref(),
        subkey_seed: SecretSeed::new(*local_yubi.subkey_seed.as_bytes()),
        certificate_chain: local_yubi.certificate_chain.clone(),
    };
    pair.local.client.foks().clear_connection_pool().unwrap();
    let wrong_identity = pair
        .local
        .client
        .foks()
        .refresh_federated_team_capability(&FederatedTeamRefreshRequest {
            remote_host: pair.remote.host(),
            remote_credential: FederationCredential::Software(&pair.remote_account.credential),
            remote_team: &pair.remote_team,
            local_host: pair.local.host(),
            local_credential: FederationCredential::Yubi(&impostor),
            local_team: &pair.local_team,
        })
        .unwrap_err();
    assert!(
        matches!(
            wrong_identity,
            foks_client::Error::CredentialBinding(_) | foks_client::Error::UserBinding(_)
        ),
        "unexpected error for a mismatched Yubi parent: {wrong_identity}"
    );

    // The actor-driven entry point accepts a caller-supplied transport
    // projection instead of authenticating one itself, so it must re-bind the
    // credential to that projection rather than trust it.
    pair.local.client.foks().clear_connection_pool().unwrap();
    let local_transport = pair
        .local
        .client
        .foks()
        .authenticate_yubi_and_pin(pair.local.host(), &local_yubi)
        .unwrap();
    let remote_transport = pair
        .remote
        .client
        .foks()
        .authenticate_and_pin(pair.remote.host(), &pair.remote_account.credential)
        .unwrap();
    let supplied_transport = pair
        .local
        .client
        .foks()
        .refresh_federated_team_capability_with_actors(
            &FederatedTeamRefreshRequest {
                remote_host: pair.remote.host(),
                remote_credential: FederationCredential::Software(&pair.remote_account.credential),
                remote_team: &pair.remote_team,
                local_host: pair.local.host(),
                local_credential: FederationCredential::Yubi(&impostor),
                local_team: &pair.local_team,
            },
            &remote_transport.verified,
            None,
            &local_transport.verified,
            None,
        )
        .unwrap_err();
    assert!(matches!(
        supplied_transport,
        foks_client::Error::CredentialBinding(
            "Yubi credential is not enrolled in the verified user chain"
        )
    ));

    // A transport projection from the wrong user must be rejected even when
    // the credential itself is genuine.
    let crossed_transport = pair
        .local
        .client
        .foks()
        .refresh_federated_team_capability_with_actors(
            &FederatedTeamRefreshRequest {
                remote_host: pair.remote.host(),
                remote_credential: FederationCredential::Software(&pair.remote_account.credential),
                remote_team: &pair.remote_team,
                local_host: pair.local.host(),
                local_credential: FederationCredential::Yubi(&local_yubi),
                local_team: &pair.local_team,
            },
            &remote_transport.verified,
            None,
            &remote_transport.verified,
            None,
        )
        .unwrap_err();
    assert!(matches!(
        crossed_transport,
        foks_client::Error::UserBinding(
            "transport user does not match the credential and pinned host"
        )
    ));

    // Both sides on one host is not a federation and must fail before any
    // grant is issued.
    let swapped_hosts = pair
        .local
        .client
        .foks()
        .refresh_federated_team_capability_with_actors(
            &FederatedTeamRefreshRequest {
                remote_host: pair.local.host(),
                remote_credential: FederationCredential::Yubi(&local_yubi),
                remote_team: &pair.remote_team,
                local_host: pair.local.host(),
                local_credential: FederationCredential::Yubi(&local_yubi),
                local_team: &pair.local_team,
            },
            &local_transport.verified,
            None,
            &local_transport.verified,
            None,
        )
        .unwrap_err();
    assert!(matches!(
        swapped_hosts,
        foks_client::Error::TeamRequest("federation refresh hosts or parties are invalid")
    ));

    // A user with no administrative authority over the exported team cannot
    // renew its bearer, whatever credential form it presents.
    let outsider = TestClient::new(&pair.remote.environment, "federation-yubi-outsider").unwrap();
    let outsider_host = outsider.probe_and_pin().unwrap().pinned;
    let outsider_account = outsider
        .create_account(&outsider_host, &TestAccountSpec::new("fedyubiout", 0x79))
        .unwrap();
    let no_authority = pair
        .local
        .client
        .foks()
        .refresh_federated_team_capability(&FederatedTeamRefreshRequest {
            remote_host: pair.remote.host(),
            remote_credential: FederationCredential::Software(&outsider_account.credential),
            remote_team: &pair.remote_team,
            local_host: pair.local.host(),
            local_credential: FederationCredential::Yubi(&local_yubi),
            local_team: &pair.local_team,
        })
        .unwrap_err();
    assert!(
        !matches!(no_authority, foks_client::Error::TeamRequest(_)),
        "an unauthorized grantor must fail on authority, not request shape: {no_authority}"
    );
}
