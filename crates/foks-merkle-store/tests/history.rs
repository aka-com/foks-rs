mod common;

use foks_merkle_store::{back_pointer_hash, back_pointer_sequence, collect_roots};
use foks_proto::{decode_merkle_back_pointers, MerkleRoot};

#[test]
fn official_back_pointer_lists_hash_to_the_published_roots() {
    for epoch in [996, 997, 998] {
        let pointers = decode_merkle_back_pointers(&common::fixture(&format!(
            "merkle-back-pointers-{epoch}.snowp"
        )))
        .unwrap();
        assert_eq!(
            pointers
                .iter()
                .map(|pointer| pointer.epoch)
                .collect::<Vec<_>>(),
            back_pointer_sequence(epoch)
        );
        let pairs = pointers
            .iter()
            .map(|pointer| (pointer.epoch, pointer.hash))
            .collect::<Vec<_>>();
        let root =
            MerkleRoot::decode(&common::fixture(&format!("merkle-root-{epoch}.snowp"))).unwrap();
        assert_eq!(back_pointer_hash(&pairs).unwrap(), root.back_pointers);
    }
}

#[test]
fn historical_collection_matches_the_fixture_request_span() {
    assert_eq!(
        collect_roots(998, 995),
        (vec![998, 996], vec![997, 996, 995, 994, 992])
    );
}

#[test]
fn early_and_power_of_two_sequences_match_v019() {
    assert_eq!(back_pointer_sequence(0), []);
    assert_eq!(back_pointer_sequence(1), []);
    assert_eq!(back_pointer_sequence(2), [1]);
    assert_eq!(back_pointer_sequence(3), [2, 1]);
    assert_eq!(back_pointer_sequence(4), [3, 2, 1]);
    assert_eq!(back_pointer_sequence(8), [7, 6, 4]);
    assert_eq!(back_pointer_sequence(16), [15, 14, 12, 8]);
}

#[test]
fn go_rejected_array16_back_pointer_lists_are_not_hashed() {
    let pointers = (0..16).map(|epoch| (epoch, [0; 32])).collect::<Vec<_>>();
    assert!(matches!(
        back_pointer_hash(&pointers),
        Err(foks_merkle_store::Error::Snowpack(_))
    ));
}
