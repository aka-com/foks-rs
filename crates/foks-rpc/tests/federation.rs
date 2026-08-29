use foks_proto::{
    EntityId, FqParty, FqTeam, PermissionToken, RemoteViewPermissionPayload, Role, SecretBox,
    Signature, TeamRemoteMemberViewTokenBoxPayload, TeamRemoteMemberViewTokenInner,
    TeamRemoteViewTokenSet, ENTITY_HOST, ENTITY_NAMED_TEAM, ENTITY_USER,
};
use foks_rpc::{
    encode_beacon_lookup_request, encode_grant_remote_view_permission_for_team_request,
    encode_grant_remote_view_permission_for_user_request, encode_load_remote_team_chain_request,
    encode_load_remote_user_chain_request, encode_load_team_remote_view_tokens_request,
};

fn entity(kind: u8, fill: u8) -> EntityId {
    EntityId::from_bytes([vec![kind], vec![fill; 32]].concat()).unwrap()
}

fn hex(input: &str) -> Vec<u8> {
    assert_eq!(input.len() % 2, 0);
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |value: u8| match value {
                b'0'..=b'9' => value - b'0',
                b'a'..=b'f' => value - b'a' + 10,
                _ => panic!("invalid fixture hex"),
            };
            digit(pair[0]) << 4 | digit(pair[1])
        })
        .collect()
}

#[test]
fn federation_requests_match_the_go_v019_oracle() {
    let remote_host = entity(ENTITY_HOST, 0x22);
    let viewee = entity(ENTITY_USER, 0x33);
    let viewer = FqParty::new(entity(ENTITY_USER, 0x44), remote_host.clone()).unwrap();
    let token = PermissionToken::new(std::array::from_fn(|index| 0x50 + index as u8));
    let payload = RemoteViewPermissionPayload::new(viewee.clone(), viewer, 123_456_789).unwrap();

    assert_eq!(
        encode_beacon_lookup_request(&remote_host).unwrap(),
        hex("48950500cebe314f3c0282a44461746191c421022222222222222222222222222222222222222222222222222222222222222222a648656164657282a15601a2663181a45665727301")
    );
    assert_eq!(
        encode_load_remote_user_chain_request(&viewee, 1, None, &token).unwrap(),
        hex("63950500cef7ab85f30b82a4446174619194c42101333333333333333333333333333333333333333333333333333333333333333301c0920181a131c411505152535455565758595a5b5c5d5e5f60a648656164657282a15601a2663181a45665727301")
    );
    assert_eq!(
        encode_grant_remote_view_permission_for_user_request(&payload).unwrap(),
        hex("cc95950500ce823f08991282a4446174619193c42101333333333333333333333333333333333333333333333333333333333333333392c421014444444444444444444444444444444444444444444444444444444444444444c421022222222222222222222222222222222222222222222222222222222222222222ce075bcd15a648656164657282a15601a2663181a45665727301")
    );
    assert_eq!(
        encode_grant_remote_view_permission_for_team_request(
            &payload,
            &Signature::Ed25519(std::array::from_fn(|index| index as u8)),
            1,
            Role::OWNER,
        )
        .unwrap(),
        hex("cce1950500cebda3b9d30182a4446174619293c42101333333333333333333333333333333333333333333333333333333333333333392c421014444444444444444444444444444444444444444444444444444444444444444c421022222222222222222222222222222222222222222222222222222222222222222ce075bcd1593920081a130c440000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f01920380a648656164657282a15601a2663181a45665727301")
    );

    let team = entity(ENTITY_NAMED_TEAM, 0x66);
    assert_eq!(
        encode_load_remote_team_chain_request(&team, &remote_host, &token, 1, None).unwrap(),
        hex("cc89950500cef91285790382a4446174619792c421036666666666666666666666666666666666666666666666666666666666666666c421022222222222222222222222222222222222222222222222222222222222222222920281a131c411505152535455565758595a5b5c5d5e5f6001c0c0c2c2a648656164657282a15601a2663181a45665727301")
    );
    let member = FqParty::new(entity(ENTITY_USER, 0x44), remote_host.clone()).unwrap();
    let boxed_payload = TeamRemoteMemberViewTokenBoxPayload {
        token: token.clone(),
        party: member.clone(),
        time: 123_456_789,
    };
    assert_eq!(
        boxed_payload.encoded().unwrap(),
        hex("93c411505152535455565758595a5b5c5d5e5f6092c421014444444444444444444444444444444444444444444444444444444444444444c421022222222222222222222222222222222222222222222222222222222222222222ce075bcd15")
    );
    let inner = TeamRemoteMemberViewTokenInner {
        member: member.clone(),
        ptk_generation: 1,
        secret_box: SecretBox {
            nonce: std::array::from_fn(|index| 0x80 + index as u8),
            ciphertext: vec![0x90, 0x91, 0x92, 0x93],
        },
        ptk_role: Role::member(0),
    };
    assert_eq!(
        TeamRemoteViewTokenSet {
            tokens: vec![inner],
        }
        .encoded()
        .unwrap(),
        hex("91919492c421014444444444444444444444444444444444444444444444444444444444444444c42102222222222222222222222222222222222222222222222222222222222222222201920081a13092c410808182838485868788898a8b8c8d8e8fc40490919293920181a13000")
    );
    let view_token = std::array::from_fn(|index| 0x70 + index as u8);
    assert_eq!(
        encode_load_team_remote_view_tokens_request(
            &FqTeam::new(team, remote_host).unwrap(),
            &view_token,
            &[member],
        )
        .unwrap(),
        hex("ccc6950500cef91285790682a4446174619392c421036666666666666666666666666666666666666666666666666666666666666666c421022222222222222222222222222222222222222222222222222222222222222222c410707172737475767778797a7b7c7d7e7f9192c421014444444444444444444444444444444444444444444444444444444444444444c421022222222222222222222222222222222222222222222222222222222222222222a648656164657282a15601a2663181a45665727301")
    );
}

#[test]
fn federation_encoders_reject_invalid_authority_scopes() {
    let user = entity(ENTITY_USER, 1);
    let host = entity(ENTITY_HOST, 2);
    let token = PermissionToken::new([3; 17]);
    assert!(encode_beacon_lookup_request(&user).is_err());
    assert!(encode_load_remote_user_chain_request(&user, 0, None, &token).is_err());
    let payload =
        RemoteViewPermissionPayload::new(user.clone(), FqParty::new(user, host).unwrap(), 1)
            .unwrap();
    assert!(encode_grant_remote_view_permission_for_team_request(
        &payload,
        &Signature::Ed25519([0; 64]),
        0,
        Role::NONE,
    )
    .is_err());
}
