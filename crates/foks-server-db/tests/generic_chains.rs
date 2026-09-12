mod common;

use foks_merkle_store::{prepare, LeafChange};
use foks_server_db::{GenericLinkMutation, GenericMutation};

#[test]
fn generic_links_advance_from_the_derived_seed_location() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();
    let prior = fixture.database.current_root().unwrap().unwrap();
    let uid = foks_proto::EntityId::from_bytes(vec![1; 33]).unwrap();
    let seed = [0x35; 32];
    let location =
        foks_crypto::subchain_tree_location(&seed, foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP).unwrap();
    let link_hash = [0x81; 32];
    let key = foks_merkle_store::chain_key(
        foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP,
        &uid,
        1,
        Some(&location),
    )
    .unwrap();
    let leaf = (key, link_hash);
    let commit = prepare(
        &fixture.database.node_reader(),
        prior.root_node,
        &[LeafChange::Set {
            key: leaf.0,
            value: leaf.1,
        }],
    )
    .unwrap();
    fixture
        .database
        .commit_generic_mutation(&GenericMutation {
            invitation: None,
            link: GenericLinkMutation {
                entity_id: uid.as_bytes(),
                chain_type: foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP,
                signer_credential_id: &[4; 33],
                sequence: 1,
                previous: None,
                link_root_epoch: prior.epoch,
                link_root_hash: &prior.root_hash,
                current_tree_location: &location,
                next_tree_location: &[0x82; 32],
                link_hash: &link_hash,
                exact_link: b"generic-link-one",
                passphrase_info: None,
            },
            passphrase: None,
            expected_root_epoch: prior.epoch,
            expected_root_hash: &prior.root_hash,
            merkle_commit: &commit,
            merkle_leaves: &[leaf],
            root_epoch: 2,
            root_hash: &[0x83; 32],
            exact_root: b"generic-root-two",
            exact_signed_root: b"signed-generic-root-two",
            back_pointers: &[(1, prior.root_hash)],
            now: 1_000_001,
        })
        .unwrap();

    let chain = fixture
        .database
        .generic_chain(uid.as_bytes(), foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP)
        .unwrap()
        .unwrap();
    assert_eq!(chain.location_seed, seed);
    assert_eq!(chain.links.len(), 1);
    assert_eq!(chain.links[0].sequence, 1);
    assert_eq!(chain.links[0].link_hash, link_hash);
    assert_eq!(chain.links[0].next_tree_location, [0x82; 32]);
    assert_eq!(fixture.database.current_root().unwrap().unwrap().epoch, 2);
}
