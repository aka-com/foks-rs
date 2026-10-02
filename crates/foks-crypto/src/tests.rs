//! Cross-domain Go fixtures, round trips, and tamper rejection.

use crate::backup::{BackupKey, BACKUP_SEED_BYTES};
use crate::hybrid::{
    derive_device_public, derive_hybrid_key, derive_shared_public, derive_yubi_public_material,
    open_hybrid_box, open_puk_parcel, open_puk_parcel_for_role, open_puk_parcel_with,
    open_puk_parcel_with_for_role, open_puk_seed_chain, open_shared_key_parcel_with,
    open_shared_key_seed_chain, seal_initial_puk_box, seal_puk_seed_chain_box,
    seal_shared_key_boxes, seal_software_puk_boxes, seal_software_puk_boxes_mixed,
    seal_yubi_puk_boxes, yubi_mlkem_decapsulate, DevicePublicMaterial, HybridSecretDecapsulator,
    InitialPukBoxRandomness, PukBoxRandomness, SharedKeyBoxInput, SharedKeyDecapsulator,
    SoftwarePukBoxInput, SoftwarePukBoxSetRandomness, YubiDevice, YubiPublicMaterial,
    YubiPukBoxInput, YubiPukBoxSetRandomness,
};
use crate::kv::{
    bind_kv_dirent, derive_kv_keys, open_kv_chunk, open_kv_dirent_name, seal_kv_chunk,
    seal_kv_dirent_name, LARGE_FILE_SIZE_PAYLOAD_TYPE_ID,
};
use crate::primitives::{
    prefixed_hash, prefixed_hash_signable, seal_typed_secretbox, subchain_tree_location,
};
use crate::signatures::{
    sign_ed25519_blob, sign_ed25519_typed, sign_shared_key_blob, verify_blob, verify_typed,
};
use crate::team::{
    make_add_local_team_member_link, make_change_team_member_link,
    make_remove_local_team_member_link, make_single_owner_adhoc_team,
    make_single_owner_adhoc_team_yubi, make_single_owner_named_team, make_team_removal_proof,
    open_team_remote_member_view_token, open_team_removal_key, open_team_removal_key_for_member,
    seal_team_remote_member_view_token, team_removal_key_commitment, AdHocTeamInput,
    AddLocalTeamMemberInput, ChangeTeamMemberInput, NamedTeamInput, RemoveLocalTeamMemberInput,
    TeamPtkRotation, TeamRemovalKeyExpectation,
};
use crate::user::{
    make_backup_provision_link, make_software_eldest_link, make_software_provision_link,
    make_software_provision_link_from_backup, make_software_puk_rotation_link,
    make_software_revoke_link, make_yubi_puk_rotation_link, PukRotation, SoftwareEldestInput,
    SoftwareProvisionInput, UserMutationBase,
};
use crate::{Error, Result};
use foks_proto::{
    DeviceLabelNameAndCommitmentKey, DhPublicKey, EntityId, Hepk, KvDirent, KvEncryptedChunk,
    KvLargeFileMetadata, KvNodeId, KvParty, KvRoot, KvSmallFileBox, KvSmallFilePlaintext, Role,
    RoleAndGeneration, SecretBox, SecretSeed, SharedKeyBoxSet, SharedKeySeed, Signature,
    TeamRemovalBoxData, TeamRemovalKeyBox, LINK_OUTER_V1_TYPE_ID, SHARED_KEY_SEED_TYPE_ID,
};
use foks_proto::{
    ProbeResponse, PukParcel, TeamChain, UserChain, UserLink, ENTITY_PTK_VERIFY, ENTITY_PUK_VERIFY,
    PUBLIC_ZONE_BLOB_TYPE_ID,
};
use foks_snowpack::{decode, encode_ref, Value, ValueRef};
use p256::ecdh::diffie_hellman as p256_diffie_hellman;
use p256::ecdsa::Signature as P256Signature;
use p256::elliptic_curve::sec1::ToEncodedPoint as _;
use p256::{PublicKey as P256PublicKey, SecretKey as P256SecretKey};
use sha2::{Digest as _, Sha512_256};
use zeroize::Zeroizing;

const PROBE: &[u8] =
    include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp");
const USER_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";
const SIGNUP_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/signup";
const MUTATION_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

fn user_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{USER_DIR}/{name}")).unwrap()
}

fn signup_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{SIGNUP_DIR}/{name}")).unwrap()
}

fn mutation_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{MUTATION_DIR}/{name}")).unwrap()
}

struct MockYubi {
    public: YubiPublicMaterial,
    signing: p256::ecdsa::SigningKey,
    dh: P256SecretKey,
    pq_self_secret: [u8; 32],
}

impl MockYubi {
    fn new(signing_scalar: u8, pq_scalar: u8) -> Self {
        let signing_bytes = [signing_scalar; 32];
        let pq_bytes = [pq_scalar; 32];
        let signing = p256::ecdsa::SigningKey::from_bytes((&signing_bytes).into()).unwrap();
        let dh = P256SecretKey::from_slice(&signing_bytes).unwrap();
        let pq = P256SecretKey::from_slice(&pq_bytes).unwrap();
        let signing_public: [u8; 33] = signing
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .try_into()
            .unwrap();
        let pq_public: [u8; 33] = pq
            .public_key()
            .to_encoded_point(true)
            .as_bytes()
            .try_into()
            .unwrap();
        let pq_self = p256_diffie_hellman(pq.to_nonzero_scalar(), pq.public_key().as_affine());
        let pq_self_secret = pq_self.raw_secret_bytes().as_slice().try_into().unwrap();
        let public =
            derive_yubi_public_material(signing_public, pq_public, pq_self_secret).unwrap();
        Self {
            public,
            signing,
            dh,
            pq_self_secret,
        }
    }
}

impl HybridSecretDecapsulator for MockYubi {
    fn entity_id(&self) -> &EntityId {
        &self.public.device.id
    }

    fn hepk(&self) -> &Hepk {
        &self.public.device.hepk
    }

    fn derive_dh_shared(&self, peer: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
        let DhPublicKey::P256(peer) = peer else {
            return Err(Error::HybridBox);
        };
        let peer = P256PublicKey::from_sec1_bytes(peer).map_err(|_| Error::HybridBox)?;
        let shared = p256_diffie_hellman(self.dh.to_nonzero_scalar(), peer.as_affine());
        Ok(Zeroizing::new(
            shared.raw_secret_bytes().as_slice().try_into().unwrap(),
        ))
    }

    fn decapsulate_mlkem768(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        yubi_mlkem_decapsulate(self.pq_self_secret, ciphertext)
    }
}

impl YubiDevice for MockYubi {
    fn pq_key_id(&self) -> [u8; 32] {
        self.public.pq_key_id
    }

    fn sign_sha512_256(&self, digest: &[u8; 32]) -> Result<Vec<u8>> {
        use p256::ecdsa::signature::hazmat::PrehashSigner as _;

        let signature: P256Signature = self
            .signing
            .sign_prehash(digest)
            .map_err(|_| Error::YubiSigning)?;
        Ok(signature.to_der().as_bytes().to_vec())
    }
}

#[test]
fn subchain_location_matches_the_go_reference() {
    assert_eq!(
        subchain_tree_location(&[0x35; 32], foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP).unwrap(),
        [
            0x90, 0xde, 0x90, 0x44, 0xfc, 0xee, 0x5b, 0x28, 0x84, 0xe0, 0xd0, 0x4f, 0x7c, 0x71,
            0xd6, 0xaf, 0x83, 0xce, 0x04, 0x7b, 0x05, 0x1f, 0x56, 0xc2, 0x07, 0xb5, 0x0b, 0xdb,
            0x9d, 0x7f, 0x40, 0x96,
        ]
    );
}

#[test]
fn signing_and_verification_reject_a_non_canonical_signable_object() {
    // array16 with 16..=31 elements is canonical for RPC arguments (the
    // signup argument is array16(16)) but not for signed, verified, or
    // hashed objects, matching go-foks's AssertCanonicalMsgpack. Both the
    // signer and the verifier reject it before any signature operation.
    let mut non_canonical = vec![0xdc, 0x00, 0x10];
    non_canonical.extend(std::iter::repeat_n(0xc0, 16));
    let signer = EntityId::from_bytes([vec![ENTITY_PUK_VERIFY], vec![0x11; 32]].concat()).unwrap();
    assert!(matches!(
        sign_ed25519_typed(&[0u8; 32], 1, &non_canonical).unwrap_err(),
        Error::Snowpack(_)
    ));
    assert!(matches!(
        verify_typed(&signer, &Signature::Ed25519([0; 64]), 1, &non_canonical).unwrap_err(),
        Error::Snowpack(_)
    ));
    assert!(matches!(
        sign_ed25519_blob(&[0u8; 32], 1, &non_canonical).unwrap_err(),
        Error::Snowpack(_)
    ));
    assert!(matches!(
        sign_shared_key_blob(&SecretSeed::new([0; 32]), 1, &non_canonical).unwrap_err(),
        Error::Snowpack(_)
    ));
    assert!(matches!(
        verify_blob(&signer, &Signature::Ed25519([0; 64]), 1, &non_canonical,).unwrap_err(),
        Error::Snowpack(_)
    ));
    assert!(matches!(
        prefixed_hash_signable(1, &non_canonical).unwrap_err(),
        Error::Snowpack(_)
    ));

    // A fixarray-shaped signed object is still accepted by the signer (the
    // error path is specific to the disallowed array16 form).
    let canonical = vec![0x91, 0xc0];
    assert!(sign_ed25519_typed(&[0u8; 32], 1, &canonical).is_ok());
    assert!(sign_ed25519_blob(&[0u8; 32], 1, &canonical).is_ok());
    assert_eq!(
        prefixed_hash_signable(1, &canonical).unwrap(),
        prefixed_hash(1, &canonical)
    );
}

#[test]
fn software_eldest_link_matches_official_signup_fixture() {
    let expected = UserLink::decode(&signup_fixture("eldest-link.snowp")).unwrap();
    let opened = expected.decode_eldest().unwrap();
    let device_seed = SecretSeed::new(signup_fixture("device-seed.bin").try_into().unwrap());
    let puk_seed = SecretSeed::new(signup_fixture("puk-seed.bin").try_into().unwrap());
    let material = make_software_eldest_link(
        &SoftwareEldestInput {
            host: &opened.host,
            root: &opened.root,
            time: opened.time,
            next_tree_location: signup_fixture("next-tree-location.bin").try_into().unwrap(),
            subchain_tree_location: signup_fixture("subchain-tree-location.bin")
                .try_into()
                .unwrap(),
            normalized_username: b"signupfixture",
            username_sequence: 1,
            username_commitment_key: signup_fixture("username-commitment-key.bin")
                .try_into()
                .unwrap(),
            device_name: &DeviceLabelNameAndCommitmentKey {
                label: foks_proto::DeviceLabel {
                    device_type: foks_proto::DeviceType::Computer,
                    normalized_name: b"signup device".to_vec(),
                    serial: 1,
                },
                normalization_version: 0,
                display_name: b"signup device".to_vec(),
                commitment_key: signup_fixture("device-commitment-key.bin")
                    .try_into()
                    .unwrap(),
            },
        },
        &device_seed,
        &puk_seed,
    )
    .unwrap();
    assert_eq!(
        material.link.encoded().unwrap(),
        expected.encoded().unwrap()
    );
    let expected_uid = match decode(&signup_fixture("uid.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        other => panic!("expected UID fixture, got {other:?}"),
    };
    assert_eq!(material.uid, expected_uid);
}

#[test]
fn software_device_mutations_match_official_user_fixtures() {
    let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
    let expected_provision = UserLink::decode(&user_fixture("user-provision-link.snowp")).unwrap();
    let provision_change = expected_provision.decode_group_change().unwrap();
    let existing_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let new_seed = SecretSeed::new(user_fixture("second-device-seed.bin").try_into().unwrap());
    let disclosed = &chain.device_names[1];
    let provision = make_software_provision_link(
        &SoftwareProvisionInput {
            base: UserMutationBase {
                uid: &provision_change.uid,
                host: &provision_change.host,
                seqno: provision_change.seqno,
                previous: provision_change.previous.unwrap(),
                root: &provision_change.root,
                time: provision_change.time,
                next_tree_location: chain.locations[1],
            },
            role: Role::OWNER,
            device_label: &disclosed.label,
            device_name_commitment_key: disclosed.commitment_key,
        },
        &existing_seed,
        &new_seed,
        None,
    )
    .unwrap();
    assert_eq!(
        provision.link.encoded().unwrap(),
        expected_provision.encoded().unwrap()
    );

    let expected_revoke = UserLink::decode(&user_fixture("user-revoke-link.snowp")).unwrap();
    let revoke_change = expected_revoke.decode_group_change().unwrap();
    let rotated = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let revoke = make_software_revoke_link(
        &UserMutationBase {
            uid: &revoke_change.uid,
            host: &revoke_change.host,
            seqno: revoke_change.seqno,
            previous: revoke_change.previous.unwrap(),
            root: &revoke_change.root,
            time: revoke_change.time,
            next_tree_location: chain.locations[2],
        },
        &existing_seed,
        &provision.device.id,
        &[PukRotation {
            role: Role::OWNER,
            generation: 2,
            seed: &rotated,
        }],
    )
    .unwrap();
    assert_eq!(
        revoke.encoded().unwrap(),
        expected_revoke.encoded().unwrap()
    );
}

#[test]
fn standalone_puk_rotation_matches_official_go_fixture() {
    let expected = UserLink::decode(&mutation_fixture("rotation-link.snowp")).unwrap();
    let change = expected.decode_group_change().unwrap();
    assert!(change.changes.is_empty());
    let signer_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let rotation_seed = SecretSeed::new(
        mutation_fixture("rotation-puk-seed.bin")
            .try_into()
            .unwrap(),
    );
    let actual = make_software_puk_rotation_link(
        &UserMutationBase {
            uid: &change.uid,
            host: &change.host,
            seqno: change.seqno,
            previous: change.previous.unwrap(),
            root: &change.root,
            time: change.time,
            next_tree_location: mutation_fixture("rotation-next-tree-location.bin")
                .try_into()
                .unwrap(),
        },
        &signer_seed,
        &[PukRotation {
            role: Role::OWNER,
            generation: 3,
            seed: &rotation_seed,
        }],
    )
    .unwrap();
    assert_eq!(actual.encoded().unwrap(), expected.encoded().unwrap());
    assert!(make_software_puk_rotation_link(
        &UserMutationBase {
            uid: &change.uid,
            host: &change.host,
            seqno: change.seqno,
            previous: change.previous.unwrap(),
            root: &change.root,
            time: change.time,
            next_tree_location: [0; 32],
        },
        &signer_seed,
        &[],
    )
    .is_err());
}

#[test]
fn yubi_puk_rotation_stacks_rotated_keys_before_the_parent() {
    let expected = UserLink::decode(&mutation_fixture("rotation-link.snowp")).unwrap();
    let base_change = expected.decode_group_change().unwrap();
    let parent = MockYubi::new(0x11, 0x12);
    let member_seed = SecretSeed::new([0x21; 32]);
    let owner_seed = SecretSeed::new([0x22; 32]);
    let rotations = [
        PukRotation {
            role: Role::member(0),
            generation: 2,
            seed: &member_seed,
        },
        PukRotation {
            role: Role::OWNER,
            generation: 3,
            seed: &owner_seed,
        },
    ];
    let base = UserMutationBase {
        uid: &base_change.uid,
        host: &base_change.host,
        seqno: base_change.seqno,
        previous: base_change.previous.unwrap(),
        root: &base_change.root,
        time: base_change.time,
        next_tree_location: [0x31; 32],
    };
    let link = make_yubi_puk_rotation_link(&base, &parent, &rotations).unwrap();
    assert_eq!(
        link.encoded().unwrap(),
        make_yubi_puk_rotation_link(&base, &parent, &rotations)
            .unwrap()
            .encoded()
            .unwrap()
    );
    let change = link.decode_group_change().unwrap();
    assert_eq!(change.signer, *parent.entity_id());
    assert!(change.changes.is_empty());
    assert_eq!(
        change
            .shared_keys
            .iter()
            .map(|key| (key.role, key.generation))
            .collect::<Vec<_>>(),
        vec![(Role::member(0), 2), (Role::OWNER, 3)]
    );
    assert_eq!(link.signatures().len(), 3);
    let member = derive_shared_public(&member_seed, ENTITY_PUK_VERIFY).unwrap();
    let owner = derive_shared_public(&owner_seed, ENTITY_PUK_VERIFY).unwrap();
    verify_typed(
        &member.verify_key,
        &link.signatures()[0],
        LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(0).unwrap(),
    )
    .unwrap();
    verify_typed(
        &owner.verify_key,
        &link.signatures()[1],
        LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(1).unwrap(),
    )
    .unwrap();
    verify_typed(
        parent.entity_id(),
        &link.signatures()[2],
        LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(2).unwrap(),
    )
    .unwrap();
    assert!(verify_typed(
        parent.entity_id(),
        &link.signatures()[2],
        LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(1).unwrap(),
    )
    .is_err());

    let mut impostor = MockYubi::new(0x13, 0x14);
    impostor.public = parent.public.clone();
    assert!(matches!(
        make_yubi_puk_rotation_link(&base, &impostor, &rotations),
        Err(Error::Verification)
    ));
    assert!(make_yubi_puk_rotation_link(&base, &parent, &[]).is_err());
}

#[test]
fn yubi_puk_boxes_round_trip_without_temp_dh_and_reject_wrong_bindings() {
    let host =
        EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x41; 32]].concat()).unwrap();
    let wrong_host =
        EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x42; 32]].concat()).unwrap();
    let parent = MockYubi::new(0x31, 0x32);
    let receiver_a = MockYubi::new(0x33, 0x34);
    let receiver_b = MockYubi::new(0x35, 0x36);
    let wrong_parent = MockYubi::new(0x37, 0x38);
    let member_seed = SecretSeed::new([0x51; 32]);
    let owner_seed = SecretSeed::new([0x52; 32]);
    let inputs = [
        YubiPukBoxInput {
            seed: &member_seed,
            generation: 2,
            role: Role::member(0),
            receiver: &receiver_a.public.device,
        },
        YubiPukBoxInput {
            seed: &owner_seed,
            generation: 3,
            role: Role::OWNER,
            receiver: &receiver_b.public.device,
        },
    ];
    let randomness = [
        PukBoxRandomness {
            kem_message: [0x61; 32],
            nonce: [0x62; 16],
        },
        PukBoxRandomness {
            kem_message: [0x63; 32],
            nonce: [0x64; 16],
        },
    ];
    let set_randomness = YubiPukBoxSetRandomness {
        ephemeral_secret: [0x65; 32],
        time: 1_724_000_000_000,
    };
    let boxes = seal_yubi_puk_boxes(
        &host,
        &parent,
        [0x71; 16],
        &inputs,
        &randomness,
        set_randomness,
    )
    .unwrap();
    let repeated = seal_yubi_puk_boxes(
        &host,
        &parent,
        [0x71; 16],
        &inputs,
        &randomness,
        YubiPukBoxSetRandomness {
            ephemeral_secret: [0x65; 32],
            time: 1_724_000_000_000,
        },
    )
    .unwrap();
    assert_eq!(boxes.encoded(), repeated.encoded());
    assert!(boxes.temp_dh_key.is_none());
    assert!(boxes
        .boxes
        .iter()
        .all(|boxed| boxed.hybrid.dh_type == 2 && boxed.hybrid.sender_dh.is_none()));

    let member_public = derive_shared_public(&member_seed, ENTITY_PUK_VERIFY).unwrap();
    let owner_public = derive_shared_public(&owner_seed, ENTITY_PUK_VERIFY).unwrap();
    let member_parcel =
        PukParcel::from_box_set(&boxes, 0, parent.entity_id().clone(), Vec::new()).unwrap();
    let owner_parcel =
        PukParcel::from_box_set(&boxes, 1, parent.entity_id().clone(), Vec::new()).unwrap();
    let opened_member = open_puk_parcel_with_for_role(
        &member_parcel,
        &receiver_a,
        parent.hepk(),
        &member_public.verify_key,
        &member_public.hepk,
        2,
        &host,
        Role::member(0),
    )
    .unwrap();
    let opened_owner = open_puk_parcel_with_for_role(
        &owner_parcel,
        &receiver_b,
        parent.hepk(),
        &owner_public.verify_key,
        &owner_public.hepk,
        3,
        &host,
        Role::OWNER,
    )
    .unwrap();
    assert_eq!(opened_member.seed, member_seed);
    assert_eq!(opened_owner.seed, owner_seed);

    assert!(open_puk_parcel_with_for_role(
        &member_parcel,
        &receiver_b,
        parent.hepk(),
        &member_public.verify_key,
        &member_public.hepk,
        2,
        &host,
        Role::member(0),
    )
    .is_err());
    let mut rebound_receiver = member_parcel.clone();
    rebound_receiver.target = receiver_b.entity_id().clone();
    assert!(open_puk_parcel_with_for_role(
        &rebound_receiver,
        &receiver_b,
        parent.hepk(),
        &member_public.verify_key,
        &member_public.hepk,
        2,
        &host,
        Role::member(0),
    )
    .is_err());
    assert!(open_puk_parcel_with_for_role(
        &member_parcel,
        &receiver_a,
        wrong_parent.hepk(),
        &member_public.verify_key,
        &member_public.hepk,
        2,
        &host,
        Role::member(0),
    )
    .is_err());
    assert!(matches!(
        open_puk_parcel_with_for_role(
            &member_parcel,
            &receiver_a,
            parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &wrong_host,
            Role::member(0),
        ),
        Err(Error::PukBinding)
    ));
    let mut rebound_generation = member_parcel.clone();
    rebound_generation.generation = 4;
    assert!(matches!(
        open_puk_parcel_with_for_role(
            &rebound_generation,
            &receiver_a,
            parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            4,
            &host,
            Role::member(0),
        ),
        Err(Error::PukBinding)
    ));
    let mut rebound_role = member_parcel.clone();
    rebound_role.role = Role::ADMIN;
    assert!(matches!(
        open_puk_parcel_with_for_role(
            &rebound_role,
            &receiver_a,
            parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &host,
            Role::ADMIN,
        ),
        Err(Error::PukBinding)
    ));
    let wrong_puk = derive_shared_public(&SecretSeed::new([0x53; 32]), ENTITY_PUK_VERIFY).unwrap();
    assert!(matches!(
        open_puk_parcel_with_for_role(
            &member_parcel,
            &receiver_a,
            parent.hepk(),
            &wrong_puk.verify_key,
            &wrong_puk.hepk,
            2,
            &host,
            Role::member(0),
        ),
        Err(Error::PukBinding)
    ));
}

#[test]
fn yubi_puk_boxes_round_trip_for_mixed_recipient_curves() {
    let host =
        EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x43; 32]].concat()).unwrap();
    let parent = MockYubi::new(0x39, 0x3a);
    let yubi_receiver = MockYubi::new(0x3b, 0x3c);
    let software_seed = SecretSeed::new([0x3d; 32]);
    let software_receiver = derive_device_public(&software_seed).unwrap();
    let member_seed = SecretSeed::new([0x54; 32]);
    let owner_seed = SecretSeed::new([0x55; 32]);
    let inputs = [
        YubiPukBoxInput {
            seed: &member_seed,
            generation: 2,
            role: Role::member(0),
            receiver: &software_receiver,
        },
        YubiPukBoxInput {
            seed: &owner_seed,
            generation: 3,
            role: Role::OWNER,
            receiver: &yubi_receiver.public.device,
        },
    ];
    let boxes = seal_yubi_puk_boxes(
        &host,
        &parent,
        [0x72; 16],
        &inputs,
        &[
            PukBoxRandomness {
                kem_message: [0x66; 32],
                nonce: [0x67; 16],
            },
            PukBoxRandomness {
                kem_message: [0x68; 32],
                nonce: [0x69; 16],
            },
        ],
        YubiPukBoxSetRandomness {
            ephemeral_secret: [0x6a; 32],
            time: 1_724_000_000_001,
        },
    )
    .unwrap();
    assert!(boxes.temp_dh_key.is_some());
    assert_eq!(boxes.boxes[0].hybrid.dh_type, 1);
    assert_eq!(boxes.boxes[1].hybrid.dh_type, 2);

    let member_public = derive_shared_public(&member_seed, ENTITY_PUK_VERIFY).unwrap();
    let owner_public = derive_shared_public(&owner_seed, ENTITY_PUK_VERIFY).unwrap();
    let software_parcel =
        PukParcel::from_box_set(&boxes, 0, parent.entity_id().clone(), Vec::new()).unwrap();
    let yubi_parcel =
        PukParcel::from_box_set(&boxes, 1, parent.entity_id().clone(), Vec::new()).unwrap();
    assert_eq!(
        open_puk_parcel_for_role(
            &software_parcel,
            &software_seed,
            parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &host,
            Role::member(0),
        )
        .unwrap()
        .seed,
        member_seed
    );
    assert_eq!(
        open_puk_parcel_with_for_role(
            &yubi_parcel,
            &yubi_receiver,
            parent.hepk(),
            &owner_public.verify_key,
            &owner_public.hepk,
            3,
            &host,
            Role::OWNER,
        )
        .unwrap()
        .seed,
        owner_seed
    );

    let mut tampered = software_parcel.clone();
    let Signature::Ecdsa(signature) = &mut tampered
        .temp_dh_key
        .as_mut()
        .expect("mixed set has a temporary key")
        .signature
    else {
        panic!("Yubi temporary key has the wrong signature type")
    };
    signature[0] ^= 1;
    assert!(open_puk_parcel_for_role(
        &tampered,
        &software_seed,
        parent.hepk(),
        &member_public.verify_key,
        &member_public.hepk,
        2,
        &host,
        Role::member(0),
    )
    .is_err());
    // Go ignores the set-level temporary key for same-curve boxes.
    let mut same_curve = yubi_parcel;
    same_curve.temp_dh_key = tampered.temp_dh_key;
    assert!(open_puk_parcel_with_for_role(
        &same_curve,
        &yubi_receiver,
        parent.hepk(),
        &owner_public.verify_key,
        &owner_public.hepk,
        3,
        &host,
        Role::OWNER,
    )
    .is_ok());
}

#[test]
fn mixed_puk_boxers_support_backup_and_bot_token_recipients() {
    let host =
        EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x4a; 32]].concat()).unwrap();
    let software_seed = SecretSeed::new([0x4b; 32]);
    let backup = BackupKey::from_seed([0x01; BACKUP_SEED_BYTES])
        .unwrap()
        .into_key_material()
        .unwrap();
    let backup_public = DevicePublicMaterial {
        id: backup.entity_id().clone(),
        hepk: backup.hepk().clone(),
    };
    let bot_id =
        EntityId::from_bytes([vec![foks_proto::ENTITY_BOT_TOKEN_KEY], vec![0x4c; 32]].concat())
            .unwrap();
    let bot_public = DevicePublicMaterial {
        id: bot_id,
        hepk: derive_device_public(&SecretSeed::new([0x4d; 32]))
            .unwrap()
            .hepk,
    };
    let puk_seed = SecretSeed::new([0x58; 32]);
    let inputs = [&backup_public, &bot_public].map(|receiver| SoftwarePukBoxInput {
        seed: &puk_seed,
        generation: 2,
        role: Role::OWNER,
        receiver,
    });
    let software_boxes = seal_software_puk_boxes_mixed(
        &host,
        &software_seed,
        [0x74; 16],
        &inputs,
        &[
            PukBoxRandomness {
                kem_message: [0x70; 32],
                nonce: [0x71; 16],
            },
            PukBoxRandomness {
                kem_message: [0x72; 32],
                nonce: [0x73; 16],
            },
        ],
        SoftwarePukBoxSetRandomness {
            ephemeral_secret: [0x75; 32],
            time: 1_724_000_000_003,
        },
    )
    .unwrap();
    assert!(software_boxes.temp_dh_key.is_none());

    let parent = MockYubi::new(0x4e, 0x4f);
    let yubi_boxes = seal_yubi_puk_boxes(
        &host,
        &parent,
        [0x76; 16],
        &[YubiPukBoxInput {
            seed: &puk_seed,
            generation: 2,
            role: Role::OWNER,
            receiver: &backup_public,
        }],
        &[PukBoxRandomness {
            kem_message: [0x77; 32],
            nonce: [0x78; 16],
        }],
        YubiPukBoxSetRandomness {
            ephemeral_secret: [0x79; 32],
            time: 1_724_000_000_004,
        },
    )
    .unwrap();
    assert!(yubi_boxes.temp_dh_key.is_some());
    let public_puk = derive_shared_public(&puk_seed, ENTITY_PUK_VERIFY).unwrap();
    let parcel =
        PukParcel::from_box_set(&yubi_boxes, 0, parent.entity_id().clone(), Vec::new()).unwrap();
    assert_eq!(
        open_puk_parcel_with_for_role(
            &parcel,
            &backup,
            parent.hepk(),
            &public_puk.verify_key,
            &public_puk.hepk,
            2,
            &host,
            Role::OWNER,
        )
        .unwrap()
        .seed,
        puk_seed
    );
}

#[test]
fn software_puk_boxes_round_trip_for_mixed_recipient_curves() {
    let host =
        EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x44; 32]].concat()).unwrap();
    let sender_seed = SecretSeed::new([0x45; 32]);
    let sender = derive_device_public(&sender_seed).unwrap();
    let software_seed = SecretSeed::new([0x46; 32]);
    let software_receiver = derive_device_public(&software_seed).unwrap();
    let yubi_receiver = MockYubi::new(0x47, 0x48);
    let member_seed = SecretSeed::new([0x56; 32]);
    let owner_seed = SecretSeed::new([0x57; 32]);
    let inputs = [
        SoftwarePukBoxInput {
            seed: &member_seed,
            generation: 2,
            role: Role::member(0),
            receiver: &software_receiver,
        },
        SoftwarePukBoxInput {
            seed: &owner_seed,
            generation: 3,
            role: Role::OWNER,
            receiver: &yubi_receiver.public.device,
        },
    ];
    let boxes = seal_software_puk_boxes_mixed(
        &host,
        &sender_seed,
        [0x73; 16],
        &inputs,
        &[
            PukBoxRandomness {
                kem_message: [0x6b; 32],
                nonce: [0x6c; 16],
            },
            PukBoxRandomness {
                kem_message: [0x6d; 32],
                nonce: [0x6e; 16],
            },
        ],
        SoftwarePukBoxSetRandomness {
            ephemeral_secret: [0x6f; 32],
            time: 1_724_000_000_002,
        },
    )
    .unwrap();
    assert!(matches!(
        boxes.temp_dh_key.as_ref().map(|key| &key.signature),
        Some(Signature::Ed25519(_))
    ));

    let member_public = derive_shared_public(&member_seed, ENTITY_PUK_VERIFY).unwrap();
    let owner_public = derive_shared_public(&owner_seed, ENTITY_PUK_VERIFY).unwrap();
    let software_parcel =
        PukParcel::from_box_set(&boxes, 0, sender.id.clone(), Vec::new()).unwrap();
    let yubi_parcel = PukParcel::from_box_set(&boxes, 1, sender.id.clone(), Vec::new()).unwrap();
    assert_eq!(
        open_puk_parcel_for_role(
            &software_parcel,
            &software_seed,
            &sender.hepk,
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &host,
            Role::member(0),
        )
        .unwrap()
        .seed,
        member_seed
    );
    assert_eq!(
        open_puk_parcel_with_for_role(
            &yubi_parcel,
            &yubi_receiver,
            &sender.hepk,
            &owner_public.verify_key,
            &owner_public.hepk,
            3,
            &host,
            Role::OWNER,
        )
        .unwrap()
        .seed,
        owner_seed
    );
}

#[test]
fn single_owner_adhoc_team_matches_official_go_fixture() {
    let expected = UserLink::decode(&mutation_fixture("adhoc-team-link.snowp")).unwrap();
    let change = expected.decode_team_group_change().unwrap();
    let membership = UserLink::decode(&mutation_fixture("adhoc-membership-link.snowp")).unwrap();
    let device_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let owner_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let ptk_seeds = [
        "adhoc-ptk-member-min-seed.bin",
        "adhoc-ptk-member-seed.bin",
        "adhoc-ptk-admin-seed.bin",
        "adhoc-ptk-owner-seed.bin",
    ]
    .map(|name| SecretSeed::new(mutation_fixture(name).try_into().unwrap()));
    let owner = &change.changes[0];
    let material = make_single_owner_adhoc_team(
        &AdHocTeamInput {
            user: &owner.party,
            host: &change.host,
            root: &change.root,
            time: change.time,
            owner_puk_generation: owner.keys.as_ref().unwrap().generation,
            membership_sequence: 1,
            membership_previous: None,
            next_tree_location: mutation_fixture("adhoc-next-tree-location.bin")
                .try_into()
                .unwrap(),
            subchain_tree_location: mutation_fixture("adhoc-subchain-tree-location.bin")
                .try_into()
                .unwrap(),
            membership_next_tree_location: mutation_fixture(
                "adhoc-membership-next-tree-location.bin",
            )
            .try_into()
            .unwrap(),
        },
        &device_seed,
        &owner_seed,
        [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3]],
    )
    .unwrap();
    assert_eq!(
        material.link.encoded().unwrap(),
        expected.encoded().unwrap()
    );
    assert_eq!(
        material.membership_link.encoded().unwrap(),
        membership.encoded().unwrap()
    );
    let expected_team = match decode(&mutation_fixture("adhoc-team-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        other => panic!("expected ad-hoc TeamID fixture, got {other:?}"),
    };
    assert_eq!(material.team, expected_team);

    let owner_public = derive_shared_public(&owner_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let roles = [
        Role::member(-0x4000),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ];
    let inputs = ptk_seeds
        .iter()
        .zip(roles)
        .map(|(seed, role)| SharedKeyBoxInput {
            seed,
            generation: 1,
            role,
            receiver_id: &owner.party,
            receiver_host: None,
            receiver_hepk: &owner_public.hepk,
            receiver_role: Role::OWNER,
            receiver_generation: owner.keys.as_ref().unwrap().generation,
        })
        .collect::<Vec<_>>();
    let randomness = (0_u8..4)
        .map(|offset| PukBoxRandomness {
            kem_message: [31 + offset; 32],
            nonce: [41 + offset; 16],
        })
        .collect::<Vec<_>>();
    let boxes = seal_shared_key_boxes(
        &change.host,
        &owner_seed,
        &owner_public.hepk,
        [51; 16],
        &inputs,
        &randomness,
    )
    .unwrap();
    let receiver = SharedKeyDecapsulator::new(&owner_seed, owner.party.clone()).unwrap();
    for (index, shared) in boxes.boxes.iter().enumerate() {
        let parcel = PukParcel {
            generation: shared.generation,
            role: shared.role,
            hybrid: shared.hybrid.clone(),
            target: shared.target.entity.clone(),
            target_host: shared.target.host.clone(),
            target_role: shared.target.role,
            target_generation: shared.target.generation,
            sender: owner_public.verify_key.clone(),
            box_id: boxes.box_id,
            temp_dh_key: boxes.temp_dh_key.clone(),
            seed_chain: Vec::new(),
        };
        let clear = open_shared_key_parcel_with(
            &parcel,
            &receiver,
            &owner_public.hepk,
            &material.ptks[index].verify_key,
            &material.ptks[index].hepk,
            shared.generation,
            &change.host,
            Role::OWNER,
            owner.keys.as_ref().unwrap().generation,
            roles[index],
            ENTITY_PTK_VERIFY,
        )
        .unwrap();
        assert_eq!(clear.seed, ptk_seeds[index]);
    }
    let wrong_sender = derive_shared_public(&ptk_seeds[0], ENTITY_PTK_VERIFY).unwrap();
    assert!(seal_shared_key_boxes(
        &change.host,
        &owner_seed,
        &wrong_sender.hepk,
        [51; 16],
        &inputs,
        &randomness,
    )
    .is_err());
    assert!(make_single_owner_adhoc_team(
        &AdHocTeamInput {
            user: &owner.party,
            host: &change.host,
            root: &change.root,
            time: change.time,
            owner_puk_generation: 0,
            membership_sequence: 1,
            membership_previous: None,
            next_tree_location: [1; 32],
            subchain_tree_location: [1; 32],
            membership_next_tree_location: [1; 32],
        },
        &device_seed,
        &owner_seed,
        [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3],],
    )
    .is_err());
}

#[test]
fn single_owner_named_team_matches_official_go_fixture() {
    let expected = UserLink::decode(&mutation_fixture("named-team-link.snowp")).unwrap();
    let change = expected.decode_team_group_change().unwrap();
    let expected_membership =
        UserLink::decode(&mutation_fixture("named-membership-link.snowp")).unwrap();
    let device_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let owner_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let ptk_seeds = [
        "adhoc-ptk-member-min-seed.bin",
        "adhoc-ptk-member-seed.bin",
        "adhoc-ptk-admin-seed.bin",
        "adhoc-ptk-owner-seed.bin",
    ]
    .map(|name| SecretSeed::new(mutation_fixture(name).try_into().unwrap()));
    let removal_key = SecretSeed::new(
        mutation_fixture("named-removal-key.bin")
            .try_into()
            .unwrap(),
    );
    let owner = &change.changes[0];
    let material = make_single_owner_named_team(
        &NamedTeamInput {
            user: &owner.party,
            host: &change.host,
            root: &change.root,
            time: change.time,
            owner_puk_generation: owner.keys.as_ref().unwrap().generation,
            membership_sequence: 1,
            membership_previous: None,
            normalized_name: b"auditteam",
            name_sequence: 7,
            team_name_commitment_key: mutation_fixture("named-team-name-commitment-key.bin")
                .try_into()
                .unwrap(),
            next_tree_location: mutation_fixture("named-next-tree-location.bin")
                .try_into()
                .unwrap(),
            subchain_tree_location: mutation_fixture("named-subchain-tree-location.bin")
                .try_into()
                .unwrap(),
            membership_next_tree_location: mutation_fixture(
                "named-membership-next-tree-location.bin",
            )
            .try_into()
            .unwrap(),
        },
        &device_seed,
        &owner_seed,
        [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3]],
        &removal_key,
    )
    .unwrap();
    assert_eq!(
        material.link.encoded().unwrap(),
        expected.encoded().unwrap()
    );
    assert_eq!(
        material.membership_link.encoded().unwrap(),
        expected_membership.encoded().unwrap()
    );
    let expected_team = match decode(&mutation_fixture("named-team-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        other => panic!("expected named TeamID fixture, got {other:?}"),
    };
    assert_eq!(material.team, expected_team);
    assert_eq!(
        material.removal_key_commitment,
        owner.keys.as_ref().unwrap().removal_key_commitment.unwrap()
    );
    let removal_box =
        TeamRemovalBoxData::decode(&mutation_fixture("named-removal-boxes.snowp")).unwrap();
    let member_receiver = SharedKeyDecapsulator::new(&owner_seed, owner.party.clone()).unwrap();
    let opened_member = open_team_removal_key(
        &removal_box.member_box,
        &member_receiver,
        Role::OWNER,
        owner.keys.as_ref().unwrap().generation,
        &removal_box.commitment,
        &removal_box.metadata,
    )
    .unwrap();
    let team_receiver = SharedKeyDecapsulator::new(&ptk_seeds[2], material.team.clone()).unwrap();
    let opened_team = open_team_removal_key(
        &removal_box.team_box,
        &team_receiver,
        Role::ADMIN,
        1,
        &removal_box.commitment,
        &removal_box.metadata,
    )
    .unwrap();
    assert_eq!(opened_member, removal_key);
    assert_eq!(opened_team, removal_key);
}

#[test]
fn team_admin_opens_official_member_removal_key_box() {
    let addition = UserLink::decode(&mutation_fixture("add-member-link.snowp"))
        .unwrap()
        .decode_team_group_change()
        .unwrap();
    let member = &addition.changes[0];
    let commitment = member
        .keys
        .as_ref()
        .unwrap()
        .removal_key_commitment
        .unwrap();
    let admin_seed = SecretSeed::new(
        mutation_fixture("adhoc-ptk-admin-seed.bin")
            .try_into()
            .unwrap(),
    );
    let receiver = SharedKeyDecapsulator::new(&admin_seed, addition.team.clone()).unwrap();
    let boxed =
        TeamRemovalKeyBox::decode(&mutation_fixture("team-removal-admin-box.snowp")).unwrap();
    let (opened, metadata) = open_team_removal_key_for_member(
        &boxed,
        &receiver,
        &TeamRemovalKeyExpectation {
            commitment: &commitment,
            team: &addition.team,
            host: &addition.host,
            member: &member.party,
            member_host: &addition.host,
            source_role: member.source_role,
        },
    )
    .unwrap();
    assert_eq!(
        opened.as_slice(),
        mutation_fixture("add-member-removal-key.bin")
    );
    assert_eq!(metadata.destination_role, member.role);
    assert_eq!(metadata.team_sequence, addition.seqno);
}

#[test]
fn federated_shared_key_box_binds_remote_host_in_target_and_plaintext() {
    let sender_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let receiver_seed = SecretSeed::new(
        mutation_fixture("add-member-target-puk-seed.bin")
            .try_into()
            .unwrap(),
    );
    let sender = derive_shared_public(&sender_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let receiver = derive_shared_public(&receiver_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let receiver_id = match decode(&mutation_fixture("add-member-target-uid.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        other => panic!("expected target user fixture, got {other:?}"),
    };
    let local_host = UserLink::decode(&mutation_fixture("named-team-link.snowp"))
        .unwrap()
        .decode_team_group_change()
        .unwrap()
        .host;
    let mut remote_bytes = local_host.as_bytes().to_vec();
    remote_bytes[32] ^= 1;
    let remote_host = EntityId::from_bytes(remote_bytes).unwrap();
    let shared_seed = SecretSeed::new([0x55; 32]);
    let boxes = seal_shared_key_boxes(
        &local_host,
        &sender_seed,
        &sender.hepk,
        [0x33; 16],
        &[SharedKeyBoxInput {
            seed: &shared_seed,
            generation: 4,
            role: Role::member(0),
            receiver_id: &receiver_id,
            receiver_host: Some(&remote_host),
            receiver_hepk: &receiver.hepk,
            receiver_role: Role::OWNER,
            receiver_generation: 3,
        }],
        &[PukBoxRandomness {
            kem_message: [0x44; 32],
            nonce: [0x22; 16],
        }],
    )
    .unwrap();
    assert_eq!(boxes.boxes[0].target.host, Some(remote_host.clone()));
    let decapsulator = SharedKeyDecapsulator::new(&receiver_seed, receiver_id).unwrap();
    let sender_dh = sender.hepk.curve25519().copied().unwrap();
    let plaintext = open_hybrid_box(
        &boxes.boxes[0].hybrid,
        &decapsulator,
        &DhPublicKey::Curve25519(sender_dh),
        SHARED_KEY_SEED_TYPE_ID,
    )
    .unwrap();
    let clear = SharedKeySeed::decode(&plaintext).unwrap();
    assert_eq!(clear.host, remote_host);
    assert_eq!(clear.seed, shared_seed);
}

#[test]
fn additive_named_team_link_matches_official_go_fixture() {
    let eldest = UserLink::decode(&mutation_fixture("named-team-link.snowp")).unwrap();
    let expected = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
    let change = expected.decode_team_group_change().unwrap();
    let actor_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let target_seed = SecretSeed::new(
        mutation_fixture("add-member-target-puk-seed.bin")
            .try_into()
            .unwrap(),
    );
    let target = derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let removal_key = SecretSeed::new(
        mutation_fixture("add-member-removal-key.bin")
            .try_into()
            .unwrap(),
    );
    let member = &change.changes[0];
    let material = make_add_local_team_member_link(
        &AddLocalTeamMemberInput {
            actor: &change.signer_owner.party,
            actor_source_role: change.signer_owner.source_role,
            team: &change.team,
            host: &change.host,
            sequence: change.seqno,
            previous: prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &eldest.encoded().unwrap()),
            root: &change.root,
            time: change.time,
            next_tree_location: mutation_fixture("add-member-next-tree-location.bin")
                .try_into()
                .unwrap(),
            member: &member.party,
            member_source_role: member.source_role,
            member_destination_role: member.role,
            member_generation: member.keys.as_ref().unwrap().generation,
            member_public: &target,
        },
        &actor_seed,
        &removal_key,
    )
    .unwrap();
    assert_eq!(material.link.decode_team_group_change().unwrap(), change);
    assert_eq!(
        material.link.encoded().unwrap(),
        expected.encoded().unwrap()
    );
    assert_eq!(
        material.removal_key_commitment,
        member
            .keys
            .as_ref()
            .unwrap()
            .removal_key_commitment
            .unwrap()
    );
    assert_eq!(
        material.removal_key_commitment,
        team_removal_key_commitment(&removal_key).unwrap()
    );
}

#[test]
fn named_team_demotion_matches_official_go_fixture() {
    let addition = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
    let expected = UserLink::decode(&mutation_fixture("demote-member-link.snowp")).unwrap();
    let change = expected.decode_team_group_change().unwrap();
    let member = &change.changes[0];
    let actor_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let target_seed = SecretSeed::new(
        mutation_fixture("add-member-target-puk-seed.bin")
            .try_into()
            .unwrap(),
    );
    let target = derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let rotation_seed = SecretSeed::new(
        mutation_fixture("demote-member-ptk-seed.bin")
            .try_into()
            .unwrap(),
    );
    let rotations = [TeamPtkRotation {
        role: Role::member(0),
        generation: 2,
        seed: &rotation_seed,
    }];
    let material = make_change_team_member_link(
        &ChangeTeamMemberInput {
            actor: &change.signer_owner.party,
            actor_source_role: change.signer_owner.source_role,
            team: &change.team,
            host: &change.host,
            sequence: change.seqno,
            previous: prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &addition.encoded().unwrap()),
            root: &change.root,
            time: change.time,
            next_tree_location: mutation_fixture("demote-member-next-tree-location.bin")
                .try_into()
                .unwrap(),
            member: &member.party,
            member_host: member.scoped_host.as_ref(),
            member_source_role: member.source_role,
            destination_role: member.role,
            member_generation: Some(member.keys.as_ref().unwrap().generation),
            member_public: Some(&target),
            member_index_range: None,
        },
        &actor_seed,
        &rotations,
    )
    .unwrap();
    assert_eq!(
        material.link.encoded().unwrap(),
        expected.encoded().unwrap()
    );
    assert_eq!(material.link.decode_team_group_change().unwrap(), change);
}

#[test]
fn role_only_team_promotion_needs_no_ptk_rotation() {
    let addition = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
    let prior = addition.decode_team_group_change().unwrap();
    let member = &prior.changes[0];
    let actor_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let target_seed = SecretSeed::new(
        mutation_fixture("add-member-target-puk-seed.bin")
            .try_into()
            .unwrap(),
    );
    let target = derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let material = make_change_team_member_link(
        &ChangeTeamMemberInput {
            actor: &prior.signer_owner.party,
            actor_source_role: prior.signer_owner.source_role,
            team: &prior.team,
            host: &prior.host,
            sequence: prior.seqno + 1,
            previous: prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &addition.encoded().unwrap()),
            root: &prior.root,
            time: prior.time + 1,
            next_tree_location: [7; 32],
            member: &member.party,
            member_host: member.scoped_host.as_ref(),
            member_source_role: member.source_role,
            destination_role: Role::OWNER,
            member_generation: Some(member.keys.as_ref().unwrap().generation),
            member_public: Some(&target),
            member_index_range: None,
        },
        &actor_seed,
        &[],
    )
    .unwrap();
    let change = material.link.decode_team_group_change().unwrap();
    assert!(change.shared_keys.is_empty());
    assert_eq!(change.changes[0].role, Role::OWNER);
    assert_eq!(material.link.signatures().len(), 1);
}

#[test]
fn named_team_removal_rotation_matches_official_go_fixture() {
    let addition = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
    let expected = UserLink::decode(&mutation_fixture("remove-member-link.snowp")).unwrap();
    let change = expected.decode_team_group_change().unwrap();
    let actor_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let old_seeds = [
        SecretSeed::new(
            mutation_fixture("adhoc-ptk-member-min-seed.bin")
                .try_into()
                .unwrap(),
        ),
        SecretSeed::new(
            mutation_fixture("adhoc-ptk-member-seed.bin")
                .try_into()
                .unwrap(),
        ),
    ];
    let new_seeds = [
        SecretSeed::new(
            mutation_fixture("remove-member-ptk-member-min-seed.bin")
                .try_into()
                .unwrap(),
        ),
        SecretSeed::new(
            mutation_fixture("remove-member-ptk-member-seed.bin")
                .try_into()
                .unwrap(),
        ),
    ];
    let roles = [Role::member(-0x4000), Role::member(0)];
    let rotations = new_seeds
        .iter()
        .zip(roles)
        .map(|(seed, role)| TeamPtkRotation {
            role,
            generation: 2,
            seed,
        })
        .collect::<Vec<_>>();
    let removed = &change.changes[0];
    let material = make_remove_local_team_member_link(
        &RemoveLocalTeamMemberInput {
            actor: &change.signer_owner.party,
            actor_source_role: change.signer_owner.source_role,
            team: &change.team,
            host: &change.host,
            sequence: change.seqno,
            previous: prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &addition.encoded().unwrap()),
            root: &change.root,
            time: change.time,
            next_tree_location: mutation_fixture("remove-member-next-tree-location.bin")
                .try_into()
                .unwrap(),
            member: &removed.party,
            member_source_role: removed.source_role,
        },
        &actor_seed,
        &rotations,
    )
    .unwrap();
    assert_eq!(
        material.link.encoded().unwrap(),
        expected.encoded().unwrap()
    );
    assert_eq!(material.link.decode_team_group_change().unwrap(), change);

    let duplicated = [
        TeamPtkRotation {
            role: roles[0],
            generation: 2,
            seed: &new_seeds[0],
        },
        TeamPtkRotation {
            role: roles[1],
            generation: 2,
            seed: &new_seeds[0],
        },
    ];
    assert!(make_remove_local_team_member_link(
        &RemoveLocalTeamMemberInput {
            actor: &change.signer_owner.party,
            actor_source_role: change.signer_owner.source_role,
            team: &change.team,
            host: &change.host,
            sequence: change.seqno,
            previous: change.previous.unwrap(),
            root: &change.root,
            time: change.time,
            next_tree_location: [1; 32],
            member: &removed.party,
            member_source_role: removed.source_role,
        },
        &actor_seed,
        &duplicated,
    )
    .is_err());

    let rotated_boxes =
        SharedKeyBoxSet::decode(&mutation_fixture("remove-member-ptk-boxes.snowp")).unwrap();
    let actor_public = derive_shared_public(&actor_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let receiver =
        SharedKeyDecapsulator::new(&actor_seed, change.signer_owner.party.clone()).unwrap();
    for (index, role) in roles.into_iter().enumerate() {
        let expected_box = foks_proto::SeedChainBox::decode(&mutation_fixture(
            [
                "remove-member-seed-chain-member-min.snowp",
                "remove-member-seed-chain-member.snowp",
            ][index],
        ))
        .unwrap();
        let actual = seal_puk_seed_chain_box(
            &new_seeds[index],
            &old_seeds[index],
            &change.signer_owner.party,
            &change.host,
            1,
            role,
            expected_box.secret_box.nonce,
        )
        .unwrap();
        assert_eq!(actual, expected_box);
        let shared = &rotated_boxes.boxes[index];
        let parcel = PukParcel {
            generation: shared.generation,
            role: shared.role,
            hybrid: shared.hybrid.clone(),
            target: shared.target.entity.clone(),
            target_host: shared.target.host.clone(),
            target_role: shared.target.role,
            target_generation: shared.target.generation,
            sender: actor_public.verify_key.clone(),
            box_id: rotated_boxes.box_id,
            temp_dh_key: rotated_boxes.temp_dh_key.clone(),
            seed_chain: vec![expected_box],
        };
        let clear = open_shared_key_parcel_with(
            &parcel,
            &receiver,
            &actor_public.hepk,
            &material.ptks[index].verify_key,
            &material.ptks[index].hepk,
            shared.generation,
            &change.host,
            Role::OWNER,
            2,
            role,
            ENTITY_PTK_VERIFY,
        )
        .unwrap();
        let history =
            open_shared_key_seed_chain(clear, &parcel, &change.signer_owner.party, &change.host)
                .unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].seed, old_seeds[index]);
        assert_eq!(history[1].seed, new_seeds[index]);
    }

    let removal_key = SecretSeed::new(
        mutation_fixture("add-member-removal-key.bin")
            .try_into()
            .unwrap(),
    );
    let expected_proof = foks_proto::TeamRemovalAndCommitment::decode(&mutation_fixture(
        "remove-member-proof.snowp",
    ))
    .unwrap();
    let actual_proof =
        make_team_removal_proof(&removal_key, expected_proof.removal.payload.clone()).unwrap();
    assert_eq!(actual_proof, expected_proof);
    assert_eq!(
        actual_proof.encoded().unwrap(),
        mutation_fixture("remove-member-proof.snowp")
    );
}

#[test]
fn yubi_parent_signs_adhoc_membership_without_exporting_its_key() {
    use p256::ecdsa::{
        signature::hazmat::PrehashSigner as _, Signature as P256Signature,
        SigningKey as P256SigningKey,
    };

    struct FixtureYubi {
        id: EntityId,
        hepk: Hepk,
        signing: P256SigningKey,
    }

    impl HybridSecretDecapsulator for FixtureYubi {
        fn entity_id(&self) -> &EntityId {
            &self.id
        }

        fn hepk(&self) -> &Hepk {
            &self.hepk
        }

        fn derive_dh_shared(&self, _: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
            Err(Error::YubiSigning)
        }

        fn decapsulate_mlkem768(&self, _: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
            Err(Error::YubiSigning)
        }
    }

    impl YubiDevice for FixtureYubi {
        fn pq_key_id(&self) -> [u8; 32] {
            [0; 32]
        }

        fn sign_sha512_256(&self, digest: &[u8; 32]) -> Result<Vec<u8>> {
            let signature: P256Signature = self
                .signing
                .sign_prehash(digest)
                .map_err(|_| Error::YubiSigning)?;
            Ok(signature.to_der().as_bytes().to_vec())
        }
    }

    let signing = P256SigningKey::from_bytes((&[7_u8; 32]).into()).unwrap();
    let mut id = vec![foks_proto::ENTITY_YUBI];
    id.extend_from_slice(signing.verifying_key().to_encoded_point(true).as_bytes());
    let software_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let yubi = FixtureYubi {
        id: EntityId::from_bytes(id).unwrap(),
        hepk: derive_device_public(&software_seed).unwrap().hepk,
        signing,
    };
    let expected = UserLink::decode(&mutation_fixture("adhoc-team-link.snowp")).unwrap();
    let change = expected.decode_team_group_change().unwrap();
    let owner = &change.changes[0];
    let owner_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let ptk_seeds = [
        "adhoc-ptk-member-min-seed.bin",
        "adhoc-ptk-member-seed.bin",
        "adhoc-ptk-admin-seed.bin",
        "adhoc-ptk-owner-seed.bin",
    ]
    .map(|name| SecretSeed::new(mutation_fixture(name).try_into().unwrap()));
    let material = make_single_owner_adhoc_team_yubi(
        &AdHocTeamInput {
            user: &owner.party,
            host: &change.host,
            root: &change.root,
            time: change.time,
            owner_puk_generation: owner.keys.as_ref().unwrap().generation,
            membership_sequence: 1,
            membership_previous: None,
            next_tree_location: mutation_fixture("adhoc-next-tree-location.bin")
                .try_into()
                .unwrap(),
            subchain_tree_location: mutation_fixture("adhoc-subchain-tree-location.bin")
                .try_into()
                .unwrap(),
            membership_next_tree_location: mutation_fixture(
                "adhoc-membership-next-tree-location.bin",
            )
            .try_into()
            .unwrap(),
        },
        &yubi,
        &owner_seed,
        [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3]],
    )
    .unwrap();
    assert_eq!(
        material.link.encoded().unwrap(),
        expected.encoded().unwrap()
    );
    let [Signature::Ecdsa(_)] = material.membership_link.signatures() else {
        panic!("Yubi membership link must carry exactly one ECDSA signature");
    };
    verify_typed(
        &yubi.id,
        &material.membership_link.signatures()[0],
        LINK_OUTER_V1_TYPE_ID,
        &material.membership_link.signing_bytes(0).unwrap(),
    )
    .unwrap();

    let wrong_signer = FixtureYubi {
        id: yubi.id.clone(),
        hepk: yubi.hepk.clone(),
        signing: P256SigningKey::from_bytes((&[8_u8; 32]).into()).unwrap(),
    };
    assert!(matches!(
        make_single_owner_adhoc_team_yubi(
            &AdHocTeamInput {
                user: &owner.party,
                host: &change.host,
                root: &change.root,
                time: change.time,
                owner_puk_generation: owner.keys.as_ref().unwrap().generation,
                membership_sequence: 1,
                membership_previous: None,
                next_tree_location: [1; 32],
                subchain_tree_location: [2; 32],
                membership_next_tree_location: [3; 32],
            },
            &wrong_signer,
            &owner_seed,
            [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3]],
        ),
        Err(Error::Verification)
    ));
}

#[test]
fn software_mutation_boxes_round_trip_and_preserve_history() {
    let eldest = UserLink::decode(&user_fixture("user-eldest-link.snowp"))
        .unwrap()
        .decode_eldest()
        .unwrap();
    let sender_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let receiver_seed = SecretSeed::new(user_fixture("second-device-seed.bin").try_into().unwrap());
    let receiver = derive_device_public(&receiver_seed).unwrap();
    let current_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let boxes = seal_software_puk_boxes(
        &eldest.host,
        &sender_seed,
        [21; 16],
        &[SoftwarePukBoxInput {
            seed: &current_seed,
            generation: 2,
            role: Role::OWNER,
            receiver: &receiver,
        }],
        &[PukBoxRandomness {
            kem_message: [22; 32],
            nonce: [23; 16],
        }],
    )
    .unwrap();
    assert_eq!(SharedKeyBoxSet::decode(&boxes.encoded()).unwrap(), boxes);
    let shared = boxes.boxes[0].clone();
    let sender = derive_device_public(&sender_seed).unwrap();
    let current_public =
        derive_shared_public(&current_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let previous_seed = SecretSeed::new(user_fixture("initial-puk-seed.bin").try_into().unwrap());
    let historical = seal_puk_seed_chain_box(
        &current_seed,
        &previous_seed,
        &eldest.uid,
        &eldest.host,
        1,
        Role::OWNER,
        [24; 16],
    )
    .unwrap();
    let parcel = PukParcel {
        generation: shared.generation,
        role: shared.role,
        hybrid: shared.hybrid,
        target: shared.target.entity,
        target_host: shared.target.host,
        target_role: shared.target.role,
        target_generation: shared.target.generation,
        sender: sender.id,
        box_id: boxes.box_id,
        temp_dh_key: boxes.temp_dh_key,
        seed_chain: vec![historical],
    };
    let current = open_puk_parcel_for_role(
        &parcel,
        &receiver_seed,
        &sender.hepk,
        &current_public.verify_key,
        &current_public.hepk,
        parcel.generation,
        &eldest.host,
        Role::OWNER,
    )
    .unwrap();
    let history = open_puk_seed_chain(current, &parcel, &eldest.uid, &eldest.host).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].seed, previous_seed);
    assert_eq!(history[1].seed, current_seed);
}

#[test]
fn official_initial_signup_puk_box_opens() {
    let expected = UserLink::decode(&signup_fixture("eldest-link.snowp")).unwrap();
    let opened = expected.decode_eldest().unwrap();
    let device_seed = SecretSeed::new(signup_fixture("device-seed.bin").try_into().unwrap());
    let puk_bytes: [u8; 32] = signup_fixture("puk-seed.bin").try_into().unwrap();
    let puk_seed = SecretSeed::new(puk_bytes);
    let device = derive_device_public(&device_seed).unwrap();
    let puk = derive_shared_public(&puk_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let boxed = SharedKeyBoxSet::decode(&signup_fixture("puk-box-set.snowp")).unwrap();
    let box_id = boxed.box_id;
    let temporary = boxed.temp_dh_key;
    let shared = boxed.boxes.into_iter().next().unwrap();
    let parcel = PukParcel {
        generation: shared.generation,
        role: shared.role,
        hybrid: shared.hybrid,
        target: shared.target.entity,
        target_host: shared.target.host,
        target_role: shared.target.role,
        target_generation: shared.target.generation,
        sender: device.id,
        box_id,
        temp_dh_key: temporary,
        seed_chain: Vec::new(),
    };
    let clear = open_puk_parcel_for_role(
        &parcel,
        &device_seed,
        &device.hepk,
        &puk.verify_key,
        &puk.hepk,
        parcel.generation,
        &opened.host,
        Role::OWNER,
    )
    .unwrap();
    assert_eq!(clear.seed.as_bytes(), &puk_bytes);
}

#[test]
fn sha512_256_type_prefix_is_big_endian() {
    let object = [0x91, 0xc0];
    let mut direct = Sha512_256::new();
    direct.update(0x0102_0304_0506_0708_u64.to_be_bytes());
    direct.update(object);
    assert_eq!(
        prefixed_hash(0x0102_0304_0506_0708, &object),
        <[u8; 32]>::from(direct.finalize())
    );
}

#[test]
fn initial_software_puk_box_round_trips_and_is_bound() {
    let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
    let host = chain.links[0].decode_eldest().unwrap().host;
    let device_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let puk_bytes: [u8; 32] = user_fixture("initial-puk-seed.bin").try_into().unwrap();
    let puk_seed = SecretSeed::new(puk_bytes);
    let boxed = seal_initial_puk_box(
        &host,
        &device_seed,
        &puk_seed,
        InitialPukBoxRandomness {
            box_id: [7; 16],
            kem_message: [8; 32],
            nonce: [9; 16],
        },
    )
    .unwrap();
    assert_eq!(SharedKeyBoxSet::decode(&boxed.encoded()).unwrap(), boxed);
    let device = derive_device_public(&device_seed).unwrap();
    let puk = derive_shared_public(&puk_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
    let shared = boxed.boxes.first().unwrap();
    let parcel = PukParcel {
        generation: shared.generation,
        role: shared.role,
        hybrid: shared.hybrid.clone(),
        target: shared.target.entity.clone(),
        target_host: shared.target.host.clone(),
        target_role: shared.target.role,
        target_generation: shared.target.generation,
        sender: device.id.clone(),
        box_id: boxed.box_id,
        temp_dh_key: boxed.temp_dh_key.clone(),
        seed_chain: Vec::new(),
    };
    let opened = open_puk_parcel_for_role(
        &parcel,
        &device_seed,
        &device.hepk,
        &puk.verify_key,
        &puk.hepk,
        parcel.generation,
        &host,
        Role::OWNER,
    )
    .unwrap();
    assert_eq!(opened.seed.as_bytes(), &puk_bytes);

    let wrong_puk = derive_shared_public(&SecretSeed::new([0x55; 32]), ENTITY_PUK_VERIFY).unwrap();
    assert!(matches!(
        open_puk_parcel_for_role(
            &parcel,
            &device_seed,
            &device.hepk,
            &puk.verify_key,
            &wrong_puk.hepk,
            parcel.generation,
            &host,
            Role::OWNER,
        ),
        Err(Error::PukBinding)
    ));
    assert!(open_puk_parcel_for_role(
        &parcel,
        &device_seed,
        &device.hepk,
        &puk.verify_key,
        &puk.hepk,
        parcel.generation + 1,
        &host,
        Role::OWNER,
    )
    .is_err());

    let mut tampered = parcel;
    tampered.hybrid.ciphertext[0] ^= 1;
    assert!(open_puk_parcel_for_role(
        &tampered,
        &device_seed,
        &device.hepk,
        &puk.verify_key,
        &puk.hepk,
        tampered.generation,
        &host,
        Role::OWNER,
    )
    .is_err());
}

#[test]
fn official_public_zone_signature_verifies() {
    let probe = ProbeResponse::decode(PROBE).unwrap();
    let change = probe.hostchain[0].decode_change().unwrap();
    let signer = change
        .changes
        .iter()
        .find_map(|change| match change {
            foks_proto::HostchainChangeItem::Key(id)
                if id.entity_type() == foks_proto::ENTITY_HOST_METADATA_SIGNER =>
            {
                Some(id)
            }
            _ => None,
        })
        .unwrap();
    verify_blob(
        signer,
        &probe.public_zone.signature,
        PUBLIC_ZONE_BLOB_TYPE_ID,
        &probe.public_zone.inner,
    )
    .unwrap();
}

#[test]
fn signed_blob_tampering_is_rejected() {
    let probe = ProbeResponse::decode(PROBE).unwrap();
    let change = probe.hostchain[0].decode_change().unwrap();
    let signer = change
        .changes
        .iter()
        .find_map(|change| match change {
            foks_proto::HostchainChangeItem::Key(id)
                if id.entity_type() == foks_proto::ENTITY_HOST_METADATA_SIGNER =>
            {
                Some(id)
            }
            _ => None,
        })
        .unwrap();
    let mut inner = probe.public_zone.inner.clone();
    inner[5] ^= 1;
    assert!(verify_blob(
        signer,
        &probe.public_zone.signature,
        PUBLIC_ZONE_BLOB_TYPE_ID,
        &inner,
    )
    .is_err());
}

#[test]
fn official_device_derivation_and_puk_unboxing_match() {
    let seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let derived = derive_device_public(&seed).unwrap();
    let expected_id = match foks_snowpack::decode(&user_fixture("device-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        _ => panic!("device fixture is not binary"),
    };
    assert_eq!(derived.id, expected_id);

    let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
    assert!(chain.hepks.iter().any(|hepk| hepk == &derived.hepk));
    let eldest = chain.links[0].decode_eldest().unwrap();
    let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0].clone();
    let rotated_public = derive_shared_public(
        &SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap()),
        ENTITY_PUK_VERIFY,
    )
    .unwrap();
    assert_eq!(rotated_public.verify_key, rotated.verify_key);
    let mut parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
    let (hybrid_key, payload) =
        derive_hybrid_key(&parcel, &seed, derived.hepk.classical(), &derived.hepk).unwrap();
    assert_eq!(payload.as_slice(), user_fixture("hybrid-payload.snowp"));
    assert_eq!(
        hybrid_key.as_slice(),
        user_fixture("hybrid-secretbox-key.bin")
    );
    let clear = open_puk_parcel(
        &parcel,
        &seed,
        &derived.hepk,
        &rotated.verify_key,
        &rotated_public.hepk,
        rotated.generation,
        &eldest.host,
    )
    .unwrap();
    assert_eq!(clear.seed.as_slice(), user_fixture("puk-seed.bin"));
    let puks = open_puk_seed_chain(clear, &parcel, &eldest.uid, &eldest.host).unwrap();
    assert_eq!(puks.len(), 2);
    assert_eq!(puks[0].generation, 1);
    assert_eq!(
        puks[0].seed.as_slice(),
        user_fixture("initial-puk-seed.bin")
    );
    assert_eq!(puks[1].generation, 2);
    assert_eq!(puks[1].seed.as_slice(), user_fixture("puk-seed.bin"));
    let mut wrong_generation = parcel.clone();
    wrong_generation.seed_chain[0].generation = 2;
    let clear = open_puk_parcel(
        &wrong_generation,
        &seed,
        &derived.hepk,
        &rotated.verify_key,
        &rotated_public.hepk,
        rotated.generation,
        &eldest.host,
    )
    .unwrap();
    assert!(open_puk_seed_chain(clear, &wrong_generation, &eldest.uid, &eldest.host).is_err());
    parcel.seed_chain[0].secret_box.ciphertext[0] ^= 1;
    let clear = open_puk_parcel(
        &parcel,
        &seed,
        &derived.hepk,
        &rotated.verify_key,
        &rotated_public.hepk,
        rotated.generation,
        &eldest.host,
    )
    .unwrap();
    assert!(open_puk_seed_chain(clear, &parcel, &eldest.uid, &eldest.host).is_err());
}

#[test]
fn official_team_ptk_parcels_unbox_for_every_role() {
    let puk_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let uid = match foks_snowpack::decode(&user_fixture("uid.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        _ => panic!("UID fixture is not binary"),
    };
    let receiver = SharedKeyDecapsulator::new(&puk_seed, uid).unwrap();
    let chain = TeamChain::decode(&user_fixture("team-chain.snowp")).unwrap();
    let change = chain.links[0].decode_team_group_change().unwrap();
    let member = &change.changes[0];
    let member_keys = member.keys.as_ref().unwrap();
    let expected = [
        (Role::member(-0x4000), "team-ptk-member-min-seed.bin"),
        (Role::member(0), "team-ptk-member-seed.bin"),
        (Role::ADMIN, "team-ptk-admin-seed.bin"),
        (Role::OWNER, "team-ptk-owner-seed.bin"),
    ];
    assert_eq!(chain.boxes.len(), expected.len());
    for (role, seed_file) in expected {
        let key = change
            .shared_keys
            .iter()
            .find(|key| key.role == role)
            .expect("fixture has a PTK for every eldest role");
        let parcel = chain
            .boxes
            .iter()
            .find(|parcel| parcel.role == role)
            .expect("fixture has a parcel for every eldest role");
        assert_eq!(parcel.target_role, member.source_role);
        assert_eq!(parcel.target_generation, member_keys.generation);
        assert!(parcel.target_host.is_none());
        let expected_public = derive_shared_public(
            &SecretSeed::new(user_fixture(seed_file).try_into().unwrap()),
            ENTITY_PTK_VERIFY,
        )
        .unwrap();
        assert_eq!(expected_public.verify_key, key.verify_key);
        let clear = open_shared_key_parcel_with(
            parcel,
            &receiver,
            receiver.hepk(),
            &key.verify_key,
            &expected_public.hepk,
            key.generation,
            &change.host,
            member.source_role,
            member_keys.generation,
            role,
            ENTITY_PTK_VERIFY,
        )
        .unwrap();
        assert_eq!(clear.seed.as_slice(), user_fixture(seed_file));
    }
}

#[test]
fn official_team_kv_tree_verifies_and_decrypts_end_to_end() {
    let seed = SecretSeed::new(
        user_fixture("team-ptk-member-min-seed.bin")
            .try_into()
            .unwrap(),
    );
    let team = match foks_snowpack::decode(&user_fixture("team-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        _ => panic!("team ID fixture is not binary"),
    };
    let chain = TeamChain::decode(&user_fixture("team-chain.snowp")).unwrap();
    let host = chain.links[0].decode_team_group_change().unwrap().host;
    let party = KvParty { party: team, host };
    let root = KvRoot::decode(&user_fixture("kv-root.snowp")).unwrap();
    let keys = derive_kv_keys(&seed).unwrap();
    keys.verify_root(&root, &party).unwrap();

    let directory =
        foks_proto::KvDirectoryPair::decode(&user_fixture("kv-root-dir.snowp")).unwrap();
    assert_eq!(directory.active.id, root.root);
    let directory_seed = keys.open_directory_seed(&directory.active).unwrap();
    assert_eq!(
        directory_seed.as_slice(),
        user_fixture("kv-root-dir-seed.bin")
    );

    let listing = foks_proto::KvListResponse::decode(&user_fixture("kv-list.snowp")).unwrap();
    assert!(listing.final_page);
    assert_eq!(listing.entries.len(), 3);
    let names = listing
        .entries
        .iter()
        .map(|entry| open_kv_dirent_name(&directory_seed, entry).unwrap().name)
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            b"small.txt".to_vec(),
            b"latest".to_vec(),
            b"large.bin".to_vec()
        ]
    );

    let small = match foks_proto::KvNode::decode(&user_fixture("kv-small-node.snowp")).unwrap() {
        foks_proto::KvNode::SmallFile(boxed) => boxed,
        _ => panic!("small-file fixture has wrong node type"),
    };
    let small_id = listing.entries[0].value;
    assert_eq!(
        keys.open_small_file(small_id, &small).unwrap(),
        KvSmallFilePlaintext::File(user_fixture("kv-small-plaintext.bin"))
    );

    let symlink = match foks_proto::KvNode::decode(&user_fixture("kv-symlink-node.snowp")).unwrap()
    {
        foks_proto::KvNode::Symlink(boxed) => boxed,
        _ => panic!("symlink fixture has wrong node type"),
    };
    assert_eq!(
        keys.open_small_file(listing.entries[1].value, &symlink)
            .unwrap(),
        KvSmallFilePlaintext::Symlink(user_fixture("kv-symlink-plaintext.bin"))
    );

    let metadata = match foks_proto::KvNode::decode(&user_fixture("kv-large-node.snowp")).unwrap() {
        foks_proto::KvNode::File(metadata) => metadata,
        _ => panic!("large-file fixture has wrong node type"),
    };
    let large_id = listing.entries[2].value;
    let file_seed = keys.open_file_seed(large_id, &metadata).unwrap();
    assert_eq!(file_seed.as_slice(), user_fixture("kv-file-seed.bin"));
    // The official Go v0.1.9 fixture predates the Rust size extension.
    assert_eq!(
        keys.open_large_file_size(large_id, &metadata).unwrap(),
        None
    );
    let chunk = KvEncryptedChunk::decode(&user_fixture("kv-large-chunk.snowp")).unwrap();
    assert_eq!(
        open_kv_chunk(&file_seed, large_id, 0, &chunk).unwrap(),
        user_fixture("kv-large-plaintext.bin")
    );
}

#[test]
fn kv_write_sealing_matches_official_v019_objects() {
    let shared = SecretSeed::new(
        user_fixture("team-ptk-member-min-seed.bin")
            .try_into()
            .unwrap(),
    );
    let keys = derive_kv_keys(&shared).unwrap();
    let key = RoleAndGeneration {
        role: Role::member(-16_384),
        generation: 1,
    };
    let listing = foks_proto::KvListResponse::decode(&user_fixture("kv-list.snowp")).unwrap();
    let small_id = listing.entries[0].value;
    let small = keys
        .seal_small_file(
            small_id,
            key,
            KvSmallFilePlaintext::File(user_fixture("kv-small-plaintext.bin")),
        )
        .unwrap();
    assert_eq!(small.encode().unwrap(), user_fixture("kv-small-box.snowp"));

    let file_seed = SecretSeed::new(user_fixture("kv-file-seed.bin").try_into().unwrap());
    let large_id = listing.entries[2].value;
    let expected_metadata =
        KvLargeFileMetadata::decode(&user_fixture("kv-write-large-metadata.snowp")).unwrap();
    let metadata = keys
        .seal_file_seed(
            large_id,
            key,
            1,
            &file_seed,
            expected_metadata.key_seed.nonce,
        )
        .unwrap();
    assert_eq!(metadata, expected_metadata);

    let clear = user_fixture("kv-large-plaintext.bin");
    let chunk = seal_kv_chunk(&file_seed, large_id, 0, true, &clear, 0).unwrap();
    assert_eq!(
        chunk.encode().unwrap(),
        user_fixture("kv-upload-chunk.snowp")
    );

    let directory_seed = SecretSeed::new(user_fixture("kv-root-dir-seed.bin").try_into().unwrap());
    let expected = KvDirent::decode(&user_fixture("kv-write-dirent.snowp")).unwrap();
    let (name_mac, name_box) = seal_kv_dirent_name(
        &directory_seed,
        expected.parent,
        expected.directory_version,
        b"write.txt".to_vec(),
        expected.name_box.nonce,
    )
    .unwrap();
    assert_eq!(name_mac, expected.name_mac);
    assert_eq!(name_box, expected.name_box);
    assert_eq!(
        bind_kv_dirent(&directory_seed, &expected).unwrap(),
        expected.binding_mac
    );
}

#[test]
fn large_file_size_metadata_is_versioned_bound_and_authenticated() {
    let seed = SecretSeed::new([7; 32]);
    let keys = derive_kv_keys(&seed).unwrap();
    let mut id_bytes = [0; 17];
    id_bytes[0] = 2;
    id_bytes[1..].copy_from_slice(&[9; 16]);
    let id = KvNodeId(id_bytes);
    let key = RoleAndGeneration {
        role: Role::OWNER,
        generation: 1,
    };
    let mut metadata = keys
        .seal_file_seed(id, key, 4, &SecretSeed::new([3; 32]), [1; 16])
        .unwrap();
    metadata.custom_metadata = Some(
        keys.seal_large_file_size(id, metadata.version, 0, [2; 16])
            .unwrap(),
    );
    assert_eq!(keys.open_large_file_size(id, &metadata).unwrap(), Some(0));

    let mut other_id = id;
    other_id.0[16] ^= 1;
    assert!(matches!(
        keys.open_large_file_size(other_id, &metadata),
        Err(Error::KvBinding)
    ));
    let mut other_version = metadata.clone();
    other_version.version += 1;
    assert!(matches!(
        keys.open_large_file_size(id, &other_version),
        Err(Error::KvBinding)
    ));

    let mut tampered = metadata.clone();
    tampered.custom_metadata.as_mut().unwrap().ciphertext[0] ^= 1;
    assert!(matches!(
        keys.open_large_file_size(id, &tampered),
        Err(Error::Decryption)
    ));

    let nonce = [4; 16];
    let unsupported = encode_ref(&ValueRef::Array(vec![ValueRef::Unsigned(2)])).unwrap();
    metadata.custom_metadata = Some(SecretBox {
        nonce,
        ciphertext: seal_typed_secretbox(
            &keys.box_key,
            LARGE_FILE_SIZE_PAYLOAD_TYPE_ID,
            &nonce,
            &unsupported,
            false,
        )
        .unwrap(),
    });
    assert_eq!(keys.open_large_file_size(id, &metadata).unwrap(), None);
}

#[test]
fn kv_bindings_and_ciphertexts_fail_closed() {
    let seed = SecretSeed::new(
        user_fixture("team-ptk-member-min-seed.bin")
            .try_into()
            .unwrap(),
    );
    let keys = derive_kv_keys(&seed).unwrap();
    let mut root = KvRoot::decode(&user_fixture("kv-root.snowp")).unwrap();
    let team = match foks_snowpack::decode(&user_fixture("team-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        _ => unreachable!(),
    };
    let chain = TeamChain::decode(&user_fixture("team-chain.snowp")).unwrap();
    let party = KvParty {
        party: team,
        host: chain.links[0].decode_team_group_change().unwrap().host,
    };
    root.binding_mac[0] ^= 1;
    assert!(matches!(
        keys.verify_root(&root, &party),
        Err(Error::KvBinding)
    ));

    let mut small = KvSmallFileBox::decode(&user_fixture("kv-small-box.snowp")).unwrap();
    small.ciphertext[0] ^= 1;
    let listing = foks_proto::KvListResponse::decode(&user_fixture("kv-list.snowp")).unwrap();
    assert!(matches!(
        keys.open_small_file(listing.entries[0].value, &small),
        Err(Error::Decryption)
    ));
}

#[test]
fn large_chunk_requires_the_exact_requested_offset() {
    let file_seed = SecretSeed::new([0x55; 32]);
    let mut id = [0x77; 17];
    id[0] = 2;
    let id = KvNodeId(id);
    let chunk_offset = 0u64;
    let clear = b"abcdef".to_vec();
    let chunk = seal_kv_chunk(&file_seed, id, chunk_offset, true, &clear, 0).unwrap();
    let chunk = KvEncryptedChunk {
        ciphertext: chunk.ciphertext,
        offset: chunk.offset,
        final_chunk: chunk.final_upload.is_some(),
    };
    assert_eq!(
        open_kv_chunk(&file_seed, id, chunk_offset, &chunk).unwrap(),
        b"abcdef"
    );
    let requested = 2u64;
    assert!(matches!(
        open_kv_chunk(&file_seed, id, requested, &chunk),
        Err(Error::KvBinding)
    ));
    let ahead = KvEncryptedChunk {
        offset: requested + 1,
        ..chunk.clone()
    };
    assert!(matches!(
        open_kv_chunk(&file_seed, id, requested, &ahead),
        Err(Error::KvBinding)
    ));
    let lying = KvEncryptedChunk {
        offset: requested,
        ..chunk
    };
    assert!(matches!(
        open_kv_chunk(&file_seed, id, requested, &lying),
        Err(Error::Decryption)
    ));
}

#[test]
fn hybrid_puk_tampering_is_rejected() {
    let seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let derived = derive_device_public(&seed).unwrap();
    let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
    let eldest = chain.links[0].decode_eldest().unwrap();
    let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0].clone();
    let rotated_public = derive_shared_public(
        &SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap()),
        ENTITY_PUK_VERIFY,
    )
    .unwrap();
    let mut parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
    parcel.hybrid.ciphertext[0] ^= 1;
    assert!(matches!(
        open_puk_parcel(
            &parcel,
            &seed,
            &derived.hepk,
            &rotated.verify_key,
            &rotated_public.hepk,
            rotated.generation,
            &eldest.host,
        ),
        Err(Error::Decryption)
    ));
}

#[test]
fn official_mock_yubi_eldest_signature_stack_verifies() {
    let link = UserLink::decode(&user_fixture("yubi/yubi-eldest-link.snowp")).unwrap();
    let eldest = link.decode_eldest().unwrap();
    let subkey = eldest.member_subkey.as_ref().unwrap();
    assert_eq!(link.signatures().len(), 3);
    verify_typed(
        &eldest.puk_verify_key,
        &link.signatures()[0],
        foks_proto::LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(0).unwrap(),
    )
    .unwrap();
    verify_typed(
        subkey,
        &link.signatures()[1],
        foks_proto::LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(1).unwrap(),
    )
    .unwrap();
    verify_typed(
        &eldest.member,
        &link.signatures()[2],
        foks_proto::LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(2).unwrap(),
    )
    .unwrap();

    let mut tampered = link.signatures()[2].clone();
    let Signature::Ecdsa(bytes) = &mut tampered else {
        panic!("mock Yubi fixture did not use ECDSA");
    };
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert!(verify_typed(
        &eldest.member,
        &tampered,
        foks_proto::LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(2).unwrap(),
    )
    .is_err());
}

#[test]
fn official_yubi_to_software_cross_curve_parcel_unboxes() {
    let seed = SecretSeed::new(
        user_fixture("yubi/software-device-seed.bin")
            .try_into()
            .unwrap(),
    );
    let sender_hepk = Hepk::decode(&user_fixture("yubi/yubi-hepk.snowp")).unwrap();
    let sender = match foks_snowpack::decode(&user_fixture("yubi/yubi-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        _ => panic!("Yubi fixture is not an EntityID"),
    };
    let parcel =
        PukParcel::decode(&user_fixture("yubi/yubi-to-software-puk-parcel.snowp")).unwrap();
    assert_eq!(parcel.sender, sender);
    assert!(parcel.temp_dh_key.is_some());
    let link = UserLink::decode(&user_fixture("yubi/yubi-eldest-link.snowp")).unwrap();
    let eldest = link.decode_eldest().unwrap();
    let expected_puk = derive_shared_public(
        &SecretSeed::new(user_fixture("yubi/puk-seed.bin").try_into().unwrap()),
        ENTITY_PUK_VERIFY,
    )
    .unwrap();
    let clear = open_puk_parcel(
        &parcel,
        &seed,
        &sender_hepk,
        &eldest.puk_verify_key,
        &expected_puk.hepk,
        parcel.generation,
        &eldest.host,
    )
    .unwrap();
    assert_eq!(clear.seed.as_slice(), user_fixture("yubi/puk-seed.bin"));

    let mut tampered = parcel;
    let Signature::Ecdsa(signature) = &mut tampered
        .temp_dh_key
        .as_mut()
        .expect("cross-curve parcel has a temporary key")
        .signature
    else {
        panic!("Yubi fixture has an unexpected signature type");
    };
    signature[0] ^= 1;
    assert!(open_puk_parcel(
        &tampered,
        &seed,
        &sender_hepk,
        &eldest.puk_verify_key,
        &expected_puk.hepk,
        tampered.generation,
        &eldest.host,
    )
    .is_err());
}

#[test]
fn official_go_mixed_curve_box_sets_open_with_the_matching_sender_path() {
    let software_seed = SecretSeed::new(
        user_fixture("yubi/software-device-seed.bin")
            .try_into()
            .unwrap(),
    );
    let software = derive_device_public(&software_seed).unwrap();
    let link = UserLink::decode(&user_fixture("yubi/yubi-eldest-link.snowp")).unwrap();
    let eldest = link.decode_eldest().unwrap();
    let puk_seed = SecretSeed::new(user_fixture("yubi/puk-seed.bin").try_into().unwrap());
    let puk = derive_shared_public(&puk_seed, ENTITY_PUK_VERIFY).unwrap();

    let software_mixed =
        SharedKeyBoxSet::decode(&user_fixture("yubi/software-mixed-puk-box-set.snowp")).unwrap();
    assert_eq!(software_mixed.boxes.len(), 2);
    assert!(software_mixed.temp_dh_key.is_some());
    let mut same_curve =
        PukParcel::from_box_set(&software_mixed, 0, software.id.clone(), Vec::new()).unwrap();
    // Go's OpenBoxInSet deliberately ignores the set-level temporary key
    // when sender and receiver use the same classical curve.
    let Signature::Ed25519(signature) = &mut same_curve
        .temp_dh_key
        .as_mut()
        .expect("mixed set has a temporary key")
        .signature
    else {
        panic!("software sender used the wrong temporary-key signature")
    };
    signature[0] ^= 1;
    assert_eq!(
        open_puk_parcel(
            &same_curve,
            &software_seed,
            &software.hepk,
            &puk.verify_key,
            &puk.hepk,
            1,
            &eldest.host,
        )
        .unwrap()
        .seed,
        puk_seed
    );

    let yubi_mixed =
        SharedKeyBoxSet::decode(&user_fixture("yubi/yubi-mixed-puk-box-set.snowp")).unwrap();
    assert_eq!(yubi_mixed.boxes.len(), 2);
    let yubi_sender_hepk = Hepk::decode(&user_fixture("yubi/yubi-hepk.snowp")).unwrap();
    let yubi_sender = match foks_snowpack::decode(&user_fixture("yubi/yubi-id.snowp")).unwrap() {
        Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
        _ => panic!("Yubi fixture is not an EntityID"),
    };
    let cross_curve = PukParcel::from_box_set(&yubi_mixed, 1, yubi_sender, Vec::new()).unwrap();
    assert_eq!(
        open_puk_parcel(
            &cross_curve,
            &software_seed,
            &yubi_sender_hepk,
            &puk.verify_key,
            &puk.hepk,
            1,
            &eldest.host,
        )
        .unwrap()
        .seed,
        puk_seed
    );
}

#[test]
fn hardware_boundary_reproduces_the_official_hybrid_secrets() {
    struct FixtureHardware {
        public: DevicePublicMaterial,
    }

    impl HybridSecretDecapsulator for FixtureHardware {
        fn entity_id(&self) -> &EntityId {
            &self.public.id
        }
        fn hepk(&self) -> &Hepk {
            &self.public.hepk
        }
        fn derive_dh_shared(&self, _: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
            Ok(Zeroizing::new(
                user_fixture("hybrid-dh-shared.bin").try_into().unwrap(),
            ))
        }
        fn decapsulate_mlkem768(&self, _: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
            Ok(Zeroizing::new(
                user_fixture("hybrid-kem-shared.bin").try_into().unwrap(),
            ))
        }
    }

    let seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let hardware = FixtureHardware {
        public: derive_device_public(&seed).unwrap(),
    };
    let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
    let eldest = chain.links[0].decode_eldest().unwrap();
    let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0].clone();
    let rotated_public = derive_shared_public(
        &SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap()),
        ENTITY_PUK_VERIFY,
    )
    .unwrap();
    let parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
    let clear = open_puk_parcel_with(
        &parcel,
        &hardware,
        &hardware.public.hepk,
        &rotated.verify_key,
        &rotated_public.hepk,
        rotated.generation,
        &eldest.host,
    )
    .unwrap();
    assert_eq!(clear.seed.as_slice(), user_fixture("puk-seed.bin"));
}

#[test]
fn backup_enrollment_and_recovery_links_match_go_v019() {
    let backup =
        BackupKey::from_seed(mutation_fixture("backup-seed.bin").try_into().unwrap()).unwrap();
    let existing_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());

    let expected_enroll = UserLink::decode(&mutation_fixture("backup-enroll-link.snowp")).unwrap();
    let enroll = expected_enroll.decode_group_change().unwrap();
    let name = backup.device_name();
    let enroll_material = make_backup_provision_link(
        &SoftwareProvisionInput {
            base: UserMutationBase {
                uid: &enroll.uid,
                host: &enroll.host,
                seqno: enroll.seqno,
                previous: enroll.previous.unwrap(),
                root: &enroll.root,
                time: enroll.time,
                next_tree_location: mutation_fixture("backup-enroll-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
            },
            role: Role::OWNER,
            device_label: &foks_proto::DeviceLabel {
                device_type: foks_proto::DeviceType::Backup,
                normalized_name: name.as_bytes().to_vec(),
                serial: 1,
            },
            device_name_commitment_key: mutation_fixture(
                "backup-enroll-device-name-commitment-key.bin",
            )
            .try_into()
            .unwrap(),
        },
        &existing_seed,
        &backup,
        None,
    )
    .unwrap();
    assert_eq!(
        enroll_material.link.encoded().unwrap(),
        expected_enroll.encoded().unwrap()
    );

    let expected_recover =
        UserLink::decode(&mutation_fixture("backup-recover-link.snowp")).unwrap();
    let recover = expected_recover.decode_group_change().unwrap();
    let replacement_seed = SecretSeed::new(
        mutation_fixture("backup-recover-device-seed.bin")
            .try_into()
            .unwrap(),
    );
    let recover_material = make_software_provision_link_from_backup(
        &SoftwareProvisionInput {
            base: UserMutationBase {
                uid: &recover.uid,
                host: &recover.host,
                seqno: recover.seqno,
                previous: recover.previous.unwrap(),
                root: &recover.root,
                time: recover.time,
                next_tree_location: mutation_fixture("backup-recover-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
            },
            role: Role::OWNER,
            device_label: &foks_proto::DeviceLabel {
                device_type: foks_proto::DeviceType::Computer,
                normalized_name: b"recovered fixture device".to_vec(),
                serial: 1,
            },
            device_name_commitment_key: mutation_fixture(
                "backup-recover-device-name-commitment-key.bin",
            )
            .try_into()
            .unwrap(),
        },
        &backup,
        &replacement_seed,
        None,
    )
    .unwrap();
    assert_eq!(
        recover_material.link.encoded().unwrap(),
        expected_recover.encoded().unwrap()
    );
}

#[test]
fn backup_opens_the_official_enrollment_puk_box() {
    let backup =
        BackupKey::from_seed(mutation_fixture("backup-seed.bin").try_into().unwrap()).unwrap();
    let boxes = SharedKeyBoxSet::decode(&mutation_fixture("backup-enroll-boxes.snowp")).unwrap();
    let boxed = boxes.boxes[0].clone();
    let sender_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
    let sender = derive_device_public(&sender_seed).unwrap();
    let parcel = PukParcel {
        generation: boxed.generation,
        role: boxed.role,
        hybrid: boxed.hybrid,
        target: boxed.target.entity,
        target_host: boxed.target.host,
        target_role: boxed.target.role,
        target_generation: boxed.target.generation,
        sender: sender.id,
        box_id: boxes.box_id,
        temp_dh_key: boxes.temp_dh_key,
        seed_chain: Vec::new(),
    };
    let expected_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
    let expected = derive_shared_public(&expected_seed, ENTITY_PUK_VERIFY).unwrap();
    let receiver = backup.key_material().unwrap();
    let opened = open_puk_parcel_with_for_role(
        &parcel,
        &receiver,
        &sender.hepk,
        &expected.verify_key,
        &expected.hepk,
        parcel.generation,
        &expected_enroll_host(),
        Role::OWNER,
    )
    .unwrap();
    assert_eq!(opened.seed, expected_seed);
}

#[test]
fn remote_member_view_token_box_round_trips_and_binds_ciphertext() {
    let seed = SecretSeed::new([0xa1; 32]);
    let party = foks_proto::FqParty::new(
        EntityId::from_bytes([vec![foks_proto::ENTITY_NAMED_TEAM], vec![0xa2; 32]].concat())
            .unwrap(),
        EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0xa3; 32]].concat()).unwrap(),
    )
    .unwrap();
    let mut token = [0xa4; 17];
    token[0] = 54;
    let payload = foks_proto::TeamRemoteMemberViewTokenBoxPayload {
        token: foks_proto::PermissionToken::new(token),
        party,
        time: 42,
    };
    let mut boxed = seal_team_remote_member_view_token(&seed, &payload, [0xa5; 16]).unwrap();
    assert_eq!(
        open_team_remote_member_view_token(&seed, &boxed).unwrap(),
        payload
    );
    boxed.ciphertext[0] ^= 1;
    assert!(open_team_remote_member_view_token(&seed, &boxed).is_err());
}

fn expected_enroll_host() -> EntityId {
    UserLink::decode(&mutation_fixture("backup-enroll-link.snowp"))
        .unwrap()
        .decode_group_change()
        .unwrap()
        .host
}
