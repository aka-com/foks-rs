mod common;

use foks_merkle_store::{prepare, LeafChange};
use foks_server_db::{
    GenericLinkMutation, GenericMutation, GenericPassphraseInfo, PassphraseMutation,
    SharedKeyMutation, UserMutation, UserMutationFailurePoint,
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
        UserMutationFailurePoint::IdempotencyRecord,
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
            username: None,
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
            cited_root_epoch: 1,
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
            idempotency_expires_at: 2_000_001,
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

#[test]
fn revocation_rejects_generic_links_signed_after_its_cited_root() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    common::commit_with_passphrase(&mut fixture.database, None, passphrase(1, 1, 1_000_000))
        .unwrap();
    let root_one = fixture.database.current_root().unwrap().unwrap();
    let uid = foks_proto::EntityId::from_bytes(vec![foks_proto::ENTITY_USER; 33]).unwrap();
    let host =
        foks_proto::EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x41; 32]].concat())
            .unwrap();
    let signer = foks_proto::EntityId::from_bytes(vec![foks_proto::ENTITY_DEVICE; 33]).unwrap();
    let settings_location =
        foks_crypto::subchain_tree_location(&[0x35; 32], foks_proto::CHAIN_TYPE_USER_SETTINGS)
            .unwrap();
    let passphrase_info = foks_proto::PassphraseInfo {
        generation: 1,
        salt: Some(SALT),
        stretch_version: 1,
    };
    let exact_generic =
        foks_proto::UnsignedUserLink::user_settings(&foks_proto::UserSettingsLinkPublic {
            user: &uid,
            host: &host,
            signer: &signer,
            sequence: 1,
            previous: None,
            root: &foks_proto::TreeRoot {
                epoch: root_one.epoch,
                hash: root_one.root_hash,
            },
            time: 1_000,
            next_location_commitment: [0x71; 32],
            passphrase: &passphrase_info,
        })
        .unwrap()
        .finish_for_kex()
        .unwrap()
        .encoded()
        .unwrap();
    let generic_hash = [0x81; 32];
    let generic_leaf = (
        foks_merkle_store::chain_key(
            foks_proto::CHAIN_TYPE_USER_SETTINGS,
            &uid,
            1,
            Some(&settings_location),
        )
        .unwrap(),
        generic_hash,
    );
    let generic_commit = prepare(
        &fixture.database.node_reader(),
        root_one.root_node,
        &[LeafChange::Set {
            key: generic_leaf.0,
            value: generic_leaf.1,
        }],
    )
    .unwrap();
    fixture
        .database
        .commit_generic_mutation(&GenericMutation {
            invitation: None,
            link: GenericLinkMutation {
                entity_id: uid.as_bytes(),
                chain_type: foks_proto::CHAIN_TYPE_USER_SETTINGS,
                signer_credential_id: signer.as_bytes(),
                sequence: 1,
                previous: None,
                link_root_epoch: root_one.epoch,
                link_root_hash: &root_one.root_hash,
                current_tree_location: &settings_location,
                next_tree_location: &[0x82; 32],
                link_hash: &generic_hash,
                exact_link: &exact_generic,
                passphrase_info: Some(GenericPassphraseInfo {
                    generation: 1,
                    salt: Some(&SALT),
                    stretch_version: 1,
                }),
            },
            passphrase: None,
            expected_root_epoch: root_one.epoch,
            expected_root_hash: &root_one.root_hash,
            merkle_commit: &generic_commit,
            merkle_leaves: &[generic_leaf],
            root_epoch: 2,
            root_hash: &[0x83; 32],
            exact_root: b"generic-root-two",
            exact_signed_root: b"signed-generic-root-two",
            back_pointers: &[(1, root_one.root_hash)],
            now: 1_000_001,
        })
        .unwrap();

    let root_two = fixture.database.current_root().unwrap().unwrap();
    let user_hash = [0x91; 32];
    let user_leaf = (
        foks_merkle_store::chain_key(0, &uid, 2, Some(&[0x33; 32])).unwrap(),
        user_hash,
    );
    let user_commit = prepare(
        &fixture.database.node_reader(),
        root_two.root_node,
        &[LeafChange::Set {
            key: user_leaf.0,
            value: user_leaf.1,
        }],
    )
    .unwrap();
    let pointers = foks_merkle_store::back_pointer_sequence(3)
        .into_iter()
        .map(|epoch| (epoch, [0x92; 32]))
        .collect::<Vec<_>>();
    let result = fixture.database.commit_user_mutation(&UserMutation {
        username: None,
        uid: uid.as_bytes(),
        signer_device_id: signer.as_bytes(),
        expected_sequence: 2,
        expected_tail_hash: &[0x32; 32],
        link_hash: &user_hash,
        exact_link: b"racing-revoke-link",
        current_tree_location: &[0x33; 32],
        next_tree_location: &[0x93; 32],
        added_credential: None,
        revoked_device_id: Some(signer.as_bytes()),
        shared_keys: &[],
        parcels: &[],
        seed_chain: &[],
        passphrase: None,
        user_settings: None,
        cited_root_epoch: root_one.epoch,
        expected_root_epoch: root_two.epoch,
        expected_root_hash: &root_two.root_hash,
        merkle_commit: &user_commit,
        merkle_leaves: &[user_leaf],
        root_epoch: 3,
        root_hash: &[0x94; 32],
        exact_root: b"root-three",
        exact_signed_root: b"signed-root-three",
        back_pointers: &pointers,
        idempotency_key: &[0x95; 16],
        request_hash: &[0x96; 32],
        response: b"",
        now: 1_000_002,
        idempotency_expires_at: 2_000_002,
    });
    assert!(matches!(result, Err(foks_server_db::Error::StaleRoot)));
    assert_eq!(
        fixture
            .database
            .user_authority(uid.as_bytes())
            .unwrap()
            .unwrap()
            .chain_sequence,
        1
    );
}
