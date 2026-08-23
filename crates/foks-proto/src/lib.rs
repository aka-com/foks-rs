//! Exact, schema-checked types for the implemented FOKS v0.1.9 protocols.
//!
//! The generic Snowpack codec proves canonical encoding. This crate applies
//! the v0.1.9 host, identity, team, key-distribution, mutation, and KV schemas
//! without translating authenticated bytes through JSON or AKA types.

#![forbid(unsafe_code)]

use foks_snowpack::{decode, encode, Value};
use thiserror::Error;

pub const HOSTCHAIN_LINK_OUTER_V1_TYPE_ID: u64 = 0xa23b_a362_0d75_8f7a;
pub const HOSTCHAIN_LINK_OUTER_TYPE_ID: u64 = 0x8d87_ac22_4920_355c;
pub const MERKLE_ROOT_TYPE_ID: u64 = 0xa88f_c49b_6df3_a111;
pub const MERKLE_ROOT_BLOB_TYPE_ID: u64 = 0xa22f_0c09_21d4_e651;
pub const PUBLIC_ZONE_BLOB_TYPE_ID: u64 = 0xd4f1_ec4f_90eb_2c6d;
pub const LINK_OUTER_V1_TYPE_ID: u64 = 0xc274_5284_af61_7745;
pub const LINK_OUTER_TYPE_ID: u64 = 0xed4c_c0f7_0817_32b6;
pub const SHARED_KEY_SEED_TYPE_ID: u64 = 0xa999_8e7a_59e8_ae25;
pub const HEPK_V1_TYPE_ID: u64 = 0x9c26_7d45_631b_b8c1;
pub const HEPK_TYPE_ID: u64 = 0x9c58_1bee_d36c_7e0c;
pub const HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID: u64 = 0x8a9e_3276_4726_2289;
pub const TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID: u64 = 0xd51b_5d99_0285_023e;
pub const SUBKEY_SEED_TYPE_ID: u64 = 0x9bfc_a0e8_fc32_288f;
pub const MERKLE_TREE_RF_INPUT_TYPE_ID: u64 = 0xb0e2_68f3_88ac_c97a;
pub const MERKLE_NODE_TYPE_ID: u64 = 0xe941_750d_c5b9_6783;
pub const MERKLE_BACK_POINTERS_TYPE_ID: u64 = 0x8c7c_4b85_5fba_9000;
pub const TREE_LOCATION_TYPE_ID: u64 = 0xaeff_d88b_6cd2_67d9;
pub const NAME_COMMITMENT_TYPE_ID: u64 = 0xe37b_1fcf_ba97_2353;
pub const DEVICE_LABEL_TYPE_ID: u64 = 0x9650_2272_0548_6122;
pub const NAME_HASH_PREIMAGE_TYPE_ID: u64 = 0xf855_6f05_4c4e_036b;
pub const ENTITY_ID_MERKLE_VALUE_TYPE_ID: u64 = 0xd3d2_1c7d_c1d6_4ea1;
pub const TEAM_VIEW_CHALLENGE_TYPE_ID: u64 = 0x9686_1830_ffa9_6bff;
pub const APP_KEY_DERIVATION_TYPE_ID: u64 = 0x9431_8317_830b_409b;
pub const KV_KEY_DERIVATION_TYPE_ID: u64 = 0xdbdf_2ba2_9c0d_e2cb;
pub const KV_DIRENT_NAME_PAYLOAD_TYPE_ID: u64 = 0xb9c1_587f_a732_c2c9;
pub const KV_DIRENT_BINDING_PAYLOAD_TYPE_ID: u64 = 0x9cc3_7c83_63dc_39fa;
pub const KV_ROOT_BINDING_PAYLOAD_TYPE_ID: u64 = 0xcfac_dd4e_ab21_3a36;
pub const KV_FILE_KEY_PAYLOAD_TYPE_ID: u64 = 0x9211_ae1e_1721_3884;
pub const KV_CHUNK_NONCE_PAYLOAD_TYPE_ID: u64 = 0xadba_174b_7e8d_cc08;

pub const ENTITY_HOST: u8 = 2;
pub const ENTITY_USER: u8 = 1;
pub const ENTITY_NAMED_TEAM: u8 = 3;
pub const ENTITY_HOST_MERKLE_SIGNER: u8 = 10;
pub const ENTITY_HOST_TLS_CA: u8 = 11;
pub const ENTITY_HOST_METADATA_SIGNER: u8 = 12;
pub const ENTITY_DEVICE: u8 = 4;
pub const ENTITY_YUBI: u8 = 8;
pub const ENTITY_SUBKEY: u8 = 13;
pub const ENTITY_PUK_VERIFY: u8 = 14;
pub const ENTITY_PTK_VERIFY: u8 = 15;
pub const ENTITY_AD_HOC_TEAM: u8 = 20;

mod codec;
mod entity;
mod error;
mod host;
mod identity;
mod key_material;
mod kv;
mod role;
mod service;

pub use entity::*;
pub use error::*;
pub use host::*;
pub use identity::*;
pub use key_material::*;
pub use kv::*;
pub use role::*;
pub use service::*;

pub(crate) use codec::{
    array, binary, boolean, device_entity, entity, expect_unsigned, fixed_blob, integer, list,
    option, text, text_bytes, type_error, unsigned, variant,
};

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );

    fn user_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "../foks-snowpack/tests/fixtures/foks-v0.1.9/user/{name}"
        ))
        .unwrap()
    }

    #[test]
    fn official_probe_decodes_into_exact_schema_types() {
        let probe = ProbeResponse::decode(FIXTURE).unwrap();
        assert_eq!(probe.hostchain.len(), 1);
        let change = probe.hostchain[0].decode_change().unwrap();
        assert_eq!(change.chainer.seqno, 1);
        assert_eq!(change.host, change.signer);
        assert_eq!(change.changes.len(), 3);
        assert_eq!(probe.hostchain[0].signatures.len(), 4);

        let zone = PublicZone::decode(&probe.public_zone.inner).unwrap();
        assert_eq!(zone.ttl_seconds, 60);
        assert_eq!(zone.services.probe, "foks.app:4430");

        let root = MerkleRoot::decode(&probe.merkle_root.inner).unwrap();
        assert_eq!(root.epoch, 995);
        assert_eq!(root.hostchain.seqno, 1);
    }

    #[test]
    fn exact_nested_wire_objects_are_retained() {
        let probe = ProbeResponse::decode(FIXTURE).unwrap();
        assert_eq!(
            probe.hostchain[0].encoded().unwrap(),
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/hostchain-link-0001.snowp"
            )
        );
        assert_eq!(
            probe.public_zone.encoded().unwrap(),
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/signed-public-zone.snowp"
            )
        );
        assert_eq!(
            probe.merkle_root.encoded().unwrap(),
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/signed-merkle-root.snowp"
            )
        );
    }

    #[test]
    fn malformed_schema_is_rejected_after_canonical_decode() {
        let wrong_arity = encode(&Value::Array(vec![Value::Null])).unwrap();
        assert!(matches!(
            ProbeResponse::decode(&wrong_arity),
            Err(Error::FieldCount { .. })
        ));

        let bad_entity = EntityId::from_bytes(vec![2; 32]).unwrap_err();
        assert!(matches!(bad_entity, Error::Length { .. }));
        assert!(EntityId::from_bytes(vec![0xff; 33]).is_err());
    }

    #[test]
    fn service_types_are_exact_and_reject_unknown_values() {
        for service in [
            ServiceType::Registration,
            ServiceType::User,
            ServiceType::MerkleQuery,
            ServiceType::Probe,
            ServiceType::KvStore,
            ServiceType::Realtime,
        ] {
            assert_eq!(
                ServiceType::try_from(service.protocol_value()).unwrap(),
                service
            );
        }
        assert!(matches!(
            ServiceType::try_from(0),
            Err(Error::UnknownEnum {
                kind: "service type",
                value: 0
            })
        ));
        assert!(ServiceType::try_from(u64::MAX).is_err());
    }

    #[test]
    fn roles_enforce_v019_type_and_visibility_rules() {
        assert_eq!(
            role(&Value::Array(vec![
                Value::Unsigned(0),
                Value::Variant(None)
            ]))
            .unwrap(),
            Role::NONE
        );
        assert_eq!(
            role(&Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((b"0".to_vec(), Box::new(Value::Negative(-32_768)))))
            ]))
            .unwrap(),
            Role::member(-32_768)
        );
        assert!(role(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((b"0".to_vec(), Box::new(Value::Negative(-32_769)))))
        ]))
        .is_err());
        assert!(role(&Value::Array(vec![
            Value::Unsigned(3),
            Value::Variant(Some((b"0".to_vec(), Box::new(Value::Unsigned(0)))))
        ]))
        .is_err());
        assert!(role(&Value::Array(vec![
            Value::Unsigned(4),
            Value::Variant(None)
        ]))
        .is_err());
    }

    #[test]
    fn official_kv_objects_decode_with_exact_nested_bytes() {
        let root = KvRoot::decode(&user_fixture("kv-root.snowp")).unwrap();
        assert_eq!(root.encoded(), user_fixture("kv-root.snowp"));
        assert_eq!(root.key.role, Role::member(-16_384));
        let directory = KvDirectoryPair::decode(&user_fixture("kv-root-dir.snowp")).unwrap();
        assert_eq!(directory.encoded(), user_fixture("kv-root-dir.snowp"));
        assert_eq!(directory.active.id, root.root);
        let listing = KvListResponse::decode(&user_fixture("kv-list.snowp")).unwrap();
        assert_eq!(listing.encoded(), user_fixture("kv-list.snowp"));
        assert_eq!(listing.entries.len(), 3);
        assert!(matches!(
            KvNode::decode(&user_fixture("kv-small-node.snowp")).unwrap(),
            KvNode::SmallFile(_)
        ));
        assert!(matches!(
            KvNode::decode(&user_fixture("kv-symlink-node.snowp")).unwrap(),
            KvNode::Symlink(_)
        ));
        assert!(matches!(
            KvNode::decode(&user_fixture("kv-large-node.snowp")).unwrap(),
            KvNode::File(_)
        ));
        KvEncryptedChunk::decode(&user_fixture("kv-large-chunk.snowp")).unwrap();
        let versions =
            KvPathVersionVector::decode(&user_fixture("kv-path-version-vector.snowp")).unwrap();
        assert_eq!(
            versions.encode().unwrap(),
            user_fixture("kv-path-version-vector.snowp")
        );
        assert_eq!(versions.root_version, 1);
        assert_eq!(versions.directories.len(), 1);
        assert_eq!(versions.directories[0].entries.len(), 3);
    }

    #[test]
    fn malformed_kv_tags_and_widths_are_rejected() {
        let bad_node = encode(&Value::Array(vec![
            Value::Unsigned(5),
            Value::Variant(None),
        ]))
        .unwrap();
        assert!(matches!(
            KvNode::decode(&bad_node),
            Err(Error::UnknownEnum { .. })
        ));
        assert!(kv_node_id(&Value::Binary(vec![3; 16])).is_err());
        let mut root = user_fixture("kv-root.snowp");
        root.push(0);
        assert!(KvRoot::decode(&root).is_err());
    }
}
