mod common;

use foks_crypto::prefixed_hash;
use foks_merkle_store::{apply_commit, prepare, proof, MemoryStore, EMPTY_ROOT};
use foks_merkle_store::{chain_key, username_key, username_leaf, LeafChange};
use foks_proto::{UserChain, LINK_OUTER_TYPE_ID};

#[test]
fn rebuilt_tree_and_every_official_proof_match_go_v019() {
    let (chain, changes, queries) = official_tree();
    let mut store = MemoryStore::default();
    let commit = prepare(&store, EMPTY_ROOT, &changes).unwrap();
    assert_eq!(commit.root, chain.merkle.root().root_node);
    assert_eq!(commit.leaf_count, changes.len());
    apply_commit(&mut store, &commit).unwrap();

    let official = chain.merkle.paths();
    assert_eq!(official.len(), queries.len());
    for (query, expected) in queries.into_iter().zip(official) {
        assert_eq!(proof(&store, commit.root, query).unwrap(), *expected);
    }
}

fn official_tree() -> (UserChain, Vec<LeafChange>, Vec<[u8; 32]>) {
    let chain = UserChain::decode(&common::fixture("user-chain.snowp")).unwrap();
    let eldest = chain.links[0].decode_eldest().unwrap();
    let mut changes = Vec::new();
    let mut queries = vec![
        username_key(b"fixtureuser", &eldest.host, 1).unwrap(),
        username_key(b"fixtureuser", &eldest.host, 2).unwrap(),
    ];
    changes.push(LeafChange::Set {
        key: queries[0],
        value: username_leaf(&eldest.uid).unwrap(),
    });
    for (index, link) in chain.links.iter().enumerate() {
        let sequence = index as u64 + 1;
        let location = index.checked_sub(1).map(|offset| &chain.locations[offset]);
        let key = chain_key(0, &eldest.uid, sequence, location).unwrap();
        queries.push(key);
        changes.push(LeafChange::Set {
            key,
            value: prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded().unwrap()),
        });
    }
    queries.push(
        chain_key(
            0,
            &eldest.uid,
            chain.links.len() as u64 + 1,
            chain.locations.last(),
        )
        .unwrap(),
    );
    (chain, changes, queries)
}
