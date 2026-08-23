use foks_proto::{
    AdHocTeamCreateArgument, DeviceLabel, DeviceLabelNameAndCommitmentKey, EntityId, HostConfig,
    InviteCode, ProvisionDeviceArgument, PukParcel, RevokeDeviceArgument, Role, SecretSeed,
    SharedKeyBoxSet, SoftwareSignupArgument, TeamChain, TeamViewChallenge, TeamViewRequest,
    UserChain, UserLink, UsernameReservation, ViewershipMode, TEAM_VIEW_CHALLENGE_TYPE_ID,
};
use foks_rpc::{
    encode_activate_team_view_request, encode_create_adhoc_team_request,
    encode_get_client_cert_chain_request, encode_get_client_cert_chain_request_at,
    encode_get_current_merkle_root_request, encode_get_historical_merkle_roots_request,
    encode_get_host_config_request, encode_get_owner_puk_request, encode_get_puk_for_role_request,
    encode_load_team_chain_request, encode_load_user_chain_request,
    encode_merkle_select_vhost_request, encode_provision_device_request,
    encode_registration_select_vhost_request, encode_reserve_username_request_at,
    encode_revoke_device_request, encode_signup_request_at, encode_team_view_challenge_request,
};
use foks_snowpack::{decode, encode, Value};

const DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";
const SIGNUP_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/signup";
const MUTATION_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{name}")).unwrap()
}

fn signup_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{SIGNUP_DIR}/{name}")).unwrap()
}

fn mutation_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{MUTATION_DIR}/{name}")).unwrap()
}

#[test]
fn user_mutation_requests_match_go_v019() {
    let chain = UserChain::decode(&fixture("user-chain.snowp")).unwrap();
    let boxes = SharedKeyBoxSet::decode(&mutation_fixture("box-set.snowp")).unwrap();
    let provision = UserLink::decode(&fixture("user-provision-link.snowp")).unwrap();
    let actual = encode_provision_device_request(&ProvisionDeviceArgument {
        link: &provision,
        puk_boxes: &boxes,
        device_name: &chain.device_names[1],
        next_tree_location: chain.locations[1],
        self_token: mutation_fixture("self-token.bin").try_into().unwrap(),
        hepks: &chain.hepks,
    })
    .unwrap();
    let expected = mutation_fixture("provision-request.frame");
    let first_difference = actual.iter().zip(&expected).position(|(a, b)| a != b);
    assert!(
        actual == expected,
        "provision frame differs at {first_difference:?}; lengths are {} and {}",
        actual.len(),
        expected.len()
    );
    let revoke = UserLink::decode(&fixture("user-revoke-link.snowp")).unwrap();
    let parcel = PukParcel::decode(&fixture("puk-parcel.snowp")).unwrap();
    assert_eq!(
        encode_revoke_device_request(&RevokeDeviceArgument {
            link: &revoke,
            puk_boxes: &boxes,
            seed_chain: &parcel.seed_chain,
            next_tree_location: chain.locations[2],
            hepks: &chain.hepks,
        })
        .unwrap(),
        mutation_fixture("revoke-request.frame")
    );
    let rotation = UserLink::decode(&mutation_fixture("rotation-link.snowp")).unwrap();
    let rotation_seed = SecretSeed::new(
        mutation_fixture("rotation-puk-seed.bin")
            .try_into()
            .unwrap(),
    );
    let rotation_hepk =
        foks_crypto::derive_shared_public(&rotation_seed, foks_proto::ENTITY_PUK_VERIFY)
            .unwrap()
            .hepk;
    assert_eq!(
        encode_revoke_device_request(&RevokeDeviceArgument {
            link: &rotation,
            puk_boxes: &boxes,
            seed_chain: &parcel.seed_chain,
            next_tree_location: mutation_fixture("rotation-next-tree-location.bin")
                .try_into()
                .unwrap(),
            hepks: &[rotation_hepk],
        })
        .unwrap(),
        mutation_fixture("rotation-request.frame")
    );
}

#[test]
fn adhoc_team_creation_request_matches_go_v019() {
    let link = UserLink::decode(&mutation_fixture("adhoc-team-link.snowp")).unwrap();
    let membership = UserLink::decode(&mutation_fixture("adhoc-membership-link.snowp")).unwrap();
    let boxes = SharedKeyBoxSet::decode(&mutation_fixture("adhoc-box-set.snowp")).unwrap();
    let hepks = [
        "adhoc-ptk-member-min-seed.bin",
        "adhoc-ptk-member-seed.bin",
        "adhoc-ptk-admin-seed.bin",
        "adhoc-ptk-owner-seed.bin",
    ]
    .map(|name| {
        let seed = SecretSeed::new(mutation_fixture(name).try_into().unwrap());
        foks_crypto::derive_shared_public(&seed, foks_proto::ENTITY_PTK_VERIFY)
            .unwrap()
            .hepk
    });
    let actual = encode_create_adhoc_team_request(&AdHocTeamCreateArgument {
        link: &link,
        next_tree_location: mutation_fixture("adhoc-next-tree-location.bin")
            .try_into()
            .unwrap(),
        ptk_boxes: &boxes,
        hepks: &hepks,
        subchain_tree_location: mutation_fixture("adhoc-subchain-tree-location.bin")
            .try_into()
            .unwrap(),
        membership_link: &membership,
        membership_next_tree_location: mutation_fixture("adhoc-membership-next-tree-location.bin")
            .try_into()
            .unwrap(),
    })
    .unwrap();
    assert_eq!(actual, mutation_fixture("adhoc-create-request.frame"));
}

#[test]
fn host_config_request_matches_go_v019() {
    assert_eq!(
        encode_get_host_config_request().unwrap(),
        mutation_fixture("host-config-request.frame")
    );
    let config = HostConfig::decode(&mutation_fixture("host-config-open.snowp")).unwrap();
    assert_eq!(config.user_viewership, ViewershipMode::Open);
    assert_eq!(config.team_viewership, ViewershipMode::OpenToAdmin);
    assert_eq!(config.host_type, 4);
    assert_eq!(config.invite_code_regime, 2);
}

#[test]
fn host_config_rejects_unknown_policy_enums() {
    let config = |user_viewership, host_type, invite_code_regime| {
        encode(&Value::Array(vec![
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(false),
            ]),
            Value::Array(vec![Value::Unsigned(user_viewership), Value::Unsigned(0)]),
            Value::Unsigned(host_type),
            Value::Unsigned(invite_code_regime),
        ]))
        .unwrap()
    };
    assert!(HostConfig::decode(&config(3, 4, 2)).is_err());
    assert!(HostConfig::decode(&config(2, 5, 2)).is_err());
    assert!(HostConfig::decode(&config(2, 4, 4)).is_err());
}

#[test]
fn software_signup_requests_match_go_v019() {
    assert_eq!(
        encode_reserve_username_request_at(b"signupfixture", 1).unwrap(),
        signup_fixture("reserve-request.frame")
    );
    let reservation = UsernameReservation::decode(&signup_fixture("reservation.snowp")).unwrap();
    let link = UserLink::decode(&signup_fixture("eldest-link.snowp")).unwrap();
    let puk_box = SharedKeyBoxSet::decode(&signup_fixture("puk-box-set.snowp")).unwrap();
    let device_seed = SecretSeed::new(signup_fixture("device-seed.bin").try_into().unwrap());
    let puk_seed = SecretSeed::new(signup_fixture("puk-seed.bin").try_into().unwrap());
    let device = foks_crypto::derive_device_public(&device_seed).unwrap();
    let puk = foks_crypto::derive_shared_public(&puk_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let argument = SoftwareSignupArgument {
        username_utf8: b"signupfixture",
        reservation: &reservation,
        link: &link,
        puk_box: &puk_box,
        username_commitment_key: signup_fixture("username-commitment-key.bin")
            .try_into()
            .unwrap(),
        device_name: &DeviceLabelNameAndCommitmentKey {
            label: DeviceLabel {
                device_type: 0,
                normalized_name: b"signup device".to_vec(),
                serial: 1,
            },
            normalization_version: 0,
            display_name: b"signup device".to_vec(),
            commitment_key: signup_fixture("device-commitment-key.bin")
                .try_into()
                .unwrap(),
        },
        next_tree_location: signup_fixture("next-tree-location.bin").try_into().unwrap(),
        invite_code: &InviteCode::Empty,
        email: b"fixture@example.com",
        subchain_tree_location: signup_fixture("subchain-tree-location.bin")
            .try_into()
            .unwrap(),
        self_token: signup_fixture("self-token.bin").try_into().unwrap(),
        puk_hepk: &puk.hepk,
        device_hepk: &device.hepk,
    };
    let actual = encode_signup_request_at(&argument, 1).unwrap();
    let expected = signup_fixture("signup-request.frame");
    let first_difference = actual
        .iter()
        .zip(&expected)
        .position(|(actual, expected)| actual != expected);
    assert!(
        actual == expected,
        "signup frame differs at {first_difference:?}; lengths are {} and {}",
        actual.len(),
        expected.len()
    );
}

#[test]
fn merkle_query_requests_match_the_official_go_oracle() {
    let host = TeamChain::decode(&fixture("team-chain.snowp"))
        .unwrap()
        .links[0]
        .decode_team_group_change()
        .unwrap()
        .host;
    assert_eq!(
        encode_merkle_select_vhost_request(&host).unwrap(),
        fixture("merkle-select-vhost-request.frame")
    );
    assert_eq!(
        encode_get_current_merkle_root_request(&host, 1).unwrap(),
        fixture("merkle-current-root-request.frame")
    );
    assert_eq!(
        encode_get_historical_merkle_roots_request(&host, &[996], &[997, 996, 994, 992], 1,)
            .unwrap(),
        fixture("merkle-historical-roots-request.frame")
    );
}

fn binary_fixture(name: &str) -> Vec<u8> {
    match decode(&fixture(name)).unwrap() {
        Value::Binary(bytes) => bytes,
        other => panic!("expected binary fixture, got {other:?}"),
    }
}

#[test]
fn registration_certificate_request_matches_go_v019() {
    let uid = binary_fixture("uid.snowp");
    let device = binary_fixture("device-id.snowp");
    let host = TeamChain::decode(&fixture("team-chain.snowp"))
        .unwrap()
        .links[0]
        .decode_team_group_change()
        .unwrap()
        .host;
    assert_eq!(
        encode_registration_select_vhost_request(&host).unwrap(),
        fixture("reg-select-vhost-request.frame")
    );
    assert_eq!(
        encode_get_client_cert_chain_request_at(&uid, &device, 1).unwrap(),
        fixture("reg-cert-request.frame")
    );
    assert!(encode_get_client_cert_chain_request(&uid, &device).is_ok());
}

#[test]
fn user_chain_request_matches_go_v019() {
    let uid = binary_fixture("uid.snowp");
    assert_eq!(
        encode_load_user_chain_request(&uid, 1).unwrap(),
        fixture("user-load-request.frame")
    );
}

#[test]
fn owner_puk_request_matches_go_v019() {
    let device = binary_fixture("device-id.snowp");
    assert_eq!(
        encode_get_owner_puk_request(&device).unwrap(),
        fixture("user-puk-request.frame")
    );
    assert_eq!(
        encode_get_puk_for_role_request(Role::member(7), &device).unwrap(),
        fixture("user-member-puk-request.frame")
    );
}

#[test]
fn team_view_and_load_requests_match_go_v019() {
    let uid = EntityId::from_bytes(binary_fixture("uid.snowp")).unwrap();
    let team = EntityId::from_bytes(binary_fixture("team-id.snowp")).unwrap();
    let chain = TeamChain::decode(&fixture("team-chain.snowp")).unwrap();
    let host = chain.links[0].decode_team_group_change().unwrap().host;
    let request = TeamViewRequest {
        team: team.clone(),
        host: host.clone(),
        member: uid,
        member_host: host.clone(),
        source_role: Role::OWNER,
        generation: 2,
    };
    assert_eq!(
        encode_team_view_challenge_request(&request).unwrap(),
        fixture("team-view-challenge-request.frame")
    );

    let challenge = TeamViewChallenge::decode(&fixture("team-view-challenge.snowp")).unwrap();
    assert_eq!(challenge.request, request);
    let seed = SecretSeed::new(fixture("puk-seed.bin").try_into().unwrap());
    let signature = foks_crypto::sign_shared_key_typed(
        &seed,
        TEAM_VIEW_CHALLENGE_TYPE_ID,
        &challenge.encoded().unwrap(),
    )
    .unwrap();
    assert_eq!(
        encode_activate_team_view_request(&challenge, &signature).unwrap(),
        fixture("team-view-activate-request.frame")
    );
    assert_eq!(
        encode_load_team_chain_request(&team, &host, &challenge.token, 1).unwrap(),
        fixture("team-load-request.frame")
    );
}
