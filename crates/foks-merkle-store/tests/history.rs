mod common;

use foks_merkle_store::{
    back_pointer_hash, back_pointer_sequence, collect_roots, MAX_CANONICAL_MERKLE_EPOCH,
};
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
        assert_eq!(
            back_pointer_hash(epoch, &pairs).unwrap(),
            root.back_pointers
        );
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
fn maximum_canonical_epoch_is_rejected() {
    let sequence = back_pointer_sequence(MAX_CANONICAL_MERKLE_EPOCH);
    assert_eq!(sequence.len(), 16);
    let pointers = sequence
        .into_iter()
        .map(|epoch| (epoch, [0; 32]))
        .collect::<Vec<_>>();
    assert!(matches!(
        back_pointer_hash(MAX_CANONICAL_MERKLE_EPOCH, &pointers),
        Err(foks_merkle_store::Error::EpochLimitExceeded {
            epoch: MAX_CANONICAL_MERKLE_EPOCH,
            pointer_count: 16,
        })
    ));
}

#[test]
fn epoch_limit_error_reports_boundary() {
    let epoch = MAX_CANONICAL_MERKLE_EPOCH;
    let pointers = back_pointer_sequence(epoch)
        .into_iter()
        .map(|pointer_epoch| (pointer_epoch, [0; 32]))
        .collect::<Vec<_>>();
    let error = back_pointer_hash(epoch, &pointers).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Merkle epoch 65536 back-pointer count 16 exceeds canonical encoding limit"
    );
}
