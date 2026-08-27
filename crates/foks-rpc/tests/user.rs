use foks_proto::{
    AdHocTeamCreateArgument, AddTeamMemberArgument, DecodedSoftwareSignupArgument, DeviceLabel,
    DeviceLabelNameAndCommitmentKey, DeviceType, EntityId, HostConfig, InviteCode,
    NamedTeamCreateArgument, ProvisionDeviceArgument, PukParcel, RegistrationChallenge,
    RemoveTeamMemberArgument, RevokeDeviceArgument, Role, SecretSeed, SeedChainBox,
    SharedKeyBoxSet, SoftwareSignupArgument, TeamBearerTokenChallenge, TeamChain,
    TeamRemovalAndCommitment, TeamRemovalBoxData, TeamViewChallenge, TeamViewRequest, UserChain,
    UserLink, UsernameReservation, ViewershipMode, TEAM_VIEW_CHALLENGE_TYPE_ID,
};
use foks_rpc::{
    decode_team_bearer_token, decode_team_edit_result, decode_team_removal_key_box,
    encode_activate_team_bearer_token_request, encode_activate_team_view_request,
    encode_add_team_member_request, encode_create_adhoc_team_request,
    encode_create_named_team_request, encode_get_client_cert_chain_request,
    encode_get_client_cert_chain_request_at, encode_get_current_merkle_root_request,
    encode_get_historical_merkle_roots_request, encode_get_host_config_request,
    encode_get_owner_puk_request, encode_get_puk_for_role_request,
    encode_get_uid_lookup_challenge_request, encode_load_team_chain_request,
    encode_load_team_removal_key_box_request, encode_load_user_chain_request,
    encode_lookup_uid_by_device_request, encode_make_team_bearer_token_request,
    encode_merkle_select_vhost_request, encode_provision_device_request,
    encode_registration_select_vhost_request, encode_remove_team_member_request,
    encode_reserve_team_name_request, encode_reserve_username_request_at,
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
fn named_team_requests_match_go_v019() {
    assert_eq!(
        encode_reserve_team_name_request(b"auditteam").unwrap(),
        mutation_fixture("named-reserve-request.frame")
    );
    let reservation =
        UsernameReservation::decode(&mutation_fixture("named-reservation.snowp")).unwrap();
    let link = UserLink::decode(&mutation_fixture("named-team-link.snowp")).unwrap();
    let membership = UserLink::decode(&mutation_fixture("named-membership-link.snowp")).unwrap();
    let boxes = SharedKeyBoxSet::decode(&mutation_fixture("adhoc-box-set.snowp")).unwrap();
    let removal =
        TeamRemovalBoxData::decode(&mutation_fixture("named-removal-boxes.snowp")).unwrap();
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
    let actual = encode_create_named_team_request(&NamedTeamCreateArgument {
        name_utf8: b"AuditTeam",
        team_name_commitment_key: mutation_fixture("named-team-name-commitment-key.bin")
            .try_into()
            .unwrap(),
        subchain_tree_location: mutation_fixture("named-subchain-tree-location.bin")
            .try_into()
            .unwrap(),
        reservation: &reservation,
        link: &link,
        next_tree_location: mutation_fixture("named-next-tree-location.bin")
            .try_into()
            .unwrap(),
        ptk_boxes: &boxes,
        removal_keys: &[removal],
        hepks: &hepks,
        membership_link: &membership,
        membership_next_tree_location: mutation_fixture("named-membership-next-tree-location.bin")
            .try_into()
            .unwrap(),
    })
    .unwrap();
    assert_eq!(actual, mutation_fixture("named-create-request.frame"));
}

#[test]
fn additive_team_edit_matches_go_v019() {
    let link = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
    let boxes = SharedKeyBoxSet::decode(&mutation_fixture("add-member-ptk-boxes.snowp")).unwrap();
    let removal =
        TeamRemovalBoxData::decode(&mutation_fixture("add-member-removal-boxes.snowp")).unwrap();
    let target_seed = SecretSeed::new(
        mutation_fixture("add-member-target-puk-seed.bin")
            .try_into()
            .unwrap(),
    );
    let target =
        foks_crypto::derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let target_id = match decode(&mutation_fixture("add-member-target-uid.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        other => panic!("expected target UID fixture, got {other:?}"),
    };
    let actual = encode_add_team_member_request(&AddTeamMemberArgument {
        link: &link,
        next_tree_location: mutation_fixture("add-member-next-tree-location.bin")
            .try_into()
            .unwrap(),
        ptk_boxes: &boxes,
        removal_keys: &[removal],
        hepks: std::slice::from_ref(&target.hepk),
        local_permissions_for: std::slice::from_ref(&target_id),
    })
    .unwrap();
    assert_eq!(actual, mutation_fixture("add-member-request.frame"));
    assert!(AddTeamMemberArgument {
        link: &link,
        next_tree_location: [0; 32],
        ptk_boxes: &boxes,
        removal_keys: &[],
        hepks: std::slice::from_ref(&target.hepk),
        local_permissions_for: std::slice::from_ref(&target_id),
    }
    .encoded()
    .is_err());
    assert!(
        decode_team_edit_result(&mutation_fixture("add-member-edit-result.snowp"))
            .unwrap()
            .local_invitees
            .is_empty()
    );
}

#[test]
fn removal_and_ptk_rotation_edit_matches_go_v019() {
    let link = UserLink::decode(&mutation_fixture("remove-member-link.snowp")).unwrap();
    let boxes =
        SharedKeyBoxSet::decode(&mutation_fixture("remove-member-ptk-boxes.snowp")).unwrap();
    let seed_chain = [
        SeedChainBox::decode(&mutation_fixture(
            "remove-member-seed-chain-member-min.snowp",
        ))
        .unwrap(),
        SeedChainBox::decode(&mutation_fixture("remove-member-seed-chain-member.snowp")).unwrap(),
    ];
    let removal =
        TeamRemovalAndCommitment::decode(&mutation_fixture("remove-member-proof.snowp")).unwrap();
    assert_eq!(
        TeamRemovalAndCommitment::decode(&removal.encoded().unwrap()).unwrap(),
        removal
    );
    for boxed in &seed_chain {
        assert_eq!(
            SeedChainBox::decode(&boxed.encoded().unwrap()).unwrap(),
            boxed.clone()
        );
    }
    let hepks = [
        "remove-member-ptk-member-min-seed.bin",
        "remove-member-ptk-member-seed.bin",
    ]
    .map(|name| {
        let seed = SecretSeed::new(mutation_fixture(name).try_into().unwrap());
        foks_crypto::derive_shared_public(&seed, foks_proto::ENTITY_PTK_VERIFY)
            .unwrap()
            .hepk
    });
    let argument = RemoveTeamMemberArgument {
        link: &link,
        next_tree_location: mutation_fixture("remove-member-next-tree-location.bin")
            .try_into()
            .unwrap(),
        ptk_boxes: &boxes,
        seed_chain: &seed_chain,
        removals: &[removal],
        hepks: &hepks,
    };
    assert_eq!(
        encode_remove_team_member_request(&argument).unwrap(),
        mutation_fixture("remove-member-request.frame")
    );
    assert!(RemoveTeamMemberArgument {
        link: &link,
        next_tree_location: [0; 32],
        ptk_boxes: &boxes,
        seed_chain: &[],
        removals: &[],
        hepks: &hepks,
    }
    .encoded()
    .is_err());
}

#[test]
fn team_admin_bearer_and_removal_key_requests_match_go_v019() {
    let team = match decode(&mutation_fixture("named-team-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        other => panic!("expected team ID fixture, got {other:?}"),
    };
    let user = match decode(&fixture("uid.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        other => panic!("expected user ID fixture, got {other:?}"),
    };
    let target = match decode(&mutation_fixture("add-member-target-uid.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        other => panic!("expected target user fixture, got {other:?}"),
    };
    let host = UserLink::decode(&mutation_fixture("named-team-link.snowp"))
        .unwrap()
        .decode_team_group_change()
        .unwrap()
        .host;
    assert_eq!(
        encode_make_team_bearer_token_request(&team, Role::OWNER, 1).unwrap(),
        mutation_fixture("team-bearer-make-request.frame")
    );
    let token = decode_team_bearer_token(&mutation_fixture("team-bearer-token.snowp")).unwrap();
    let challenge = TeamBearerTokenChallenge {
        user,
        user_host: host.clone(),
        team,
        role: Role::OWNER,
        generation: 1,
        token,
        time: 1_700_000_000_019,
    };
    assert_eq!(
        challenge.encoded_payload().unwrap(),
        mutation_fixture("team-bearer-challenge-payload.snowp")
    );
    assert_eq!(
        challenge.encoded_blob().unwrap(),
        mutation_fixture("team-bearer-challenge-blob.snowp")
    );
    let seed = SecretSeed::new(
        mutation_fixture("adhoc-ptk-owner-seed.bin")
            .try_into()
            .unwrap(),
    );
    let signature = foks_crypto::sign_team_bearer_token_challenge(&seed, &challenge).unwrap();
    assert_eq!(
        encode(&signature.to_value()).unwrap(),
        mutation_fixture("team-bearer-signature.snowp")
    );
    assert_eq!(
        encode_activate_team_bearer_token_request(&challenge, &signature).unwrap(),
        mutation_fixture("team-bearer-activate-request.frame")
    );
    assert_eq!(
        encode_load_team_removal_key_box_request(&token, &target, &host, Role::OWNER).unwrap(),
        mutation_fixture("team-removal-key-load-request.frame")
    );
    let boxed =
        decode_team_removal_key_box(&mutation_fixture("team-removal-admin-box.snowp")).unwrap();
    assert_eq!(
        boxed.encoded().unwrap(),
        mutation_fixture("team-removal-admin-box.snowp")
    );
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
                device_type: DeviceType::Computer,
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
    let call = foks_rpc::read_call(
        &mut std::io::Cursor::new(expected),
        foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
    )
    .unwrap();
    let decoded = DecodedSoftwareSignupArgument::decode(call.argument()).unwrap();
    assert_eq!(decoded.username_utf8, b"signupfixture");
    assert_eq!(decoded.reservation, reservation);
    assert_eq!(decoded.link, link);
    assert_eq!(decoded.puk_box, puk_box);
    assert_eq!(decoded.invite_code, InviteCode::Empty);
    assert_eq!(decoded.puk_hepk, puk.hepk);
    assert_eq!(decoded.device_hepk, device.hepk);
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
fn backup_lookup_requests_match_go_v019() {
    let backup_id = match decode(&mutation_fixture("backup-entity-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        _ => panic!("backup ID fixture is not a blob"),
    };
    assert_eq!(
        encode_get_uid_lookup_challenge_request(&backup_id).unwrap(),
        mutation_fixture("backup-lookup-challenge-request.frame")
    );
    let seed = mutation_fixture("backup-seed.bin").try_into().unwrap();
    let backup = foks_crypto::BackupKey::from_seed(seed).unwrap();
    let challenge =
        RegistrationChallenge::decode(&mutation_fixture("backup-lookup-challenge.snowp")).unwrap();
    let signature = backup.sign_registration_challenge(&challenge).unwrap();
    assert_eq!(
        encode_lookup_uid_by_device_request(&backup_id, &challenge, &signature).unwrap(),
        mutation_fixture("backup-lookup-request.frame")
    );
    let uid = binary_fixture("uid.snowp");
    assert_eq!(
        encode_get_client_cert_chain_request_at(&uid, backup_id.as_bytes(), 1).unwrap(),
        mutation_fixture("backup-cert-request.frame")
    );
}

#[test]
fn backup_provision_requests_match_go_v019() {
    let backup =
        foks_crypto::BackupKey::from_seed(mutation_fixture("backup-seed.bin").try_into().unwrap())
            .unwrap();
    let backup_name = backup.device_name();
    let enroll_link = UserLink::decode(&mutation_fixture("backup-enroll-link.snowp")).unwrap();
    let enroll_boxes =
        SharedKeyBoxSet::decode(&mutation_fixture("backup-enroll-boxes.snowp")).unwrap();
    let backup_hepk = foks_proto::Hepk::decode(&mutation_fixture("backup-hepk.snowp")).unwrap();
    assert_eq!(
        encode_provision_device_request(&ProvisionDeviceArgument {
            link: &enroll_link,
            puk_boxes: &enroll_boxes,
            device_name: &DeviceLabelNameAndCommitmentKey {
                label: DeviceLabel {
                    device_type: DeviceType::Backup,
                    normalized_name: backup_name.as_bytes().to_vec(),
                    serial: 1,
                },
                normalization_version: 0,
                display_name: backup_name.into_bytes(),
                commitment_key: mutation_fixture("backup-enroll-device-name-commitment-key.bin")
                    .try_into()
                    .unwrap(),
            },
            next_tree_location: mutation_fixture("backup-enroll-next-tree-location.bin")
                .try_into()
                .unwrap(),
            self_token: mutation_fixture("backup-enroll-self-token.bin")
                .try_into()
                .unwrap(),
            hepks: &[backup_hepk],
        })
        .unwrap(),
        mutation_fixture("backup-enroll-request.frame")
    );

    let recover_link = UserLink::decode(&mutation_fixture("backup-recover-link.snowp")).unwrap();
    let recover_boxes =
        SharedKeyBoxSet::decode(&mutation_fixture("backup-recover-boxes.snowp")).unwrap();
    let replacement_seed = SecretSeed::new(
        mutation_fixture("backup-recover-device-seed.bin")
            .try_into()
            .unwrap(),
    );
    let replacement = foks_crypto::derive_device_public(&replacement_seed).unwrap();
    assert_eq!(
        encode_provision_device_request(&ProvisionDeviceArgument {
            link: &recover_link,
            puk_boxes: &recover_boxes,
            device_name: &DeviceLabelNameAndCommitmentKey {
                label: DeviceLabel {
                    device_type: DeviceType::Computer,
                    normalized_name: b"recovered fixture device".to_vec(),
                    serial: 1,
                },
                normalization_version: 0,
                display_name: b"Recovered Fixture Device".to_vec(),
                commitment_key: mutation_fixture("backup-recover-device-name-commitment-key.bin")
                    .try_into()
                    .unwrap(),
            },
            next_tree_location: mutation_fixture("backup-recover-next-tree-location.bin")
                .try_into()
                .unwrap(),
            self_token: mutation_fixture("backup-recover-self-token.bin")
                .try_into()
                .unwrap(),
            hepks: &[replacement.hepk],
        })
        .unwrap(),
        mutation_fixture("backup-recover-request.frame")
    );
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
fn incremental_chain_requests_carry_go_v019_name_cursors() {
    let uid = binary_fixture("uid.snowp");
    let user_argument = encode(&Value::Array(vec![Value::Array(vec![
        Value::Binary(uid.clone()),
        Value::Unsigned(4),
        Value::Array(vec![
            Value::Text(b"fixtureuser".to_vec()),
            Value::Unsigned(2),
        ]),
        Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
    ])]))
    .unwrap();
    assert_eq!(
        foks_rpc::encode_load_user_chain_request_from(&uid, 4, Some((b"fixtureuser", 2))).unwrap(),
        foks_rpc::encode_call(
            foks_rpc::USER_PROTOCOL_ID,
            foks_rpc::USER_LOAD_USER_CHAIN_METHOD_POSITION,
            &user_argument,
            0,
        )
        .unwrap()
    );

    let team = EntityId::from_bytes(binary_fixture("team-id.snowp")).unwrap();
    let chain = TeamChain::decode(&fixture("team-chain.snowp")).unwrap();
    let host = chain.links[0].decode_team_group_change().unwrap().host;
    let token = [0x5a; 16];
    let team_argument = encode(&Value::Array(vec![
        Value::Array(vec![
            Value::Binary(team.as_bytes().to_vec()),
            Value::Binary(host.as_bytes().to_vec()),
        ]),
        Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"0".to_vec(),
                Box::new(Value::Binary(token.to_vec())),
            ))),
        ]),
        Value::Unsigned(2),
        Value::Array(vec![
            Value::Text(b"fixtureteam".to_vec()),
            Value::Unsigned(2),
        ]),
        Value::Null,
        Value::Bool(false),
        Value::Bool(false),
    ]))
    .unwrap();
    assert_eq!(
        foks_rpc::encode_load_team_chain_request_from(
            &team,
            &host,
            &token,
            2,
            Some((b"fixtureteam", 2)),
        )
        .unwrap(),
        foks_rpc::encode_call(
            foks_rpc::TEAM_LOADER_PROTOCOL_ID,
            foks_rpc::TEAM_LOAD_CHAIN_METHOD_POSITION,
            &team_argument,
            0,
        )
        .unwrap()
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
