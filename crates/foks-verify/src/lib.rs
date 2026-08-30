//! Native verification of implemented FOKS v0.1.9 trust and policy state.
//!
//! Untrusted host, Merkle, user, and team evidence becomes usable only after
//! its signatures, transparency bindings, and state transitions verify. The
//! same replay paths restore sealed capabilities from untrusted hard state.

#![forbid(unsafe_code)]

mod error;
mod host;
mod merkle;
mod proof;
mod team;
mod user;
mod user_transition;

use std::collections::{BTreeMap, HashSet};

use foks_crypto::{commitment, verify_blob, verify_typed};
use foks_proto::{
    ChangeMetadata, EntityId, Hepk, HistoricalMerkleRoots, HostchainLink, HostchainTail,
    MerklePathCompressed, MerkleRoot, MerkleTerminal, ProbeResponse, PublicZone, Role, RoleType,
    ServiceType, SignedBlob, TeamChain, TreeRoot, UserChain, UserEldest, DEVICE_LABEL_TYPE_ID,
    ENTITY_AD_HOC_TEAM, ENTITY_HOST, ENTITY_HOST_MERKLE_SIGNER, ENTITY_HOST_METADATA_SIGNER,
    ENTITY_ID_MERKLE_VALUE_TYPE_ID, ENTITY_NAMED_TEAM, ENTITY_PTK_VERIFY, ENTITY_USER,
    HEPK_TYPE_ID, HOSTCHAIN_LINK_OUTER_TYPE_ID, HOSTCHAIN_LINK_OUTER_V1_TYPE_ID,
    LINK_OUTER_TYPE_ID, LINK_OUTER_V1_TYPE_ID, MERKLE_BACK_POINTERS_TYPE_ID, MERKLE_NODE_TYPE_ID,
    MERKLE_ROOT_BLOB_TYPE_ID, MERKLE_ROOT_TYPE_ID, MERKLE_TREE_RF_INPUT_TYPE_ID,
    NAME_COMMITMENT_TYPE_ID, NAME_HASH_PREIMAGE_TYPE_ID, PUBLIC_ZONE_BLOB_TYPE_ID,
    TREE_LOCATION_TYPE_ID,
};
use foks_snowpack::{encode, Value};
use thiserror::Error;
use user_transition::UserReplayState;
use x509_parser::prelude::parse_x509_certificate;

const ED25519_OID: &str = "1.3.101.112";

pub use error::*;
pub use host::*;
pub use merkle::*;
pub use proof::{verify_merkle_path, verify_merkle_path_present};
pub use team::*;
pub use user::*;
pub use user_transition::{verify_user_transition, VerifiedUserTransition};

fn prefixed_hash(type_id: u64, canonical_object: &[u8]) -> Result<[u8; 32]> {
    Ok(foks_crypto::prefixed_hash_signable(
        type_id,
        canonical_object,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use quickcheck::QuickCheck;
    use std::collections::BTreeSet;

    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );
    const HOST_ID: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/host-id.snowp");
    const USER_CHAIN: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/user-chain.snowp");
    const USER_ROOT: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-root-998.snowp");
    const USER_HISTORY: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-historical-response.snowp"
    );
    const TEAM_CHAIN: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/team-chain.snowp");
    const TEAM_ROOT: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/team-merkle-root-996.snowp"
    );
    const TEAM_HISTORY: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/team-merkle-historical-response.snowp"
    );

    fn binary_entity(bytes: &[u8]) -> EntityId {
        let Value::Binary(bytes) = foks_snowpack::decode(bytes).unwrap() else {
            panic!("fixture is not binary");
        };
        EntityId::from_bytes(bytes).unwrap()
    }

    fn trusted_tail(public: &VerifiedPublicHost) -> HostchainTail {
        HostchainTail {
            seqno: public.snapshot.chain_seqno,
            hash: public.snapshot.chain_tail_hash,
        }
    }

    fn incremental_noop_response(
        full_response: &[u8],
        name_path_count: usize,
        team: bool,
    ) -> Vec<u8> {
        let Value::Array(mut fields) = foks_snowpack::decode(full_response).unwrap() else {
            panic!("chain fixture is not an array");
        };
        let Value::Array(mut merkle) = fields[3].clone() else {
            panic!("chain Merkle evidence is not an array");
        };
        let Value::Array(paths) = merkle[1].clone() else {
            panic!("chain Merkle paths are not an array");
        };
        merkle[1] = Value::Array(vec![
            paths[name_path_count - 1].clone(),
            paths.last().unwrap().clone(),
        ]);
        fields[0] = Value::Null;
        let Value::Array(locations) = fields[1].clone() else {
            panic!("chain locations are not an array");
        };
        fields[1] = Value::Array(vec![locations.last().unwrap().clone()]);
        fields[2] = Value::Null;
        fields[3] = Value::Array(merkle);
        if team {
            fields[5] = Value::Unsigned(1);
            fields[9] = Value::Array(vec![Value::Null]);
        } else {
            fields[4] = Value::Null;
            fields[6] = Value::Unsigned(1);
            fields[7] = Value::Array(vec![Value::Null]);
        }
        encode(&Value::Array(fields)).unwrap()
    }

    #[test]
    fn official_probe_verifies_to_a_complete_hard_state_snapshot() {
        let verified = verify_public_host("foks.app", PROBE).unwrap();
        let encoded_host_id = foks_snowpack::decode(HOST_ID).unwrap();
        let Value::Binary(host_id) = encoded_host_id else {
            panic!("HostID fixture is not binary");
        };
        assert_eq!(verified.snapshot.host_id, host_id);
        assert_eq!(verified.snapshot.canonical_name, "foks.app");
        assert_eq!(verified.snapshot.chain_seqno, 1);
        assert_eq!(
            verified.snapshot.chain_tail_hash,
            [
                0x4b, 0x42, 0x9c, 0x68, 0xb8, 0x4d, 0x4b, 0xd8, 0x44, 0xd4, 0x4e, 0x08, 0xee, 0xb9,
                0xd9, 0x61, 0x9e, 0x3b, 0x95, 0x6c, 0xa6, 0xed, 0x02, 0xa7, 0x75, 0x2b, 0xd8, 0x3d,
                0x12, 0x2e, 0x84, 0x2c,
            ]
        );
        assert_eq!(verified.snapshot.services.len(), 6);
        assert_eq!(verified.snapshot.merkle_root.epoch, 995);
        assert_eq!(
            verified.snapshot.merkle_root.root_hash,
            [
                0x02, 0x0c, 0xb9, 0x06, 0x18, 0xf6, 0x1a, 0xaf, 0xf8, 0xab, 0x9a, 0xaa, 0xd9, 0x7e,
                0xdf, 0x05, 0x69, 0x2d, 0x10, 0x0d, 0x96, 0xa9, 0x67, 0x4e, 0x41, 0xe4, 0x33, 0x61,
                0xa5, 0x20, 0xf6, 0x67,
            ]
        );
        assert_eq!(verified.merkle_root.hostchain.seqno, 1);
    }

    #[test]
    fn merkle_skip_sequences_match_the_v019_reference_cases() {
        assert_eq!(merkle_backpointer_sequence(0), []);
        assert_eq!(merkle_backpointer_sequence(1), []);
        assert_eq!(merkle_backpointer_sequence(2), [1]);
        assert_eq!(merkle_backpointer_sequence(3), [2, 1]);
        assert_eq!(merkle_backpointer_sequence(4), [3, 2, 1]);
        assert_eq!(merkle_backpointer_sequence(996), [995, 994, 992]);
        assert_eq!(merkle_backpointer_sequence(998), [997, 996]);
    }

    #[test]
    fn merkle_backpointer_sequences_are_strictly_descending() {
        fn property(epoch: u64) -> bool {
            let sequence = merkle_backpointer_sequence(epoch);
            sequence.iter().all(|target| *target < epoch)
                && sequence.windows(2).all(|pair| pair[0] > pair[1])
        }
        QuickCheck::new()
            .tests(10_000)
            .quickcheck(property as fn(u64) -> bool);
    }

    #[test]
    fn unchanged_merkle_root_preserves_the_signed_bootstrap_bytes() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let unchanged = verify_merkle_advance(
            &public.snapshot.merkle_root,
            &public.merkle_root.encoded().unwrap(),
            &encode(&Value::Array(vec![Value::Null, Value::Null])).unwrap(),
            &trusted_tail(&public),
        )
        .unwrap();
        assert_eq!(unchanged.snapshot, public.snapshot.merkle_root);

        let mut rollback = public.merkle_root.clone();
        rollback.epoch -= 1;
        assert!(matches!(
            verify_merkle_advance(
                &public.snapshot.merkle_root,
                &rollback.encoded().unwrap(),
                &[],
                &trusted_tail(&public),
            ),
            Err(Error::MerkleRollback { .. })
        ));
    }

    #[test]
    fn merkle_advance_requires_a_merkle_signer_signature() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let MerkleRootEvidence::SignedBootstrap(signed) = public.snapshot.merkle_root.evidence()
        else {
            panic!("probe root is not a signed bootstrap");
        };
        let empty = encode(&Value::Array(vec![Value::Null, Value::Null])).unwrap();
        verify_signed_merkle_advance(
            &public.snapshot.merkle_root,
            signed,
            &empty,
            public.snapshot.chain_bytes(),
            &trusted_tail(&public),
        )
        .unwrap();
        assert!(verify_signed_merkle_advance(
            &public.snapshot.merkle_root,
            &public.merkle_root.encoded().unwrap(),
            &empty,
            public.snapshot.chain_bytes(),
            &trusted_tail(&public),
        )
        .is_err());
    }

    #[test]
    fn merkle_history_rejects_epoch_zero() {
        assert!(matches!(
            merkle_history_requirements(1, 0),
            Err(Error::MerkleHistoryShape)
        ));
        assert!(matches!(
            merkle_history_requirements(0, 0),
            Err(Error::MerkleHistoryShape)
        ));
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let mut root = public.merkle_root.clone();
        root.epoch = 0;
        assert!(MerkleRoot::decode(&root.encoded().unwrap()).is_err());
    }

    #[test]
    fn official_user_transitions_and_merkle_advancement_verify() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        assert_eq!(advance.snapshot.epoch, 998);
        assert_eq!(
            merkle_history_requirements(998, 995).unwrap(),
            MerkleHistoryRequest {
                full_roots: vec![996],
                hashes: vec![997, 996, 994, 992],
            }
        );
        let verified = verify_user_chain(
            USER_CHAIN,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        assert_eq!(verified.uid(), &uid);
        assert_eq!(verified.chain_seqno(), 3);
        assert_eq!(verified.devices().len(), 1);
        assert_eq!(verified.username(), b"fixtureuser");
        assert_eq!(verified.username_utf8(), b"fixtureuser");
        assert_eq!(verified.username_sequence(), 1);
        assert_eq!(verified.shared_keys()[0].generation, 2);
        assert_eq!(verified.shared_key_history().len(), 2);
        assert_eq!(
            verified
                .shared_key(Role::OWNER)
                .unwrap()
                .verify_key
                .entity_type(),
            foks_proto::ENTITY_PUK_VERIFY
        );

        let mut snapshot = verified.hard_state_snapshot().unwrap();
        let restored = restore_verified_user(
            snapshot.parts(),
            advance.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .unwrap();
        assert_eq!(restored, verified);
        snapshot.username[0] ^= 1;
        assert!(matches!(
            restore_verified_user(
                snapshot.parts(),
                advance.authenticated_roots(),
                public.snapshot.chain_bytes(),
            ),
            Err(Error::PersistedUserEvidence)
        ));
    }

    #[test]
    fn user_selected_history_is_proved_from_latest_and_tampering_fails() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let latest_hash = advance.snapshot.root_hash;
        let latest_only = VerifiedMerkleAdvance {
            root: advance.root.clone(),
            snapshot: VerifiedMerkleRoot {
                epoch: advance.snapshot.epoch,
                root_hash: latest_hash,
                root_bytes: advance.snapshot.root_bytes.clone(),
                evidence: MerkleRootEvidence::SignedBootstrap(Vec::new()),
                authenticated_roots: vec![AuthenticatedMerkleRoot {
                    epoch: advance.snapshot.epoch,
                    root_hash: latest_hash,
                    root_bytes: Some(advance.snapshot.root_bytes.clone()),
                }],
            },
            authenticated_roots: AuthenticatedMerkleRoots(BTreeMap::from([(
                advance.snapshot.epoch,
                latest_hash,
            )])),
        };
        let targets = user_chain_root_epochs(USER_CHAIN)
            .unwrap()
            .into_iter()
            .filter(|epoch| *epoch != latest_only.root.epoch)
            .collect::<Vec<_>>();
        let available_roots = BTreeMap::from([
            (995, public.merkle_root.clone()),
            (
                996,
                MerkleRoot::decode(include_bytes!(
                    "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-root-996.snowp"
                ))
                .unwrap(),
            ),
            (
                997,
                MerkleRoot::decode(include_bytes!(
                    "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-root-997.snowp"
                ))
                .unwrap(),
            ),
        ]);
        let original_requirements = merkle_history_requirements(998, 995).unwrap();
        let original_history = HistoricalMerkleRoots::decode(USER_HISTORY).unwrap();
        let available_hashes = original_requirements
            .hashes
            .into_iter()
            .zip(original_history.hashes)
            .collect::<BTreeMap<_, _>>();
        let mut full_epochs = BTreeSet::new();
        let mut hash_epochs = BTreeSet::new();
        for target in &targets {
            full_epochs.insert(*target);
            let requirements = merkle_history_requirements(998, *target).unwrap();
            full_epochs.extend(requirements.full_roots);
            hash_epochs.extend(requirements.hashes);
        }
        let full_epochs = full_epochs.into_iter().collect::<Vec<_>>();
        let hash_epochs = hash_epochs.into_iter().collect::<Vec<_>>();
        let history = HistoricalMerkleRoots {
            roots: full_epochs
                .iter()
                .map(|epoch| available_roots[epoch].clone())
                .collect(),
            hashes: hash_epochs
                .iter()
                .map(|epoch| available_hashes[epoch])
                .collect(),
        }
        .encoded()
        .unwrap();
        let roots = authenticate_historical_roots_from_latest(
            &latest_only,
            &targets,
            &full_epochs,
            &hash_epochs,
            &history,
        )
        .unwrap();
        verify_user_chain(
            USER_CHAIN,
            &uid,
            &host,
            &roots,
            &chain.merkle.root().hostchain,
        )
        .unwrap();

        let mut tampered = HistoricalMerkleRoots::decode(&history).unwrap();
        tampered.hashes[0][0] ^= 1;
        assert!(authenticate_historical_roots_from_latest(
            &latest_only,
            &targets,
            &full_epochs,
            &hash_epochs,
            &tampered.encoded().unwrap(),
        )
        .is_err());
    }

    #[test]
    fn incremental_user_refresh_accepts_noop_and_rejects_overlap() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let verified = verify_user_chain(
            USER_CHAIN,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        let no_op = incremental_noop_response(
            USER_CHAIN,
            usize::try_from(chain.num_username_links).unwrap(),
            false,
        );
        let refreshed = verify_user_chain_increment(
            &no_op,
            &verified,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        assert_eq!(refreshed, verified);

        let snapshot = refreshed.hard_state_snapshot().unwrap();
        assert_eq!(
            restore_verified_user(
                snapshot.parts(),
                advance.authenticated_roots(),
                public.snapshot.chain_bytes(),
            )
            .unwrap(),
            verified
        );
        assert!(verify_user_chain_increment(
            USER_CHAIN,
            &verified,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .is_err());
    }

    #[test]
    fn official_named_team_chain_replays_and_restores_from_exact_evidence() {
        let chain = TeamChain::decode(TEAM_CHAIN).unwrap();
        let team = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/team-id.snowp"
        ));
        let host = chain.links[0].decode_team_group_change().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            TEAM_ROOT,
            TEAM_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let verified = verify_team_chain(
            TEAM_CHAIN,
            &team,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        assert_eq!(verified.team(), &team);
        assert_eq!(verified.chain_seqno(), 1);
        assert_eq!(verified.team_name(), b"fixtureteam");
        assert_eq!(verified.team_name_utf8(), b"fixtureteam");
        assert_eq!(verified.team_name_sequence(), 1);
        assert_eq!(verified.member_load_floor(), Role::member(0));
        assert_eq!(verified.members().len(), 1);
        assert_eq!(verified.members()[0].role, Role::OWNER);
        assert_eq!(verified.shared_keys().len(), 4);

        let mut snapshot = verified.hard_state_snapshot().unwrap();
        assert_eq!(
            restore_verified_team(
                snapshot.parts(),
                advance.authenticated_roots(),
                public.snapshot.chain_bytes(),
            )
            .unwrap(),
            verified
        );
        snapshot.team_name[0] ^= 1;
        assert!(matches!(
            restore_verified_team(
                snapshot.parts(),
                advance.authenticated_roots(),
                public.snapshot.chain_bytes(),
            ),
            Err(Error::PersistedTeamEvidence)
        ));
    }

    #[test]
    fn incremental_team_refresh_accepts_noop_and_rejects_overlap() {
        let chain = TeamChain::decode(TEAM_CHAIN).unwrap();
        let team = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/team-id.snowp"
        ));
        let host = chain.links[0].decode_team_group_change().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            TEAM_ROOT,
            TEAM_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let verified = verify_team_chain(
            TEAM_CHAIN,
            &team,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        let no_op = incremental_noop_response(
            TEAM_CHAIN,
            usize::try_from(chain.num_team_name_links).unwrap(),
            true,
        );
        let refreshed = verify_team_chain_increment(
            &no_op,
            &verified,
            &team,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        assert_eq!(refreshed, verified);
        assert_eq!(
            refreshed.group_change_at(1).unwrap(),
            chain.links[0].decode_team_group_change().unwrap()
        );

        let snapshot = refreshed.hard_state_snapshot().unwrap();
        assert_eq!(
            restore_verified_team(
                snapshot.parts(),
                advance.authenticated_roots(),
                public.snapshot.chain_bytes(),
            )
            .unwrap(),
            verified
        );
        assert!(verify_team_chain_increment(
            TEAM_CHAIN,
            &verified,
            &team,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .is_err());
    }

    #[test]
    fn team_rekey_schedule_is_derived_from_roster_changes() {
        let chain = TeamChain::decode(TEAM_CHAIN).unwrap();
        let change = chain.links[0].decode_team_group_change().unwrap();
        let member_change = change.changes[0].clone();
        let member = verified_team_member(&member_change).unwrap();
        let member_key = team_member_key(
            &member.party,
            member.scoped_host.as_ref(),
            member.source_role,
        );
        let members = BTreeMap::from([(member_key, member)]);
        let keys = validate_team_shared_keys(&change, &chain.hepks, &BTreeMap::new(), true)
            .unwrap()
            .into_iter()
            .map(|key| (key.role, key))
            .collect::<BTreeMap<_, _>>();

        let mut generation_change = change.clone();
        generation_change.seqno = 2;
        generation_change.changes[0]
            .keys
            .as_mut()
            .unwrap()
            .generation += 1;
        let rotated = keys
            .values()
            .cloned()
            .map(|mut key| {
                key.generation += 1;
                key
            })
            .collect::<Vec<_>>();
        validate_team_rotation_schedule(&generation_change, &members, &keys, &rotated).unwrap();
        assert!(matches!(
            validate_team_rotation_schedule(
                &generation_change,
                &members,
                &keys,
                &rotated[..rotated.len() - 1],
            ),
            Err(Error::TeamKeySchedule)
        ));

        let mut downgrade = change;
        downgrade.seqno = 2;
        downgrade.changes[0].role = Role::ADMIN;
        let mut owner_rotation = keys[&Role::OWNER].clone();
        owner_rotation.generation += 1;
        validate_team_rotation_schedule(&downgrade, &members, &keys, &[owner_rotation]).unwrap();
    }

    #[test]
    fn persisted_merkle_anchor_is_reauthenticated_from_its_signed_bootstrap() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let root = public.snapshot.merkle_root();
        let restored = restore_merkle_anchor(
            root.epoch(),
            root.root_hash(),
            root.root_bytes(),
            root.evidence(),
            root.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .unwrap();
        assert_eq!(restored, *root);

        let mut bad_hash = root.root_hash();
        bad_hash[0] ^= 1;
        assert!(restore_merkle_anchor(
            root.epoch(),
            bad_hash,
            root.root_bytes(),
            root.evidence(),
            root.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .is_err());

        let unsigned_advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let unsigned = unsigned_advance.snapshot();
        assert!(restore_merkle_anchor(
            unsigned.epoch(),
            unsigned.root_hash(),
            unsigned.root_bytes(),
            unsigned.evidence(),
            unsigned.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .is_err());

        let mut mismatched_signature = unsigned.evidence().clone();
        let MerkleRootEvidence::SkipPath { signed_root, .. } = &mut mismatched_signature else {
            panic!("fixture advance did not use a skip path");
        };
        let MerkleRootEvidence::SignedBootstrap(bootstrap) = public.snapshot.merkle_root.evidence()
        else {
            panic!("probe root did not carry a signed bootstrap");
        };
        signed_root.clone_from(bootstrap);
        assert!(restore_merkle_anchor(
            unsigned.epoch(),
            unsigned.root_hash(),
            unsigned.root_bytes(),
            &mismatched_signature,
            unsigned.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .is_err());

        let mut bad_hostchain = public.snapshot.chain_bytes().to_vec();
        let last = bad_hostchain.len() - 1;
        bad_hostchain[last] ^= 1;
        assert!(restore_merkle_anchor(
            root.epoch(),
            root.root_hash(),
            root.root_bytes(),
            root.evidence(),
            root.authenticated_roots(),
            &bad_hostchain,
        )
        .is_err());
    }

    #[test]
    fn provisioning_requires_device_metadata_and_lower_role_for_a_new_puk() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let eldest = chain.links[0].decode_eldest().unwrap();
        let mut state = UserReplayState::from_eldest(
            VerifiedDevice {
                id: eldest.member.clone(),
                role: Role::OWNER,
                hepk: find_hepk(&chain.hepks, eldest.member_hepk_fingerprint).unwrap(),
                subkey: eldest.member_subkey.clone(),
            },
            VerifiedSharedKey {
                role: Role::OWNER,
                generation: 1,
                verify_key: eldest.puk_verify_key,
                hepk: find_hepk(&chain.hepks, eldest.puk_hepk_fingerprint).unwrap(),
            },
        );
        let provision = &chain.links[1];
        let original = provision.decode_group_change().unwrap();

        let mut missing_name = original.clone();
        missing_name.metadata.clear();
        assert!(matches!(
            state.replay(provision, &missing_name, &chain.hepks, &eldest.host),
            Err(Error::UserTransition {
                seqno: 2,
                rule: UserTransitionRule::Provisioning
            })
        ));

        let mut owner_with_puk = original;
        owner_with_puk.shared_keys = chain.links[2].decode_group_change().unwrap().shared_keys;
        assert!(matches!(
            state.replay(provision, &owner_with_puk, &chain.hepks, &eldest.host),
            Err(Error::UserTransition {
                seqno: 2,
                rule: UserTransitionRule::Provisioning
            })
        ));
    }

    #[test]
    fn user_root_and_merkle_leaf_tampering_are_rejected() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let mut wrong_roots = advance.authenticated_roots;
        wrong_roots.0.get_mut(&998).unwrap()[0] ^= 1;
        assert!(matches!(
            verify_user_chain(
                USER_CHAIN,
                &uid,
                &host,
                &wrong_roots,
                &chain.merkle.root().hostchain,
            ),
            Err(Error::UntrustedUserRoot)
        ));

        let mut bad_history = USER_HISTORY.to_vec();
        *bad_history.last_mut().unwrap() ^= 1;
        assert!(verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            &bad_history,
            &trusted_tail(&public),
        )
        .is_err());

        let mut history_value = foks_snowpack::decode(USER_HISTORY).unwrap();
        let Value::Array(fields) = &mut history_value else {
            panic!("historical fixture is not a struct");
        };
        let Value::Array(hashes) = &mut fields[1] else {
            panic!("historical hashes are not a list");
        };
        let Value::Binary(duplicate_root_hash) = &mut hashes[1] else {
            panic!("historical root hash is not binary");
        };
        duplicate_root_hash[0] ^= 1;
        assert!(matches!(
            verify_merkle_advance(
                &public.snapshot.merkle_root,
                USER_ROOT,
                &encode(&history_value).unwrap(),
                &trusted_tail(&public),
            ),
            Err(Error::MerkleBackPointer)
        ));

        let mut tampered_chain = USER_CHAIN.to_vec();
        let midpoint = tampered_chain.len() / 2;
        tampered_chain[midpoint] ^= 1;
        assert!(verify_user_chain(
            &tampered_chain,
            &uid,
            &host,
            &wrong_roots,
            &chain.merkle.root().hostchain,
        )
        .is_err());
    }

    #[test]
    fn user_response_mutations_cannot_change_verified_state() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let baseline = verify_user_chain(
            USER_CHAIN,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();

        let mut tampered = USER_CHAIN.to_vec();
        for index in 0..tampered.len() {
            tampered[index] ^= 1;
            if let Ok(candidate) = verify_user_chain(
                &tampered,
                &uid,
                &host,
                advance.authenticated_roots(),
                &chain.merkle.root().hostchain,
            ) {
                assert_eq!(
                    candidate, baseline,
                    "unauthenticated byte {index} changed verified state"
                );
            }
            tampered[index] ^= 1;
        }
    }

    #[test]
    fn disclosed_username_and_device_names_are_not_malleable() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        for needle in [b"fixtureuser".as_slice(), b"fixture-device".as_slice()] {
            let offsets = USER_CHAIN
                .windows(needle.len())
                .enumerate()
                .filter_map(|(offset, value)| (value == needle).then_some(offset))
                .collect::<Vec<_>>();
            assert!(
                !offsets.is_empty(),
                "fixture no longer contains disclosed field"
            );
            for offset in offsets {
                let mut tampered = USER_CHAIN.to_vec();
                tampered[offset] ^= 1;
                assert!(
                    verify_user_chain(
                        &tampered,
                        &uid,
                        &host,
                        advance.authenticated_roots(),
                        &chain.merkle.root().hostchain,
                    )
                    .is_err(),
                    "disclosed field mutation at {offset} was accepted"
                );
            }
        }
    }

    #[test]
    fn names_match_the_v019_unicode_normalization_matrix() {
        for (input, expected) in [
            ("max", Some("max")),
            ("m_a.x", Some("m_a_x")),
            ("Wkładam-Kurtkę", Some("wkladam_kurtke")),
            ("für_Ihre_Beiträge", Some("fur_ihre_beitrage")),
            ("nå_mål", Some("na_mal")),
            ("Æok", Some("aok")),
            ("ßßs", Some("sss")),
            ("Vláda_zvyšuje", Some("vlada_zvysuje")),
            ("m+a+x", None),
            ("m__a_x", None),
            ("这是书", None),
        ] {
            assert_eq!(
                normalize_username(input.as_bytes()).as_deref(),
                expected.map(str::as_bytes),
                "{input}"
            );
        }
        for (input, expected) in [
            ("max's iPhone", Some("max's iphone")),
            ("M_A.X 7.4+ Bizzle-", Some("m_a.x 7.4+ bizzle-")),
            ("Wkładam_Kurtkę-", Some("wkladam_kurtke-")),
            ("a-t-il réagi ça ne", Some("a-t-il reagi ca ne")),
            ("Æok", None),
            ("Łukasz", None),
            ("maa__a", None),
            ("maaa ", None),
            ("a’b’c’", None),
        ] {
            assert_eq!(
                normalize_device_name(input.as_bytes()).as_deref(),
                expected.map(str::as_bytes),
                "{input}"
            );
        }
    }

    #[test]
    fn canonical_public_zone_tampering_fails_signature_verification() {
        let mut tampered = PROBE.to_vec();
        let offset = tampered
            .windows(b"foks.app:4430".len())
            .position(|window| window == b"foks.app:4430")
            .unwrap();
        tampered[offset] = b'g';
        assert!(matches!(
            verify_public_host("foks.app", &tampered),
            Err(Error::DelegatedSignature("host metadata"))
        ));
    }

    #[test]
    fn persisted_public_zone_is_reauthenticated_before_service_reuse() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let snapshot = &public.snapshot;
        let identity = restore_public_host_identity(
            snapshot.host_id(),
            snapshot.genesis_key(),
            snapshot.chain_seqno(),
            snapshot.chain_tail_hash(),
            snapshot.chain_bytes(),
            snapshot.public_zone_bytes(),
        )
        .unwrap();
        assert_eq!(identity.services(), snapshot.services());

        let mut zone = snapshot.public_zone_bytes().to_vec();
        *zone.last_mut().unwrap() ^= 1;
        assert!(restore_public_host_identity(
            snapshot.host_id(),
            snapshot.genesis_key(),
            snapshot.chain_seqno(),
            snapshot.chain_tail_hash(),
            snapshot.chain_bytes(),
            &zone,
        )
        .is_err());
    }

    #[test]
    fn hostchain_signature_tampering_is_rejected() {
        let mut probe = ProbeResponse::decode(PROBE).unwrap();
        let foks_proto::Signature::Ed25519(signature) = &mut probe.hostchain[0].signatures[0]
        else {
            panic!("fixture uses unexpected signature type");
        };
        signature[0] ^= 1;
        assert!(verify_hostchain(&probe.hostchain).is_err());
    }

    #[test]
    fn merkle_binding_is_checked_separately_from_its_signature() {
        let probe = ProbeResponse::decode(PROBE).unwrap();
        let chain = verify_hostchain(&probe.hostchain).unwrap();
        let mut root = MerkleRoot::decode(&probe.merkle_root.inner).unwrap();
        root.hostchain.hash[0] ^= 1;
        assert!(matches!(
            verify_merkle_binding(&chain, &root),
            Err(Error::MerkleHostchainMismatch)
        ));
    }

    #[test]
    fn canonical_address_parser_handles_dns_and_ipv6() {
        assert_eq!(canonical_host("foks.app:4430").unwrap(), "foks.app");
        assert_eq!(canonical_host("foks.app").unwrap(), "foks.app");
        assert_eq!(canonical_host("[::1]:4430").unwrap(), "::1");
        assert!(canonical_host("[]:4430").is_err());
        assert!(canonical_host("::1").is_err());
        assert!(canonical_host(":4430").is_err());
    }

    #[test]
    fn valid_extension_and_revocation_links_verify() {
        let original = SigningKey::from_bytes(&[1; 32]);
        let added = SigningKey::from_bytes(&[2; 32]);
        let host = entity_id(ENTITY_HOST, &original);
        let added_host = entity_id(ENTITY_HOST, &added);

        let mut genesis = link(1, None, &host, &host, Value::Null, vec![&original]);
        let genesis_hash =
            prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &genesis.encoded().unwrap()).unwrap();
        let add_change = Value::Array(vec![Value::Array(vec![
            Value::Unsigned(2),
            Value::Variant(Some((
                b"2".to_vec(),
                Box::new(Value::Binary(added_host.as_bytes().to_vec())),
            ))),
        ])]);
        let extension = link(
            2,
            Some(genesis_hash),
            &host,
            &host,
            add_change,
            vec![&added, &original],
        );

        let extension_hash =
            prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &extension.encoded().unwrap()).unwrap();
        let revoke_change = Value::Array(vec![Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Binary(added_host.as_bytes().to_vec())),
            ))),
        ])]);
        let revocation = link(
            3,
            Some(extension_hash),
            &host,
            &host,
            revoke_change,
            vec![&original],
        );

        let state = verify_hostchain(&[genesis.clone(), extension, revocation]).unwrap();
        assert_eq!(state.seqno, 3);
        assert!(state.revoked.contains(&added_host));
        assert!(!state.active_keys(ENTITY_HOST).any(|key| key == &added_host));

        genesis.signatures[0] = foks_proto::Signature::Ed25519([0; 64]);
        assert!(verify_hostchain(&[genesis]).is_err());
    }

    #[test]
    fn redelegation_restores_non_chain_signers_only() {
        let original = SigningKey::from_bytes(&[1; 32]);
        let delegated = SigningKey::from_bytes(&[3; 32]);
        let host = entity_id(ENTITY_HOST, &original);
        let metadata = entity_id(ENTITY_HOST_METADATA_SIGNER, &delegated);
        let key_change = |key: &EntityId| {
            Value::Array(vec![Value::Array(vec![
                Value::Unsigned(2),
                Value::Variant(Some((
                    b"2".to_vec(),
                    Box::new(Value::Binary(key.as_bytes().to_vec())),
                ))),
            ])])
        };
        let revoke_change = |key: &EntityId| {
            Value::Array(vec![Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Binary(key.as_bytes().to_vec())),
                ))),
            ])])
        };

        let genesis = link(
            1,
            None,
            &host,
            &host,
            key_change(&metadata),
            vec![&delegated, &original],
        );
        let genesis_hash =
            prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &genesis.encoded().unwrap()).unwrap();
        let revocation = link(
            2,
            Some(genesis_hash),
            &host,
            &host,
            revoke_change(&metadata),
            vec![&original],
        );
        let revocation_hash =
            prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &revocation.encoded().unwrap()).unwrap();
        let redelegation = link(
            3,
            Some(revocation_hash),
            &host,
            &host,
            key_change(&metadata),
            vec![&delegated, &original],
        );
        let state = verify_hostchain(&[genesis, revocation, redelegation]).unwrap();
        assert!(!state.revoked.contains(&metadata));
        assert!(state
            .active_keys(ENTITY_HOST_METADATA_SIGNER)
            .any(|key| key == &metadata));

        let mut state = HostchainState::default();
        state.revoked.insert(host.clone());
        state
            .keys
            .entry(ENTITY_HOST)
            .or_default()
            .push(host.clone());
        assert!(state.revoked.contains(&host));
        assert!(!state.active_keys(ENTITY_HOST).any(|key| key == &host));
    }

    fn entity_id(entity_type: u8, key: &SigningKey) -> EntityId {
        let mut bytes = vec![entity_type];
        bytes.extend(key.verifying_key().as_bytes());
        EntityId::from_bytes(bytes).unwrap()
    }

    fn link(
        seqno: u64,
        previous: Option<[u8; 32]>,
        host: &EntityId,
        signer: &EntityId,
        changes: Value,
        signing_keys: Vec<&SigningKey>,
    ) -> HostchainLink {
        let inner = encode(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Array(vec![
                        Value::Unsigned(seqno),
                        previous.map_or(Value::Null, |hash| Value::Binary(hash.to_vec())),
                        Value::Array(vec![Value::Unsigned(0), Value::Binary(vec![0; 32])]),
                        Value::Unsigned(seqno),
                    ]),
                    Value::Binary(host.as_bytes().to_vec()),
                    Value::Binary(signer.as_bytes().to_vec()),
                    changes,
                ])),
            ))),
        ]))
        .unwrap();
        let mut link = HostchainLink {
            inner,
            signatures: Vec::new(),
        };
        for key in signing_keys {
            let object = link.signing_bytes(link.signatures.len()).unwrap();
            let mut message = Vec::with_capacity(8 + object.len());
            message.extend(HOSTCHAIN_LINK_OUTER_V1_TYPE_ID.to_be_bytes());
            message.extend(object);
            link.signatures.push(foks_proto::Signature::Ed25519(
                key.sign(&message).to_bytes(),
            ));
        }
        link
    }
}
