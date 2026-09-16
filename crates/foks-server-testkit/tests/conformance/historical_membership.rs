use foks_client::{DeviceCredential, NamedTeamSecrets};
use foks_proto::{
    EntityId, GenericLinkPayload, PostGenericLinkArgument, SecretSeed, TeamMembershipState,
    TreeRoot, UserLink, CHAIN_TYPE_TEAM_MEMBERSHIP, LINK_OUTER_TYPE_ID, LINK_OUTER_V1_TYPE_ID,
    MERKLE_ROOT_TYPE_ID, TREE_LOCATION_TYPE_ID,
};
use foks_server_testkit::TestAccountSpec;
use foks_snowpack::{decode, encode, Value};

use crate::support::Fixture;

#[test]
fn historical_go_founding_approval_preserves_evidence_and_allows_subsequent_creation() {
    let fixture = Fixture::start("historical-go-membership");
    let account = fixture
        .client
        .create_account(
            fixture.host(),
            &TestAccountSpec::new("historicalowner", 0xb1),
        )
        .unwrap();
    let first = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &account.credential,
            "historicalteam",
            &secrets(0x41),
        )
        .unwrap();
    let chain = fixture
        .client
        .foks()
        .load_generic_chain(
            fixture.host(),
            &account.credential,
            CHAIN_TYPE_TEAM_MEMBERSHIP,
            1,
        )
        .unwrap();
    let previous = foks_crypto::prefixed_hash_signable(
        LINK_OUTER_TYPE_ID,
        &chain.links.last().unwrap().encoded().unwrap(),
    )
    .unwrap();
    let root = TreeRoot {
        epoch: chain.merkle.root().epoch,
        hash: foks_crypto::prefixed_hash_signable(
            MERKLE_ROOT_TYPE_ID,
            &chain.merkle.encoded_root().unwrap(),
        )
        .unwrap(),
    };
    let argument = rebind_go_fixture(
        &account.credential,
        fixture.host().host_id(),
        &first.team,
        first.authenticated.verified.members()[0]
            .removal_key_commitment
            .unwrap(),
        previous,
        &root,
    );
    let exact = argument.link.encoded().unwrap();
    fixture
        .client
        .foks()
        .post_generic_link(fixture.host(), &account.credential, &argument)
        .unwrap();
    let user = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &account.credential)
        .unwrap();
    let graph = fixture
        .client
        .foks()
        .discover_local_team_graph(
            fixture.host(),
            &account.credential,
            &user.verified,
            &user.puks,
        )
        .unwrap();
    assert_eq!(graph.teams.len(), 1);
    assert_eq!(graph.teams[0].verified.team(), &first.team);

    // This reads and verifies the entire old membership chain before reserving
    // or submitting the next team. The zero reference must not strand creation.
    let second = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &account.credential,
            "subsequentteam",
            &secrets(0x51),
        )
        .unwrap();
    assert_ne!(first.team, second.team);
    let loaded = fixture
        .client
        .foks()
        .load_generic_chain(
            fixture.host(),
            &account.credential,
            CHAIN_TYPE_TEAM_MEMBERSHIP,
            1,
        )
        .unwrap();
    assert_eq!(loaded.links.len(), 3);
    assert_eq!(loaded.links[1].encoded().unwrap(), exact);
    let GenericLinkPayload::TeamMembership(historical) =
        loaded.links[1].decode_generic().unwrap().payload
    else {
        panic!("expected membership");
    };
    assert!(matches!(
        historical.state,
        TeamMembershipState::Approved {
            team_sequence: 0,
            ..
        }
    ));
    assert_eq!(
        loaded.links[2]
            .decode_approved_membership()
            .unwrap()
            .team_sequence,
        1
    );
}

fn secrets(fill: u8) -> NamedTeamSecrets {
    NamedTeamSecrets {
        member_min: SecretSeed::new([fill; 32]),
        member: SecretSeed::new([fill + 1; 32]),
        admin: SecretSeed::new([fill + 2; 32]),
        owner: SecretSeed::new([fill + 3; 32]),
        removal_key: SecretSeed::new([fill + 4; 32]),
        team_name_commitment_key: [fill + 5; 16],
    }
}

// Bind the checked-in actual Go producer's wire payload to this isolated test
// identity, sign it, and append it through the normal authenticated endpoint.
// The zero destination sequence is preserved from the fixture. Production
// Rust builders intentionally cannot create this historical representation.
fn rebind_go_fixture(
    credential: &DeviceCredential,
    host: &EntityId,
    team: &EntityId,
    commitment: [u8; 32],
    previous: [u8; 32],
    root: &TreeRoot,
) -> PostGenericLinkArgument {
    let template = include_bytes!("../../../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations/go-creator-zero-membership-link.snowp");
    let link = UserLink::decode(template).unwrap();
    let mut envelope = decode(&link.signing_bytes(0).unwrap()).unwrap();
    let Value::Binary(inner) = &array(&mut envelope)[0] else {
        panic!("inner bytes")
    };
    let mut inner = decode(inner).unwrap();
    let generic = array(variant(&mut array(&mut inner)[1]));
    let next_tree_location = [0x95; 32];
    let next_commitment = foks_crypto::prefixed_hash_signable(
        TREE_LOCATION_TYPE_ID,
        &encode(&Value::Binary(next_tree_location.to_vec())).unwrap(),
    )
    .unwrap();
    generic[0] = Value::Array(vec![
        Value::Array(vec![
            Value::Unsigned(2),
            Value::Binary(previous.to_vec()),
            Value::Array(vec![
                Value::Unsigned(root.epoch),
                Value::Binary(root.hash.to_vec()),
            ]),
            Value::Unsigned(0),
        ]),
        Value::Binary(next_commitment.to_vec()),
    ]);
    generic[1] = Value::Array(vec![
        Value::Binary(credential.uid.as_bytes().to_vec()),
        Value::Binary(host.as_bytes().to_vec()),
    ]);
    let device = credential.public_material().unwrap();
    generic[2] = Value::Array(vec![
        Value::Binary(device.id.as_bytes().to_vec()),
        Value::Null,
    ]);
    let membership = array(variant(&mut array(&mut generic[3])[1]));
    membership[0] = Value::Array(vec![
        Value::Binary(team.as_bytes().to_vec()),
        Value::Binary(host.as_bytes().to_vec()),
    ]);
    let approved = array(variant(&mut array(&mut membership[2])[1]));
    assert_eq!(array(&mut approved[0])[1], Value::Unsigned(0));
    approved[1] = Value::Binary(commitment.to_vec());
    let inner = encode(&inner).unwrap();
    let signing = encode(&Value::Array(vec![
        Value::Binary(inner.clone()),
        Value::Null,
    ]))
    .unwrap();
    let signature =
        foks_crypto::sign_shared_key_typed(&credential.seed, LINK_OUTER_V1_TYPE_ID, &signing)
            .unwrap();
    let wire = encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((
            b"1".to_vec(),
            Box::new(Value::Array(vec![
                Value::Binary(inner),
                Value::Array(vec![signature.to_value()]),
            ])),
        ))),
    ]))
    .unwrap();
    PostGenericLinkArgument {
        link: UserLink::decode(&wire).unwrap(),
        next_tree_location,
    }
}

fn array(value: &mut Value) -> &mut Vec<Value> {
    match value {
        Value::Array(values) => values,
        _ => panic!("expected array"),
    }
}

fn variant(value: &mut Value) -> &mut Value {
    match value {
        Value::Variant(Some((_, value))) => value,
        _ => panic!("expected variant"),
    }
}
