use foks_proto::{
    decode_merkle_back_pointers, ChangeMetadata, EntityId, Hepk, HistoricalMerkleRoots, PukParcel,
    Role, SharedKeySeed, SoftwareEldestPublic, TeamChain, UnsignedUserLink, UserChain,
    UserChainResponse, UserLink,
};

const DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{name}")).unwrap()
}

#[test]
fn official_eldest_user_chain_decodes() {
    let exact = fixture("user-chain.snowp");
    let chain = UserChain::decode(&exact).unwrap();
    assert_eq!(chain.links.len(), 3);
    assert_eq!(chain.locations.len(), 3);
    assert_eq!(chain.merkle.paths().len(), 6);
    assert_eq!(chain.num_username_links, 2);
    assert_eq!(chain.usernames.len(), 1);
    assert_eq!(chain.usernames[0].name, b"fixtureuser");
    assert_eq!(chain.usernames[0].sequence, 1);
    assert_eq!(chain.username_utf8, b"fixtureuser");
    assert_eq!(chain.device_names.len(), 2);
    assert_eq!(
        chain.device_names[0].label.normalized_name,
        b"fixture-device"
    );
    assert_eq!(chain.device_names[1].label.serial, 2);
    assert_eq!(chain.hepks.len(), 4);
    assert_eq!(chain.merkle.root().epoch, 998);

    let eldest = chain.links[0].decode_eldest().unwrap();
    assert_eq!(eldest.seqno, 1);
    assert_eq!(eldest.previous, None);
    assert_eq!(eldest.signer, eldest.member);
    assert_eq!(eldest.member, eldest.member_verify_key);
    assert_eq!(eldest.puk_generation, 1);
    assert_eq!(eldest.uid.entity_type(), 1);
    assert_eq!(eldest.host.entity_type(), 2);
    assert_eq!(eldest.signer.entity_type(), 4);
    assert_eq!(eldest.puk_verify_key.entity_type(), 14);
    assert!(matches!(
        eldest.metadata.as_slice(),
        [
            ChangeMetadata::Username(_),
            ChangeMetadata::DeviceName(_),
            ChangeMetadata::Eldest { .. }
        ]
    ));
    assert_eq!(chain.hepks[0].mlkem768().len(), 1184);

    let links = chain
        .links
        .iter()
        .map(UserLink::encoded)
        .collect::<foks_proto::Result<Vec<_>>>()
        .unwrap();
    let root = chain.merkle.encoded_root().unwrap();
    let hepks = chain
        .hepks
        .iter()
        .map(Hepk::encoded)
        .collect::<foks_proto::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        UserChainResponse {
            exact_links: &links,
            locations: &chain.locations,
            usernames: &chain.usernames,
            exact_root: &root,
            paths: chain.merkle.paths(),
            device_names: &chain.device_names,
            username_utf8: &chain.username_utf8,
            num_username_links: chain.num_username_links,
            exact_hepks: &hepks,
        }
        .encoded()
        .unwrap(),
        exact
    );
}

#[test]
fn official_transition_links_and_merkle_history_decode() {
    let provision = UserLink::decode(&fixture("user-provision-link.snowp")).unwrap();
    let provision_change = provision.decode_group_change().unwrap();
    assert_eq!(provision_change.seqno, 2);
    assert_eq!(provision_change.changes.len(), 1);
    assert_eq!(provision_change.changes[0].role, Role::OWNER);
    assert_eq!(provision_change.shared_keys.len(), 0);
    assert!(matches!(
        provision_change.metadata.as_slice(),
        [ChangeMetadata::DeviceName(_)]
    ));
    assert_eq!(provision.signatures().len(), 2);
    assert_eq!(
        UnsignedUserLink::user_group_change(&provision_change)
            .unwrap()
            .finish(provision.signatures().to_vec())
            .unwrap()
            .encoded()
            .unwrap(),
        fixture("user-provision-link.snowp")
    );

    let revoke = UserLink::decode(&fixture("user-revoke-link.snowp")).unwrap();
    let revoke_change = revoke.decode_group_change().unwrap();
    assert_eq!(revoke_change.seqno, 3);
    assert_eq!(revoke_change.changes[0].role, Role::NONE);
    assert_eq!(revoke_change.shared_keys[0].generation, 2);
    assert!(revoke_change.metadata.is_empty());
    assert_eq!(revoke.signatures().len(), 2);
    assert_eq!(
        UnsignedUserLink::user_group_change(&revoke_change)
            .unwrap()
            .finish(revoke.signatures().to_vec())
            .unwrap()
            .encoded()
            .unwrap(),
        fixture("user-revoke-link.snowp")
    );

    let pointers = decode_merkle_back_pointers(&fixture("merkle-back-pointers-996.snowp")).unwrap();
    assert_eq!(
        pointers
            .iter()
            .map(|pointer| pointer.epoch)
            .collect::<Vec<_>>(),
        [995, 994, 992]
    );
    let history =
        HistoricalMerkleRoots::decode(&fixture("merkle-historical-response.snowp")).unwrap();
    assert_eq!(history.roots[0].epoch, 996);
    assert_eq!(history.hashes.len(), 4);
    assert_eq!(
        history.encoded().unwrap(),
        fixture("merkle-historical-response.snowp")
    );
}

#[test]
fn exact_user_link_round_trips() {
    let bytes = fixture("user-eldest-link.snowp");
    let link = UserLink::decode(&bytes).unwrap();
    assert_eq!(link.signatures().len(), 2);
    assert_eq!(link.encoded().unwrap(), bytes);
}

#[test]
fn software_eldest_builder_matches_the_official_go_link() {
    let bytes = fixture("user-eldest-link.snowp");
    let expected = UserLink::decode(&bytes).unwrap();
    let eldest = expected.decode_eldest().unwrap();
    let subchain_location_commitment = eldest
        .metadata
        .iter()
        .find_map(|metadata| match metadata {
            ChangeMetadata::Eldest {
                subchain_location_commitment,
            } => Some(*subchain_location_commitment),
            _ => None,
        })
        .unwrap();
    let username_commitment = eldest
        .metadata
        .iter()
        .find_map(|metadata| match metadata {
            ChangeMetadata::Username(commitment) => Some(*commitment),
            _ => None,
        })
        .unwrap();
    let device_name_commitment = eldest
        .metadata
        .iter()
        .find_map(|metadata| match metadata {
            ChangeMetadata::DeviceName(commitment) => Some(*commitment),
            _ => None,
        })
        .unwrap();
    let unsigned = UnsignedUserLink::software_eldest(&SoftwareEldestPublic {
        host: &eldest.host,
        uid: &eldest.uid,
        device: &eldest.member,
        device_hepk_fingerprint: eldest.member_hepk_fingerprint,
        puk_verify_key: &eldest.puk_verify_key,
        puk_hepk_fingerprint: eldest.puk_hepk_fingerprint,
        root: &eldest.root,
        time: eldest.time,
        next_location_commitment: eldest.next_tree_location,
        username_commitment,
        device_name_commitment,
        subchain_location_commitment,
    })
    .unwrap();
    assert_eq!(
        unsigned
            .finish(expected.signatures().to_vec())
            .unwrap()
            .encoded()
            .unwrap(),
        bytes
    );
}

#[test]
fn official_mock_yubi_and_subkey_matrix_decodes() {
    let yubi_id = match foks_snowpack::decode(&fixture("yubi/yubi-id.snowp")).unwrap() {
        foks_snowpack::Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        _ => panic!("Yubi fixture is not an EntityID"),
    };
    assert_eq!(yubi_id.entity_type(), 8);
    assert_eq!(yubi_id.p256_key().unwrap().len(), 33);

    let hepk = Hepk::decode(&fixture("yubi/yubi-hepk.snowp")).unwrap();
    assert!(hepk.p256().is_some());
    assert_eq!(hepk.mlkem768().len(), 1184);

    let link = UserLink::decode(&fixture("yubi/yubi-eldest-link.snowp")).unwrap();
    let eldest = link.decode_eldest().unwrap();
    assert_eq!(eldest.member, yubi_id);
    assert_eq!(eldest.member_subkey.as_ref().unwrap().entity_type(), 13);
    assert_eq!(link.signatures().len(), 3);

    let to_yubi = PukParcel::decode(&fixture("yubi/software-to-yubi-puk-parcel.snowp")).unwrap();
    assert_eq!(to_yubi.target.entity_type(), 8);
    assert_eq!(to_yubi.hybrid.dh_type, 2);
    let to_software =
        PukParcel::decode(&fixture("yubi/yubi-to-software-puk-parcel.snowp")).unwrap();
    assert_eq!(to_software.target.entity_type(), 4);
    assert_eq!(to_software.hybrid.dh_type, 1);
}

#[test]
fn official_puk_parcel_and_cleartext_decode() {
    let exact = fixture("puk-parcel.snowp");
    let parcel = PukParcel::decode(&exact).unwrap();
    assert_eq!(parcel.generation, 2);
    assert_eq!(parcel.role, Role::OWNER);
    assert_eq!(parcel.hybrid.kem_ciphertext.len(), 1088);
    assert_eq!(parcel.hybrid.dh_type, 1);
    assert!(parcel.hybrid.sender_dh.is_none());
    assert_eq!(parcel.hybrid.ciphertext.len(), 126);
    assert_eq!(parcel.seed_chain.len(), 1);
    assert_eq!(parcel.seed_chain[0].generation, 1);
    assert_eq!(parcel.seed_chain[0].role, Role::OWNER);
    assert_eq!(parcel.encoded().unwrap(), exact);

    let cleartext = SharedKeySeed::decode(&fixture("puk-cleartext.snowp")).unwrap();
    assert_eq!(cleartext.generation, 2);
    assert_eq!(cleartext.role, Role::OWNER);
    let expected_seed: [u8; 32] = fixture("puk-seed.bin").try_into().unwrap();
    assert_eq!(cleartext.seed.as_bytes(), &expected_seed);
    assert_eq!(format!("{:?}", cleartext.seed), "SecretSeed([REDACTED])");
    assert!(!format!("{cleartext:?}").contains(&format!("{expected_seed:?}")));
}

#[test]
fn official_named_team_chain_and_ptk_parcels_decode() {
    let chain = TeamChain::decode(&fixture("team-chain.snowp")).unwrap();
    assert_eq!(chain.links.len(), 1);
    assert_eq!(chain.boxes.len(), 4);
    assert_eq!(chain.hepks.len(), 4);
    assert_eq!(chain.team_name_utf8, b"fixtureteam");
    let change = chain.links[0].decode_team_group_change().unwrap();
    assert_eq!(change.shared_keys.len(), 4);
    assert_eq!(change.changes.len(), 1);
}
