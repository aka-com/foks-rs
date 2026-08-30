mod common;

use foks_merkle_store::{prepare, LeafChange};
use foks_server_db::{
    GenericLinkMutation, GenericPassphraseInfo, PassphraseMutation, SharedKeyMutation,
    UserMutation, UserMutationFailurePoint,
};

const SALT: [u8; 16] = [0x72; 16];

fn passphrase(generation: u64, puk_generation: u64, now: u64) -> PassphraseMutation<'static> {
    PassphraseMutation {
        verify_key: &[17; 33],
        salt: &SALT,
        generation,
        exact_skmwk_box: b"atomic-skmwk-box",
        exact_passphrase_box: b"atomic-passphrase-box",
        exact_puk_box: Some(b"atomic-puk-box"),
        puk_generation: Some(puk_generation),
        puk_role: Some(foks_proto::Role::OWNER),
        stretch_version: 1,
        now,
    }
}

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
        common::commit_with_passphrase(&mut fixture.database, None, passphrase(1, 1, 1_000_000))
            .unwrap();
        let prior = fixture.database.current_root().unwrap().unwrap();
        let uid = foks_proto::EntityId::from_bytes(vec![1; 33]).unwrap();
        let current_tree_location = [0x33; 32];
        let link_hash = [0x63; 32];
        let leaf = (
            foks_merkle_store::chain_key(0, &uid, 2, Some(&current_tree_location)).unwrap(),
            link_hash,
        );
        let settings_location =
            foks_crypto::subchain_tree_location(&[0x35; 32], foks_proto::CHAIN_TYPE_USER_SETTINGS)
                .unwrap();
        let settings_hash = [0x68; 32];
        let settings_leaf = (
            foks_merkle_store::chain_key(
                foks_proto::CHAIN_TYPE_USER_SETTINGS,
                &uid,
                1,
                Some(&settings_location),
            )
            .unwrap(),
            settings_hash,
        );
        let commit = prepare(
            &fixture.database.node_reader(),
            prior.root_node,
            &[
                LeafChange::Set {
                    key: leaf.0,
                    value: leaf.1,
                },
                LeafChange::Set {
                    key: settings_leaf.0,
                    value: settings_leaf.1,
                },
            ],
        )
        .unwrap();
        let owner_key = SharedKeyMutation {
            role_type: 3,
            visibility: 0,
            generation: 2,
            verify_key: &[0x69; 33],
            exact_hepk: b"rotated-owner-hepk",
        };
        let passphrase = passphrase(2, 2, 1_000_001);
        let settings = GenericLinkMutation {
            entity_id: uid.as_bytes(),
            chain_type: foks_proto::CHAIN_TYPE_USER_SETTINGS,
            signer_credential_id: &[4; 33],
            sequence: 1,
            previous: None,
            link_root_epoch: prior.epoch,
            link_root_hash: &prior.root_hash,
            current_tree_location: &settings_location,
            next_tree_location: &[0x6a; 32],
            link_hash: &settings_hash,
            exact_link: b"user-settings-link-one",
            passphrase_info: Some(GenericPassphraseInfo {
                generation: 2,
                salt: Some(&SALT),
                stretch_version: 1,
            }),
        };
        let mutation = UserMutation {
            uid: &[1; 33],
            signer_device_id: &[4; 33],
            expected_sequence: 2,
            expected_tail_hash: &[0x32; 32],
            link_hash: &link_hash,
            exact_link: b"user-link-two",
            current_tree_location: &current_tree_location,
            next_tree_location: &[0x64; 32],
            added_credential: None,
            revoked_device_id: None,
            shared_keys: &[owner_key],
            parcels: &[],
            seed_chain: &[],
            passphrase: Some(passphrase),
            user_settings: Some(settings),
            expected_root_epoch: 1,
            expected_root_hash: &[0x34; 32],
            merkle_commit: &commit,
            merkle_leaves: &[leaf, settings_leaf],
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
        assert_eq!(
            fixture
                .database
                .passphrase(&[1; 33])
                .unwrap()
                .unwrap()
                .generation,
            1,
            "{point:?}"
        );
        assert!(
            fixture
                .database
                .generic_chain(&[1; 33], foks_proto::CHAIN_TYPE_USER_SETTINGS)
                .unwrap()
                .unwrap()
                .links
                .is_empty(),
            "{point:?}"
        );
        assert!(fixture.database.commit_user_mutation(&mutation).is_ok());
        let authority = fixture.database.user_authority(&[1; 33]).unwrap().unwrap();
        assert_eq!(authority.chain_sequence, 2);
        assert_eq!(authority.current_root_epoch, 2);
        assert_eq!(
            fixture
                .database
                .passphrase(&[1; 33])
                .unwrap()
                .unwrap()
                .generation,
            2
        );
        assert_eq!(
            fixture
                .database
                .generic_chain(&[1; 33], foks_proto::CHAIN_TYPE_USER_SETTINGS)
                .unwrap()
                .unwrap()
                .links
                .len(),
            1
        );
    }
}
