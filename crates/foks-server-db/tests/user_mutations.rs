mod common;

use foks_merkle_store::{prepare, LeafChange};
use foks_server_db::{UserMutation, UserMutationFailurePoint};

#[test]
fn every_user_mutation_publication_boundary_is_atomic() {
    for point in [
        UserMutationFailurePoint::Chain,
        UserMutationFailurePoint::Projection,
        UserMutationFailurePoint::Passphrase,
        UserMutationFailurePoint::MerkleNodes,
        UserMutationFailurePoint::MerkleRoot,
        UserMutationFailurePoint::Receipt,
    ] {
        let mut fixture = common::TestDatabase::new();
        fixture.reserve(1_000_000);
        fixture.commit(None).unwrap();
        let prior = fixture.database.current_root().unwrap().unwrap();
        let leaf = ([0x61; 32], [0x62; 32]);
        let commit = prepare(
            &fixture.database.node_reader(),
            prior.root_node,
            &[LeafChange::Set {
                key: leaf.0,
                value: leaf.1,
            }],
        )
        .unwrap();
        let mutation = UserMutation {
            uid: &[1; 33],
            signer_device_id: &[4; 33],
            expected_sequence: 2,
            expected_tail_hash: &[0x32; 32],
            link_hash: &[0x63; 32],
            exact_link: b"user-link-two",
            next_tree_location: &[0x64; 32],
            added_credential: None,
            revoked_device_id: None,
            shared_keys: &[],
            parcels: &[],
            seed_chain: &[],
            passphrase: None,
            expected_root_epoch: 1,
            expected_root_hash: &[0x34; 32],
            merkle_commit: &commit,
            merkle_leaves: &[leaf],
            root_epoch: 2,
            root_hash: &[0x65; 32],
            exact_root: b"root-two",
            exact_signed_root: b"signed-root-two",
            back_pointers: &[(1, [0x34; 32])],
            idempotency_key: &[0x66; 32],
            request_hash: &[0x67; 32],
            response: b"",
            now: 1_000_001,
            receipt_expires_at: 2_000_001,
        };
        assert!(fixture
            .database
            .commit_user_mutation_with_failure(&mutation, Some(point))
            .is_err());
        let authority = fixture.database.user_authority(&[1; 33]).unwrap().unwrap();
        assert_eq!(authority.chain_sequence, 1, "{point:?}");
        assert_eq!(authority.current_root_epoch, 1, "{point:?}");
        assert!(fixture.database.commit_user_mutation(&mutation).is_ok());
        let authority = fixture.database.user_authority(&[1; 33]).unwrap().unwrap();
        assert_eq!(authority.chain_sequence, 2);
        assert_eq!(authority.current_root_epoch, 2);
    }
}
