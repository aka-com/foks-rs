use foks_proto::{
    ApprovedMembershipLinkPublic, GenericLinkPayload, HostchainTail, Role, TeamChain,
    TeamMembershipPayload, TeamMembershipState, UnsignedUserLink, UserLink, ENTITY_NAMED_TEAM,
};
use foks_verify::{
    verify_merkle_advance, verify_public_host, verify_team_chain, VerifiedTeamState,
};

use super::{validate_membership_against_team, AuthenticatedTeamMembershipEvent};

fn fixture(directory: &str, name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../foks-snowpack/tests/fixtures/foks-v0.1.9/{directory}/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn verified_team() -> VerifiedTeamState {
    let public =
        verify_public_host("foks.app", &fixture("foks.app", "probe-response.snowp")).unwrap();
    let advance = verify_merkle_advance(
        public.snapshot.merkle_root(),
        &fixture("user", "team-merkle-root-996.snowp"),
        &fixture("user", "team-merkle-historical-response.snowp"),
        &HostchainTail {
            seqno: public.snapshot.chain_seqno(),
            hash: public.snapshot.chain_tail_hash(),
        },
    )
    .unwrap();
    let bytes = fixture("user", "team-chain.snowp");
    let chain = TeamChain::decode(&bytes).unwrap();
    let eldest = chain.links[0].decode_team_group_change().unwrap();
    verify_team_chain(
        &bytes,
        &eldest.team,
        &eldest.host,
        advance.authenticated_roots(),
        &advance,
    )
    .unwrap()
}

#[test]
fn go_creator_zero_sequence_retains_exact_signed_bytes_and_outbound_stays_strict() {
    let exact = fixture("user-mutations", "go-creator-zero-membership-link.snowp");
    let link = UserLink::decode(&exact).unwrap();
    let decoded = link.decode_generic().unwrap();
    let GenericLinkPayload::TeamMembership(membership) = &decoded.payload else {
        panic!("expected membership");
    };
    let TeamMembershipState::Approved {
        destination_role,
        team_sequence,
        removal_key_commitment,
    } = membership.state
    else {
        panic!("expected named approval");
    };
    assert_eq!(team_sequence, 0);
    assert_eq!(link.encoded().unwrap(), exact);
    foks_crypto::verify_typed(
        &decoded.signer,
        &link.signatures()[0],
        foks_proto::LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(0).unwrap(),
    )
    .unwrap();
    assert!(
        UnsignedUserLink::approved_membership(&ApprovedMembershipLinkPublic {
            user: &decoded.entity,
            host: &decoded.host,
            signer: &decoded.signer,
            sequence: decoded.sequence,
            previous: decoded.previous,
            root: &decoded.root,
            time: decoded.time,
            next_location_commitment: decoded.next_location_commitment,
            team: &membership.team,
            source_role: membership.source_role,
            destination_role,
            team_sequence,
            removal_key_commitment,
        })
        .is_err()
    );
}

#[test]
fn zero_sequence_requires_the_authenticated_founding_owner_and_commitment() {
    let team = verified_team();
    let founder = &team.members()[0];
    let event = AuthenticatedTeamMembershipEvent {
        sequence: 3,
        membership: TeamMembershipPayload {
            team: team.team().clone(),
            team_host: team.host().clone(),
            source_role: Role::OWNER,
            state: TeamMembershipState::Approved {
                destination_role: Role::OWNER,
                team_sequence: 0,
                removal_key_commitment: founder.removal_key_commitment.unwrap(),
            },
        },
    };
    assert!(validate_membership_against_team(&founder.party, &event, &team).unwrap());
    assert!(matches!(
        event.membership.state,
        TeamMembershipState::Approved {
            team_sequence: 0,
            ..
        }
    ));

    let mut wrong = event.clone();
    wrong.membership.team = different_entity(team.team());
    assert!(validate_membership_against_team(&founder.party, &wrong, &team).is_err());
    wrong = event.clone();
    wrong.membership.team_host = different_entity(team.host());
    assert!(validate_membership_against_team(&founder.party, &wrong, &team).is_err());
    assert!(
        validate_membership_against_team(&different_entity(&founder.party), &event, &team).is_err()
    );
    wrong = event.clone();
    wrong.membership.source_role = Role::ADMIN;
    assert!(validate_membership_against_team(&founder.party, &wrong, &team).is_err());
    wrong = event.clone();
    if let TeamMembershipState::Approved {
        destination_role, ..
    } = &mut wrong.membership.state
    {
        *destination_role = Role::ADMIN;
    }
    assert!(validate_membership_against_team(&founder.party, &wrong, &team).is_err());
    wrong = event.clone();
    if let TeamMembershipState::Approved {
        removal_key_commitment,
        ..
    } = &mut wrong.membership.state
    {
        removal_key_commitment[0] ^= 1;
    }
    assert!(validate_membership_against_team(&founder.party, &wrong, &team).is_err());

    let mut team_owner = founder.party.as_bytes().to_vec();
    team_owner[0] = ENTITY_NAMED_TEAM;
    assert!(validate_membership_against_team(
        &foks_proto::EntityId::from_bytes(team_owner).unwrap(),
        &event,
        &team
    )
    .is_err());
}

fn different_entity(entity: &foks_proto::EntityId) -> foks_proto::EntityId {
    let mut bytes = entity.as_bytes().to_vec();
    bytes[1] ^= 1;
    foks_proto::EntityId::from_bytes(bytes).unwrap()
}

#[test]
fn go_founder_removal_box_opens_with_raw_zero_metadata_but_cannot_be_emitted() {
    use foks_crypto::{
        open_team_removal_key_for_member, SharedKeyDecapsulator, TeamRemovalKeyExpectation,
    };
    use foks_proto::{SecretSeed, TeamRemovalBoxData};
    let exact = fixture("user-mutations", "go-creator-zero-removal-boxes.snowp");
    let boxed = TeamRemovalBoxData::decode(&exact).unwrap();
    let seed = SecretSeed::new(
        fixture("user-mutations", "adhoc-ptk-admin-seed.bin")
            .try_into()
            .unwrap(),
    );
    let receiver = SharedKeyDecapsulator::new(&seed, boxed.metadata.team.clone()).unwrap();
    let expected = TeamRemovalKeyExpectation {
        commitment: &boxed.commitment,
        team: &boxed.metadata.team,
        host: &boxed.metadata.host,
        member: &boxed.metadata.member,
        member_host: &boxed.metadata.member_host,
        source_role: Role::OWNER,
    };
    let (key, metadata) =
        open_team_removal_key_for_member(&boxed.team_box, &receiver, &expected).unwrap();
    assert_eq!(metadata.team_sequence, 0);
    assert_eq!(metadata, boxed.metadata);
    assert_eq!(
        key.as_bytes(),
        fixture("user-mutations", "named-removal-key.bin").as_slice()
    );
    assert!(metadata.encoded().is_err());
    assert!(boxed.encoded().is_err());
    let wrong_commitment = [0; 32];
    assert!(open_team_removal_key_for_member(
        &boxed.team_box,
        &receiver,
        &TeamRemovalKeyExpectation {
            commitment: &wrong_commitment,
            ..expected
        }
    )
    .is_err());
}

#[test]
fn zero_sequence_wire_claims_do_not_accept_nonowner_roles() {
    use foks_snowpack::{decode, encode, Value};
    let exact = fixture("user-mutations", "go-creator-zero-membership-link.snowp");
    for source in [true, false] {
        let mut wire = decode(&exact).unwrap();
        let outer = wire_array(wire_variant(&mut wire_array(&mut wire)[1]));
        let Value::Binary(bytes) = &outer[0] else {
            panic!("inner bytes")
        };
        let mut inner = decode(bytes).unwrap();
        let generic = wire_array(wire_variant(&mut wire_array(&mut inner)[1]));
        let membership = wire_array(wire_variant(&mut wire_array(&mut generic[3])[1]));
        if source {
            membership[1] = Role::ADMIN.to_value();
        } else {
            let approved = wire_array(wire_variant(&mut wire_array(&mut membership[2])[1]));
            wire_array(&mut approved[0])[0] = Role::ADMIN.to_value();
        }
        outer[0] = Value::Binary(encode(&inner).unwrap());
        assert!(UserLink::decode(&encode(&wire).unwrap())
            .unwrap()
            .decode_generic()
            .is_err());
    }

    let exact = fixture("user-mutations", "go-creator-zero-removal-boxes.snowp");
    let mut wire = decode(&exact).unwrap();
    let metadata = wire_array(&mut wire_array(&mut wire)[3]);
    metadata[2] = Role::ADMIN.to_value();
    assert!(foks_proto::TeamRemovalBoxData::decode(&encode(&wire).unwrap()).is_err());
}

fn wire_array(value: &mut foks_snowpack::Value) -> &mut Vec<foks_snowpack::Value> {
    match value {
        foks_snowpack::Value::Array(values) => values,
        _ => panic!("array"),
    }
}

fn wire_variant(value: &mut foks_snowpack::Value) -> &mut foks_snowpack::Value {
    match value {
        foks_snowpack::Value::Variant(Some((_, value))) => value,
        _ => panic!("variant"),
    }
}
