use foks_client::{
    AdHocTeamSecrets, NamedTeamSecrets, NewSoftwareDeviceSecrets, NoPassphraseConfigured,
    SoftwareDeviceProvisionRequest, UserPukRotation,
};
use foks_proto::{
    AdHocMembershipLinkPublic, PostGenericLinkArgument, Role, SecretSeed, TreeRoot,
    UnsignedUserLink, CHAIN_TYPE_TEAM_MEMBERSHIP, ENTITY_PTK_VERIFY, LINK_OUTER_TYPE_ID,
    LINK_OUTER_V1_TYPE_ID, MERKLE_ROOT_TYPE_ID, TREE_LOCATION_TYPE_ID,
};
use foks_server_testkit::TestAccountSpec;
use foks_snowpack::{encode, Value};

use crate::support::Fixture;

#[test]
pub(crate) fn generic_membership_chains_and_trusted_team_lists_work() {
    let fixture = Fixture::start("go-client-generic");
    let account = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("genericowner", 0x91))
        .unwrap();
    let adhoc = fixture
        .client
        .foks()
        .create_single_owner_adhoc_team(
            fixture.host(),
            &account.credential,
            &AdHocTeamSecrets {
                member_min: SecretSeed::new([0x11; 32]),
                member: SecretSeed::new([0x12; 32]),
                admin: SecretSeed::new([0x13; 32]),
                owner: SecretSeed::new([0x14; 32]),
            },
        )
        .unwrap();
    let first = fixture
        .client
        .foks()
        .load_generic_chain(
            fixture.host(),
            &account.credential,
            CHAIN_TYPE_TEAM_MEMBERSHIP,
            1,
        )
        .unwrap();
    assert_eq!(first.links.len(), 1);
    assert_eq!(first.locations.len(), 1);
    assert_eq!(first.merkle.paths().len(), 2);

    let device = foks_crypto::derive_device_public(&account.credential.seed).unwrap();
    let previous =
        foks_crypto::prefixed_hash_signable(LINK_OUTER_TYPE_ID, &first.links[0].encoded().unwrap())
            .unwrap();
    let exact_root = first.merkle.encoded_root().unwrap();
    let root = TreeRoot {
        epoch: first.merkle.root().epoch,
        hash: foks_crypto::prefixed_hash_signable(MERKLE_ROOT_TYPE_ID, &exact_root).unwrap(),
    };
    let next_tree_location = [0xa5; 32];
    let location_wire = encode(&Value::Binary(next_tree_location.to_vec())).unwrap();
    let unsigned = UnsignedUserLink::approved_adhoc_membership(&AdHocMembershipLinkPublic {
        user: &account.credential.uid,
        host: fixture.host().host_id(),
        signer: &device.id,
        sequence: 2,
        previous: Some(previous),
        root: &root,
        time: 0,
        next_location_commitment: foks_crypto::prefixed_hash_signable(
            TREE_LOCATION_TYPE_ID,
            &location_wire,
        )
        .unwrap(),
        team: &adhoc.team,
        source_role: Role::OWNER,
        destination_role: Role::OWNER,
        team_sequence: 1,
    })
    .unwrap();
    let signature = foks_crypto::sign_shared_key_typed(
        &account.credential.seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[]).unwrap(),
    )
    .unwrap();
    fixture
        .client
        .foks()
        .post_generic_link(
            fixture.host(),
            &account.credential,
            &PostGenericLinkArgument {
                link: unsigned.finish(vec![signature]).unwrap(),
                next_tree_location,
            },
        )
        .unwrap();
    assert_eq!(
        fixture
            .client
            .foks()
            .load_generic_chain(
                fixture.host(),
                &account.credential,
                CHAIN_TYPE_TEAM_MEMBERSHIP,
                1,
            )
            .unwrap()
            .links
            .len(),
        2
    );

    let named = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &account.credential,
            "genericteam",
            &NamedTeamSecrets {
                member_min: SecretSeed::new([0x21; 32]),
                member: SecretSeed::new([0x22; 32]),
                admin: SecretSeed::new([0x23; 32]),
                owner: SecretSeed::new([0x24; 32]),
                removal_key: SecretSeed::new([0x25; 32]),
                team_name_commitment_key: [0x26; 16],
            },
        )
        .unwrap();
    let empty_team_memberships = fixture
        .client
        .foks()
        .load_team_membership_chain(fixture.host(), &account.credential, &named.authenticated, 1)
        .unwrap();
    assert!(empty_team_memberships.links.is_empty());
    assert_eq!(empty_team_memberships.merkle.paths().len(), 1);
    let team_root_wire = empty_team_memberships.merkle.encoded_root().unwrap();
    let team_root = TreeRoot {
        epoch: empty_team_memberships.merkle.root().epoch,
        hash: foks_crypto::prefixed_hash_signable(MERKLE_ROOT_TYPE_ID, &team_root_wire).unwrap(),
    };
    let team_owner = named
        .authenticated
        .ptks
        .iter()
        .find(|key| key.role == Role::OWNER)
        .unwrap();
    let team_owner_public =
        foks_crypto::derive_shared_public(&team_owner.seed, ENTITY_PTK_VERIFY).unwrap();
    let team_membership_next = [0xb5; 32];
    let team_membership_location = encode(&Value::Binary(team_membership_next.to_vec())).unwrap();
    let unsigned = UnsignedUserLink::approved_adhoc_membership(&AdHocMembershipLinkPublic {
        user: &named.team,
        host: fixture.host().host_id(),
        signer: &team_owner_public.verify_key,
        sequence: 1,
        previous: None,
        root: &team_root,
        time: 0,
        next_location_commitment: foks_crypto::prefixed_hash_signable(
            TREE_LOCATION_TYPE_ID,
            &team_membership_location,
        )
        .unwrap(),
        team: &adhoc.team,
        source_role: Role::OWNER,
        destination_role: Role::OWNER,
        team_sequence: 1,
    })
    .unwrap();
    let signature = foks_crypto::sign_shared_key_typed(
        &team_owner.seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[]).unwrap(),
    )
    .unwrap();
    fixture
        .client
        .foks()
        .post_team_membership_link(
            fixture.host(),
            &account.credential,
            &named.authenticated,
            &PostGenericLinkArgument {
                link: unsigned.finish(vec![signature]).unwrap(),
                next_tree_location: team_membership_next,
            },
        )
        .unwrap();
    let loaded_team_memberships = fixture
        .client
        .foks()
        .load_team_membership_chain(fixture.host(), &account.credential, &named.authenticated, 1)
        .unwrap();
    assert_eq!(loaded_team_memberships.links.len(), 1);
    assert_eq!(
        loaded_team_memberships.links[0]
            .decode_approved_membership()
            .unwrap()
            .user,
        named.team
    );
    let teams = fixture
        .client
        .foks()
        .local_team_list(fixture.host(), &account.credential)
        .unwrap();
    assert_eq!(teams.len(), 2);
    assert!(teams.iter().any(|entry| entry.team == adhoc.team));
    assert!(teams.iter().any(|entry| entry.team == named.team));

    let mut protected = fixture.client.open_protected_store().unwrap();
    let provisioned = fixture
        .client
        .foks()
        .provision_software_device(
            fixture.host(),
            &account.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: "generic successor".to_owned(),
                serial: 2,
            },
            NewSoftwareDeviceSecrets::new(SecretSeed::new([0x31; 32]), None, [0x32; 17]),
            &mut protected,
        )
        .unwrap();
    let original = foks_crypto::derive_device_public(&account.credential.seed).unwrap();
    fixture
        .client
        .foks()
        .revoke_user_credential_with_software_device(
            fixture.host(),
            &provisioned.credential,
            &original.id,
            &[UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::new([0x92; 32]),
                new_seed: SecretSeed::new([0x33; 32]),
            }],
            Some(NoPassphraseConfigured),
            &mut protected,
        )
        .unwrap();
    let successor_team = fixture
        .client
        .foks()
        .create_single_owner_adhoc_team(
            fixture.host(),
            &provisioned.credential,
            &AdHocTeamSecrets {
                member_min: SecretSeed::new([0x34; 32]),
                member: SecretSeed::new([0x35; 32]),
                admin: SecretSeed::new([0x36; 32]),
                owner: SecretSeed::new([0x37; 32]),
            },
        )
        .unwrap();
    assert!(fixture
        .client
        .foks()
        .local_team_list(fixture.host(), &provisioned.credential)
        .unwrap()
        .iter()
        .any(|entry| entry.team == successor_team.team));
}
