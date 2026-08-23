//! Exact, schema-checked types for the FOKS v0.1.9 public probe path.
//!
//! The generic Snowpack codec proves canonical encoding. This crate applies
//! the v0.1.9 schema without translating authenticated bytes through JSON or
//! application-level AKA types.

#![forbid(unsafe_code)]

use foks_snowpack::{decode, encode, Value};
use thiserror::Error;
use zeroize::Zeroizing;

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

pub const ENTITY_HOST: u8 = 2;
pub const ENTITY_USER: u8 = 1;
pub const ENTITY_HOST_MERKLE_SIGNER: u8 = 10;
pub const ENTITY_HOST_TLS_CA: u8 = 11;
pub const ENTITY_HOST_METADATA_SIGNER: u8 = 12;
pub const ENTITY_DEVICE: u8 = 4;
pub const ENTITY_YUBI: u8 = 8;
pub const ENTITY_SUBKEY: u8 = 13;
pub const ENTITY_PUK_VERIFY: u8 = 14;

pub const SERVICE_REG: u64 = 1;
pub const SERVICE_USER: u64 = 2;
pub const SERVICE_MERKLE_QUERY: u64 = 5;
pub const SERVICE_PROBE: u64 = 10;
pub const SERVICE_KV_STORE: u64 = 12;
pub const SERVICE_REALTIME: u64 = 16;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum RoleType {
    None = 0,
    Member = 1,
    Admin = 2,
    Owner = 3,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Role {
    kind: RoleType,
    visibility: i16,
}

impl Role {
    pub const NONE: Self = Self::new_default(RoleType::None);
    pub const ADMIN: Self = Self::new_default(RoleType::Admin);
    pub const OWNER: Self = Self::new_default(RoleType::Owner);

    pub const fn member(visibility: i16) -> Self {
        Self {
            kind: RoleType::Member,
            visibility,
        }
    }

    pub const fn kind(self) -> RoleType {
        self.kind
    }

    pub const fn visibility(self) -> Option<i16> {
        match self.kind {
            RoleType::Member => Some(self.visibility),
            _ => None,
        }
    }

    pub const fn protocol_value(self) -> u64 {
        self.kind as u64
    }

    const fn new_default(kind: RoleType) -> Self {
        Self {
            kind,
            visibility: 0,
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid canonical Snowpack: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("expected {expected}, found {found}")]
    Type {
        expected: &'static str,
        found: &'static str,
    },
    #[error("expected {expected} fields, found {found}")]
    FieldCount { expected: usize, found: usize },
    #[error("expected variant tag {expected:?}, found {found:?}")]
    VariantTag { expected: String, found: Vec<u8> },
    #[error("unknown {kind} value {value}")]
    UnknownEnum { kind: &'static str, value: u64 },
    #[error("{kind} has length {found}, expected {expected}")]
    Length {
        kind: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("invalid entity type {0}")]
    EntityType(u8),
    #[error("expected entity type {expected}, found {found}")]
    WrongEntityType { expected: u8, found: u8 },
    #[error("integer does not fit {0}")]
    IntegerRange(&'static str),
    #[error("text is not UTF-8")]
    Utf8,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct EntityId(Vec<u8>);

impl EntityId {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let Some(&entity_type) = bytes.first() else {
            return Err(Error::Length {
                kind: "EntityID",
                expected: 33,
                found: 0,
            });
        };
        let expected = match entity_type {
            1..=7 | 9..=21 => 33,
            8 => 34,
            _ => return Err(Error::EntityType(entity_type)),
        };
        if bytes.len() != expected {
            return Err(Error::Length {
                kind: "EntityID",
                expected,
                found: bytes.len(),
            });
        }
        Ok(Self(bytes))
    }

    pub fn require_type(self, expected: u8) -> Result<Self> {
        if self.entity_type() != expected {
            return Err(Error::WrongEntityType {
                expected,
                found: self.entity_type(),
            });
        }
        Ok(self)
    }

    pub fn entity_type(&self) -> u8 {
        self.0[0]
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn ed25519_key(&self) -> Result<[u8; 32]> {
        if !is_ed25519_entity(self.entity_type()) {
            return Err(Error::EntityType(self.entity_type()));
        }
        self.0[1..].try_into().map_err(|_| Error::Length {
            kind: "Ed25519 EntityID",
            expected: 33,
            found: self.0.len(),
        })
    }

    pub fn p256_key(&self) -> Result<[u8; 33]> {
        if self.entity_type() != 8 {
            return Err(Error::EntityType(self.entity_type()));
        }
        self.0[1..].try_into().map_err(|_| Error::Length {
            kind: "P-256 EntityID",
            expected: 34,
            found: self.0.len(),
        })
    }
}

fn is_ed25519_entity(entity_type: u8) -> bool {
    matches!(
        entity_type,
        1..=7 | 10..=17 | 19 | 20
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Signature {
    Ed25519([u8; 64]),
    Ecdsa(Vec<u8>),
}

impl Signature {
    pub fn to_value(&self) -> Value {
        match self {
            Self::Ed25519(bytes) => Value::Array(vec![
                Value::Unsigned(0),
                Value::Variant(Some((
                    b"0".to_vec(),
                    Box::new(Value::Binary(bytes.to_vec())),
                ))),
            ]),
            Self::Ecdsa(bytes) => Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Binary(bytes.clone())),
                ))),
            ]),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedBlob {
    pub inner: Vec<u8>,
    pub signature: Signature,
}

impl SignedBlob {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        signed_blob(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.inner.clone()),
            self.signature.to_value(),
        ]))?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeResponse {
    pub merkle_root: SignedBlob,
    pub public_zone: SignedBlob,
    pub hostchain: Vec<HostchainLink>,
}

impl ProbeResponse {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 3)?;
        Ok(Self {
            merkle_root: signed_blob(&fields[0])?,
            public_zone: signed_blob(&fields[1])?,
            hostchain: list(&fields[2], hostchain_link)?,
        })
    }

    pub fn encoded_hostchain(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(
            self.hostchain.iter().map(HostchainLink::to_value).collect(),
        ))?)
    }
}

/// Decodes the exact standalone host-chain array stored as durable hard state.
pub fn decode_hostchain(bytes: &[u8]) -> Result<Vec<HostchainLink>> {
    list(&decode(bytes)?, hostchain_link)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostchainLink {
    pub inner: Vec<u8>,
    pub signatures: Vec<Signature>,
}

impl HostchainLink {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn signing_bytes(&self, signature_count: usize) -> Result<Vec<u8>> {
        if signature_count > self.signatures.len() {
            return Err(Error::Length {
                kind: "signature prefix",
                expected: self.signatures.len(),
                found: signature_count,
            });
        }
        let signatures = if signature_count == 0 {
            Value::Null
        } else {
            Value::Array(
                self.signatures[..signature_count]
                    .iter()
                    .map(Signature::to_value)
                    .collect(),
            )
        };
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.inner.clone()),
            signatures,
        ]))?)
    }

    pub fn decode_change(&self) -> Result<HostchainChange> {
        let inner = decode(&self.inner)?;
        let fields = array(&inner, 2)?;
        expect_unsigned(&fields[0], "hostchain link type", 1)?;
        hostchain_change(variant(&fields[1], "1")?)
    }

    fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Binary(self.inner.clone()),
                    signature_list(&self.signatures),
                ])),
            ))),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRoot {
    pub epoch: u64,
    pub hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaseChainer {
    pub seqno: u64,
    pub previous: Option<[u8; 32]>,
    pub root: TreeRoot,
    pub time: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostchainChange {
    pub chainer: BaseChainer,
    pub host: EntityId,
    pub signer: EntityId,
    pub changes: Vec<HostchainChangeItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostchainChangeItem {
    Revoke(EntityId),
    Key(EntityId),
    TlsCa { id: EntityId, certificate: Vec<u8> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicServices {
    pub probe: String,
    pub registration: String,
    pub user: String,
    pub merkle_query: String,
    pub kv_store: String,
    pub realtime: String,
}

impl PublicServices {
    pub fn entries(&self) -> [(u64, &str); 6] {
        [
            (SERVICE_PROBE, &self.probe),
            (SERVICE_REG, &self.registration),
            (SERVICE_USER, &self.user),
            (SERVICE_MERKLE_QUERY, &self.merkle_query),
            (SERVICE_KV_STORE, &self.kv_store),
            (SERVICE_REALTIME, &self.realtime),
        ]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicZone {
    pub ttl_seconds: i64,
    pub services: PublicServices,
}

impl PublicZone {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 2)?;
        let services = array(&fields[1], 6)?;
        Ok(Self {
            ttl_seconds: integer(&fields[0])?,
            services: PublicServices {
                probe: text(&services[0])?,
                registration: text(&services[1])?,
                user: text(&services[2])?,
                merkle_query: text(&services[3])?,
                kv_store: text(&services[4])?,
                realtime: text(&services[5])?,
            },
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostchainTail {
    pub seqno: u64,
    pub hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerkleRoot {
    pub epoch: u64,
    pub time: u64,
    pub back_pointers: [u8; 32],
    pub root_node: [u8; 32],
    pub hostchain: HostchainTail,
}

impl MerkleRoot {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 2)?;
        expect_unsigned(&fields[0], "Merkle root version", 1)?;
        let v1 = array(variant(&fields[1], "1")?, 5)?;
        let tail = array(&v1[4], 2)?;
        Ok(Self {
            epoch: unsigned(&v1[0])?,
            time: unsigned(&v1[1])?,
            back_pointers: fixed_blob(&v1[2], "Merkle back-pointer hash")?,
            root_node: fixed_blob(&v1[3], "Merkle root node")?,
            hostchain: HostchainTail {
                seqno: unsigned(&tail[0])?,
                hash: fixed_blob(&tail[1], "hostchain tail hash")?,
            },
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Unsigned(self.epoch),
                    Value::Unsigned(self.time),
                    Value::Binary(self.back_pointers.to_vec()),
                    Value::Binary(self.root_node.to_vec()),
                    Value::Array(vec![
                        Value::Unsigned(self.hostchain.seqno),
                        Value::Binary(self.hostchain.hash.to_vec()),
                    ]),
                ])),
            ))),
        ]))?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerkleBackPointer {
    pub epoch: u64,
    pub hash: [u8; 32],
}

pub fn decode_merkle_back_pointers(bytes: &[u8]) -> Result<Vec<MerkleBackPointer>> {
    list(&decode(bytes)?, |value| {
        let fields = array(value, 2)?;
        Ok(MerkleBackPointer {
            epoch: unsigned(&fields[0])?,
            hash: fixed_blob(&fields[1], "Merkle root hash")?,
        })
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalMerkleRoots {
    pub roots: Vec<MerkleRoot>,
    pub hashes: Vec<[u8; 32]>,
}

impl HistoricalMerkleRoots {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 2)?;
        Ok(Self {
            roots: list(&fields[0], |value| MerkleRoot::decode(&encode(value)?))?,
            hashes: list(&fields[1], |value| fixed_blob(value, "Merkle root hash"))?,
        })
    }
}

/// Exact v0.1.9 user-chain outer link. The inner blob is retained verbatim
/// because both stacked signatures and the Merkle leaf commit to these bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserLink {
    inner: Vec<u8>,
    signatures: Vec<Signature>,
    exact: Vec<u8>,
}

impl UserLink {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        user_link(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(self.exact.clone())
    }

    pub fn signatures(&self) -> &[Signature] {
        &self.signatures
    }

    pub fn signing_bytes(&self, signature_count: usize) -> Result<Vec<u8>> {
        if signature_count > self.signatures.len() {
            return Err(Error::Length {
                kind: "signature prefix",
                expected: self.signatures.len(),
                found: signature_count,
            });
        }
        let signatures = if signature_count == 0 {
            Value::Null
        } else {
            Value::Array(
                self.signatures[..signature_count]
                    .iter()
                    .map(Signature::to_value)
                    .collect(),
            )
        };
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.inner.clone()),
            signatures,
        ]))?)
    }

    /// Decodes the security-critical fields of an eldest group-change link.
    pub fn decode_eldest(&self) -> Result<UserEldest> {
        let inner = decode(&self.inner)?;
        let inner_fields = array(&inner, 2)?;
        expect_unsigned(&inner_fields[0], "link inner version", 1)?;
        let group = array(variant(&inner_fields[1], "0")?, 8)?;
        let hiding = array(&group[0], 2)?;
        let chainer = array(&hiding[0], 4)?;
        let root = array(&chainer[2], 2)?;
        let fq_user = array(&group[1], 2)?;
        let signer = array(&group[2], 2)?;
        let changes = list_values(&group[3])?;
        let shared_keys = list_values(&group[5])?;
        if changes.len() != 1 || shared_keys.len() != 1 {
            return Err(Error::FieldCount {
                expected: 1,
                found: changes.len().max(shared_keys.len()),
            });
        }
        let member_role = array(changes[0], 2)?;
        let owner_role = role(&member_role[0])?;
        if owner_role != Role::OWNER {
            return Err(Error::UnknownEnum {
                kind: "eldest member role",
                value: owner_role.protocol_value(),
            });
        }
        let member = array(&member_role[1], 3)?;
        let member_fqe = array(&member[0], 2)?;
        let member_keys = array(&member[2], 2)?;
        expect_unsigned(&member_keys[0], "member key version", 1)?;
        let member_key_v1 = array(variant(&member_keys[1], "1")?, 2)?;
        let shared = array(shared_keys[0], 4)?;
        let shared_role = role(&shared[1])?;
        if shared_role != Role::OWNER {
            return Err(Error::UnknownEnum {
                kind: "eldest shared-key role",
                value: shared_role.protocol_value(),
            });
        }
        Ok(UserEldest {
            seqno: unsigned(&chainer[0])?,
            previous: option(&chainer[1], |v| fixed_blob(v, "previous link hash"))?,
            root: TreeRoot {
                epoch: unsigned(&root[0])?,
                hash: fixed_blob(&root[1], "Merkle root hash")?,
            },
            time: unsigned(&chainer[3])?,
            next_tree_location: fixed_blob(&hiding[1], "next tree location")?,
            uid: entity(&fq_user[0])?.require_type(ENTITY_USER)?,
            host: entity(&fq_user[1])?.require_type(ENTITY_HOST)?,
            signer: device_entity(&signer[0])?,
            member: device_entity(&member_fqe[0])?,
            member_verify_key: device_entity(&member_fqe[0])?,
            member_role: owner_role,
            member_source_role: role(&member[1])?,
            member_scoped_host: option(&member_fqe[1], |value| {
                entity(value)?.require_type(ENTITY_HOST)
            })?,
            member_hepk_fingerprint: fixed_blob(&member_key_v1[0], "HEPK fingerprint")?,
            member_subkey: option(&member_key_v1[1], |value| entity(value)?.require_type(13))?,
            puk_generation: unsigned(&shared[0])?,
            puk_verify_key: entity(&shared[2])?.require_type(ENTITY_PUK_VERIFY)?,
            puk_hepk_fingerprint: fixed_blob(&shared[3], "PUK HEPK fingerprint")?,
            metadata: list(&group[6], change_metadata)?,
        })
    }

    pub fn decode_group_change(&self) -> Result<UserGroupChange> {
        let inner = decode(&self.inner)?;
        let inner_fields = array(&inner, 2)?;
        expect_unsigned(&inner_fields[0], "link inner version", 1)?;
        let group = array(variant(&inner_fields[1], "0")?, 8)?;
        let hiding = array(&group[0], 2)?;
        let chainer = array(&hiding[0], 4)?;
        let root = array(&chainer[2], 2)?;
        let fq_user = array(&group[1], 2)?;
        let signer = array(&group[2], 2)?;
        if signer[1] != Value::Null {
            return Err(type_error("empty user-chain key owner", &signer[1]));
        }
        Ok(UserGroupChange {
            seqno: unsigned(&chainer[0])?,
            previous: option(&chainer[1], |value| fixed_blob(value, "previous link hash"))?,
            root: TreeRoot {
                epoch: unsigned(&root[0])?,
                hash: fixed_blob(&root[1], "Merkle root hash")?,
            },
            time: unsigned(&chainer[3])?,
            next_location_commitment: fixed_blob(&hiding[1], "next tree location commitment")?,
            uid: entity(&fq_user[0])?.require_type(ENTITY_USER)?,
            host: entity(&fq_user[1])?.require_type(ENTITY_HOST)?,
            signer: device_entity(&signer[0])?,
            changes: list(&group[3], user_member_change)?,
            shared_keys: list(&group[5], user_shared_key)?,
            metadata: list(&group[6], change_metadata)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserGroupChange {
    pub seqno: u64,
    pub previous: Option<[u8; 32]>,
    pub root: TreeRoot,
    pub time: u64,
    pub next_location_commitment: [u8; 32],
    pub uid: EntityId,
    pub host: EntityId,
    pub signer: EntityId,
    pub changes: Vec<UserMemberChange>,
    pub shared_keys: Vec<UserSharedKey>,
    pub metadata: Vec<ChangeMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChangeMetadata {
    DeviceName([u8; 32]),
    Username([u8; 32]),
    Eldest {
        subchain_location_commitment: [u8; 32],
    },
    TeamName([u8; 32]),
    TeamIndexRange(RationalRange),
    MemberLoadFloor(Role),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RationalRange {
    pub low: Rational,
    pub high: Rational,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rational {
    pub infinity: bool,
    pub base: Vec<u8>,
    pub exponent: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserMemberChange {
    pub role: Role,
    pub entity: EntityId,
    pub scoped_host: Option<EntityId>,
    pub source_role: Role,
    pub keys: UserMemberKeys,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserMemberKeys {
    None,
    User {
        hepk_fingerprint: [u8; 32],
        subkey: Option<EntityId>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSharedKey {
    pub generation: u64,
    pub role: Role,
    pub verify_key: EntityId,
    pub hepk_fingerprint: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserEldest {
    pub seqno: u64,
    pub previous: Option<[u8; 32]>,
    pub root: TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub uid: EntityId,
    pub host: EntityId,
    pub signer: EntityId,
    pub member: EntityId,
    pub member_verify_key: EntityId,
    pub member_role: Role,
    pub member_source_role: Role,
    pub member_scoped_host: Option<EntityId>,
    pub member_hepk_fingerprint: [u8; 32],
    pub member_subkey: Option<EntityId>,
    pub puk_generation: u64,
    pub puk_verify_key: EntityId,
    pub puk_hepk_fingerprint: [u8; 32],
    pub metadata: Vec<ChangeMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hepk {
    classical: DhPublicKey,
    mlkem768: Vec<u8>,
    exact: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DhPublicKey {
    Curve25519([u8; 32]),
    P256([u8; 33]),
}

impl Hepk {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        hepk(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(self.exact.clone())
    }

    pub fn classical(&self) -> &DhPublicKey {
        &self.classical
    }

    pub fn curve25519(&self) -> Option<&[u8; 32]> {
        match &self.classical {
            DhPublicKey::Curve25519(key) => Some(key),
            DhPublicKey::P256(_) => None,
        }
    }

    pub fn p256(&self) -> Option<&[u8; 33]> {
        match &self.classical {
            DhPublicKey::Curve25519(_) => None,
            DhPublicKey::P256(key) => Some(key),
        }
    }

    pub fn mlkem768(&self) -> &[u8] {
        &self.mlkem768
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MerkleTerminal {
    Leaf {
        leaf: [u8; 32],
        found_key: Option<[u8; 32]>,
    },
    PrefixMiss {
        prefix_bit_start: u64,
        prefix_bit_count: u64,
        prefix: Vec<u8>,
        left: [u8; 32],
        right: [u8; 32],
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerklePathCompressed {
    pub edges: Vec<[u8; 33]>,
    pub terminal: MerkleTerminal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserMerklePaths {
    root: MerkleRoot,
    paths: Vec<MerklePathCompressed>,
    exact_root: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameCommitmentAndKey {
    pub name: Vec<u8>,
    pub sequence: u64,
    pub commitment_key: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceLabel {
    pub device_type: u64,
    pub normalized_name: Vec<u8>,
    pub serial: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceLabelNameAndCommitmentKey {
    pub label: DeviceLabel,
    pub normalization_version: u64,
    pub display_name: Vec<u8>,
    pub commitment_key: [u8; 16],
}

fn name_commitment_and_key(value: &Value) -> Result<NameCommitmentAndKey> {
    let fields = array(value, 2)?;
    let commitment = array(&fields[0], 2)?;
    Ok(NameCommitmentAndKey {
        name: text(&commitment[0])?.into_bytes(),
        sequence: unsigned(&commitment[1])?,
        commitment_key: fixed_blob(&fields[1], "username commitment key")?,
    })
}

fn device_label_name_and_commitment_key(value: &Value) -> Result<DeviceLabelNameAndCommitmentKey> {
    let fields = array(value, 2)?;
    let disclosed = array(&fields[0], 3)?;
    let label = array(&disclosed[0], 3)?;
    Ok(DeviceLabelNameAndCommitmentKey {
        label: DeviceLabel {
            device_type: unsigned(&label[0])?,
            normalized_name: text(&label[1])?.into_bytes(),
            serial: unsigned(&label[2])?,
        },
        normalization_version: unsigned(&disclosed[1])?,
        display_name: text(&disclosed[2])?.into_bytes(),
        commitment_key: fixed_blob(&fields[1], "device-name commitment key")?,
    })
}

impl UserMerklePaths {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        user_merkle_paths(&decode(bytes)?)
    }

    pub fn encoded_root(&self) -> Result<Vec<u8>> {
        Ok(self.exact_root.clone())
    }

    pub fn root(&self) -> &MerkleRoot {
        &self.root
    }

    pub fn paths(&self) -> &[MerklePathCompressed] {
        &self.paths
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserChain {
    pub links: Vec<UserLink>,
    pub locations: Vec<[u8; 32]>,
    pub usernames: Vec<NameCommitmentAndKey>,
    pub merkle: UserMerklePaths,
    pub device_names: Vec<DeviceLabelNameAndCommitmentKey>,
    pub username_utf8: Vec<u8>,
    pub num_username_links: u64,
    pub hepks: Vec<Hepk>,
    pub exact_bytes: Vec<u8>,
}

impl UserChain {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 8)?;
        let links = list(&fields[0], user_link)?;
        let locations = list(&fields[1], |value| fixed_blob(value, "tree location"))?;
        let usernames = list(&fields[2], name_commitment_and_key)?;
        let merkle = user_merkle_paths(&fields[3])?;
        let device_names = list(&fields[4], device_label_name_and_commitment_key)?;
        let username_utf8 = text(&fields[5])?.into_bytes();
        let num_username_links = unsigned(&fields[6])?;
        let hepk_set = array(&fields[7], 1)?;
        let hepks = array_any(&hepk_set[0])?
            .iter()
            .map(hepk)
            .collect::<Result<Vec<_>>>()?;
        let username_path_count =
            usize::try_from(num_username_links).map_err(|_| Error::FieldCount {
                expected: usize::MAX,
                found: merkle.paths.len(),
            })?;
        let expected_paths = links
            .len()
            .checked_add(username_path_count)
            .and_then(|count| count.checked_add(1))
            .ok_or(Error::FieldCount {
                expected: usize::MAX,
                found: merkle.paths.len(),
            })?;
        if links.len() != locations.len() || merkle.paths.len() != expected_paths {
            return Err(Error::FieldCount {
                expected: expected_paths,
                found: merkle.paths.len(),
            });
        }
        Ok(Self {
            links,
            locations,
            usernames,
            merkle,
            device_names,
            username_utf8,
            num_username_links,
            hepks,
            exact_bytes: bytes.to_vec(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HybridBox {
    pub kem_ciphertext: Vec<u8>,
    pub dh_type: u64,
    pub sender_dh: Option<DhPublicKey>,
    pub nonce: [u8; 16],
    pub ciphertext: Vec<u8>,
}

impl HybridBox {
    /// Decodes the outer `Box` wrapper used for Yubi subkey self-boxes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        hybrid_box(&decode(bytes)?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TempDhKeySigned {
    pub key: DhPublicKey,
    pub time: u64,
    pub signature: Signature,
}

impl TempDhKeySigned {
    pub fn signing_bytes(
        &self,
        box_id: &[u8; 16],
        signer: &EntityId,
        host: &EntityId,
    ) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Binary(box_id.to_vec()),
            Value::Array(vec![dh_public_value(&self.key), Value::Unsigned(self.time)]),
            Value::Array(vec![
                Value::Binary(signer.as_bytes().to_vec()),
                Value::Binary(host.as_bytes().to_vec()),
            ]),
        ]))?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PukParcel {
    pub generation: u64,
    pub role: Role,
    pub hybrid: HybridBox,
    pub target: EntityId,
    pub sender: EntityId,
    pub box_id: [u8; 16],
    pub temp_dh_key: Option<TempDhKeySigned>,
    pub seed_chain_len: usize,
}

impl PukParcel {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 5)?;
        let shared_box = array(&fields[0], 4)?;
        let role = role(&shared_box[1])?;
        let target = array(&shared_box[3], 4)?;
        Ok(Self {
            generation: unsigned(&shared_box[0])?,
            role,
            hybrid: hybrid_box(&shared_box[2])?,
            target: entity(&target[0])?,
            sender: entity(&fields[1])?,
            box_id: fixed_blob(&fields[2], "box set ID")?,
            temp_dh_key: option(&fields[3], temp_dh_key_signed)?,
            seed_chain_len: list_values(&fields[4])?.len(),
        })
    }
}

fn hybrid_box(value: &Value) -> Result<HybridBox> {
    let boxed = array(value, 2)?;
    expect_unsigned(&boxed[0], "box type", 2)?;
    let hybrid = array(variant(&boxed[1], "2")?, 2)?;
    expect_unsigned(&hybrid[0], "hybrid box version", 1)?;
    let v1 = array(variant(&hybrid[1], "1")?, 4)?;
    let secret_box = array(&v1[3], 2)?;
    expect_unsigned(&secret_box[0], "secret box type", 0)?;
    let nacl = array(variant(&secret_box[1], "0")?, 2)?;
    Ok(HybridBox {
        kem_ciphertext: binary(&v1[0])?.to_vec(),
        dh_type: unsigned(&v1[1])?,
        sender_dh: option(&v1[2], dh_public)?,
        nonce: fixed_blob(&nacl[0], "NaCl nonce")?,
        ciphertext: binary(&nacl[1])?.to_vec(),
    })
}

fn temp_dh_key_signed(value: &Value) -> Result<TempDhKeySigned> {
    let fields = array(value, 2)?;
    let key = array(&fields[0], 2)?;
    Ok(TempDhKeySigned {
        key: dh_public(&key[0])?,
        time: unsigned(&key[1])?,
        signature: signature(&fields[1])?,
    })
}

fn dh_public_value(key: &DhPublicKey) -> Value {
    match key {
        DhPublicKey::Curve25519(bytes) => Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"0".to_vec(),
                Box::new(Value::Binary(bytes.to_vec())),
            ))),
        ]),
        DhPublicKey::P256(bytes) => Value::Array(vec![
            Value::Unsigned(2),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Binary(bytes.to_vec())),
            ))),
        ]),
    }
}

/// A 32-byte secret that is zeroized on drop and redacted from diagnostics.
pub struct SecretSeed(Zeroizing<[u8; 32]>);

impl SecretSeed {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    fn from_slice(bytes: &[u8]) -> Result<Self> {
        let mut seed = Zeroizing::new([0u8; 32]);
        if bytes.len() != seed.len() {
            return Err(Error::Length {
                kind: "shared key seed",
                expected: seed.len(),
                found: bytes.len(),
            });
        }
        seed.copy_from_slice(bytes);
        Ok(Self(seed))
    }
}

impl std::fmt::Debug for SecretSeed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretSeed([REDACTED])")
    }
}

impl PartialEq for SecretSeed {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for SecretSeed {}

#[derive(Debug, Eq, PartialEq)]
pub struct SharedKeySeed {
    pub receiver: EntityId,
    pub host: EntityId,
    pub generation: u64,
    pub role: Role,
    pub seed: SecretSeed,
}

#[derive(Debug, Eq, PartialEq)]
pub struct SubkeySeed {
    pub parent: EntityId,
    pub subkey: EntityId,
    pub seed: SecretSeed,
}

impl SubkeySeed {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 3)?;
        Ok(Self {
            parent: entity(&fields[0])?,
            subkey: entity(&fields[1])?.require_type(ENTITY_SUBKEY)?,
            seed: SecretSeed::from_slice(binary(&fields[2])?)?,
        })
    }

    pub fn into_seed(self) -> SecretSeed {
        self.seed
    }
}

impl SharedKeySeed {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 4)?;
        let fqe = array(&fields[0], 2)?;
        Ok(Self {
            receiver: entity(&fqe[0])?,
            host: entity(&fqe[1])?.require_type(ENTITY_HOST)?,
            generation: unsigned(&fields[1])?,
            role: role(&fields[2])?,
            seed: SecretSeed::from_slice(binary(&fields[3])?)?,
        })
    }

    pub fn into_seed(self) -> SecretSeed {
        self.seed
    }
}

fn user_link(value: &Value) -> Result<UserLink> {
    let fields = array(value, 2)?;
    expect_unsigned(&fields[0], "link outer version", 1)?;
    let v1 = array(variant(&fields[1], "1")?, 2)?;
    Ok(UserLink {
        inner: binary(&v1[0])?.to_vec(),
        signatures: list(&v1[1], signature)?,
        exact: encode(value)?,
    })
}

fn role(value: &Value) -> Result<Role> {
    let fields = array(value, 2)?;
    let role_type = unsigned(&fields[0])?;
    match role_type {
        0 | 2 | 3 => {
            if fields[1] != Value::Variant(None) {
                return Err(type_error("empty role variant", &fields[1]));
            }
            Ok(match role_type {
                0 => Role::NONE,
                2 => Role::ADMIN,
                3 => Role::OWNER,
                _ => unreachable!(),
            })
        }
        1 => {
            let visibility = integer(variant(&fields[1], "0")?)?;
            let visibility = i16::try_from(visibility).map_err(|_| Error::IntegerRange("i16"))?;
            Ok(Role::member(visibility))
        }
        value => Err(Error::UnknownEnum {
            kind: "role type",
            value,
        }),
    }
}

fn change_metadata(value: &Value) -> Result<ChangeMetadata> {
    let fields = array(value, 2)?;
    let change_type = unsigned(&fields[0])?;
    match change_type {
        0 => Ok(ChangeMetadata::DeviceName(fixed_blob(
            variant(&fields[1], "0")?,
            "device-name commitment",
        )?)),
        1 => Ok(ChangeMetadata::Username(fixed_blob(
            variant(&fields[1], "0")?,
            "username commitment",
        )?)),
        2 => {
            let eldest = array(variant(&fields[1], "1")?, 1)?;
            Ok(ChangeMetadata::Eldest {
                subchain_location_commitment: fixed_blob(
                    &eldest[0],
                    "subchain location commitment",
                )?,
            })
        }
        3 => Ok(ChangeMetadata::TeamName(fixed_blob(
            variant(&fields[1], "0")?,
            "team-name commitment",
        )?)),
        4 => Ok(ChangeMetadata::TeamIndexRange(rational_range(variant(
            &fields[1], "2",
        )?)?)),
        5 => Ok(ChangeMetadata::MemberLoadFloor(role(variant(
            &fields[1], "3",
        )?)?)),
        value => Err(Error::UnknownEnum {
            kind: "change metadata type",
            value,
        }),
    }
}

fn rational_range(value: &Value) -> Result<RationalRange> {
    let fields = array(value, 2)?;
    Ok(RationalRange {
        low: rational(&fields[0])?,
        high: rational(&fields[1])?,
    })
}

fn rational(value: &Value) -> Result<Rational> {
    let fields = array(value, 3)?;
    let Value::Bool(infinity) = fields[0] else {
        return Err(type_error("boolean", &fields[0]));
    };
    Ok(Rational {
        infinity,
        base: binary(&fields[1])?.to_vec(),
        exponent: integer(&fields[2])?,
    })
}

fn hepk(value: &Value) -> Result<Hepk> {
    let outer = array(value, 2)?;
    expect_unsigned(&outer[0], "HEPK version", 1)?;
    let v1 = array(variant(&outer[1], "1")?, 2)?;
    let classical = dh_public(&v1[0])?;
    let kem = array(&v1[1], 2)?;
    expect_unsigned(&kem[0], "KEM type", 1)?;
    let mlkem768 = binary(variant(&kem[1], "1")?)?.to_vec();
    if mlkem768.len() != 1184 {
        return Err(Error::Length {
            kind: "ML-KEM-768 encapsulation key",
            expected: 1184,
            found: mlkem768.len(),
        });
    }
    Ok(Hepk {
        classical,
        mlkem768,
        exact: encode(value)?,
    })
}

fn dh_public(value: &Value) -> Result<DhPublicKey> {
    let fields = array(value, 2)?;
    match unsigned(&fields[0])? {
        1 => Ok(DhPublicKey::Curve25519(fixed_blob(
            variant(&fields[1], "0")?,
            "Curve25519 public key",
        )?)),
        2 => Ok(DhPublicKey::P256(fixed_blob(
            variant(&fields[1], "1")?,
            "P-256 compressed public key",
        )?)),
        value => Err(Error::UnknownEnum {
            kind: "DH type",
            value,
        }),
    }
}

fn user_merkle_paths(value: &Value) -> Result<UserMerklePaths> {
    let fields = array(value, 2)?;
    let exact_root = encode(&fields[0])?;
    let root = MerkleRoot::decode(&exact_root)?;
    let paths = list(&fields[1], merkle_path)?;
    Ok(UserMerklePaths {
        root,
        paths,
        exact_root,
    })
}

fn merkle_path(value: &Value) -> Result<MerklePathCompressed> {
    let fields = array(value, 2)?;
    let path = binary(&fields[0])?;
    if path.len() % 33 != 0 {
        return Err(Error::Length {
            kind: "compressed Merkle path",
            expected: path.len() / 33 * 33,
            found: path.len(),
        });
    }
    let edges = path
        .chunks_exact(33)
        .map(|edge| {
            edge.try_into().map_err(|_| Error::Length {
                kind: "compressed Merkle edge",
                expected: 33,
                found: edge.len(),
            })
        })
        .collect::<Result<Vec<[u8; 33]>>>()?;
    let terminal = array(&fields[1], 2)?;
    let Value::Bool(present) = terminal[0] else {
        return Err(type_error("boolean", &terminal[0]));
    };
    let payload = variant(&terminal[1], if present { "1" } else { "0" })?;
    let terminal = if present {
        let payload = array(payload, 2)?;
        MerkleTerminal::Leaf {
            leaf: fixed_blob(&payload[0], "Merkle leaf value")?,
            found_key: option(&payload[1], |value| fixed_blob(value, "found Merkle key"))?,
        }
    } else {
        let wrapper = array(payload, 1)?;
        let node = array(&wrapper[0], 5)?;
        MerkleTerminal::PrefixMiss {
            prefix_bit_start: unsigned(&node[0])?,
            prefix_bit_count: unsigned(&node[1])?,
            prefix: binary(&node[2])?.to_vec(),
            left: fixed_blob(&node[3], "left Merkle child")?,
            right: fixed_blob(&node[4], "right Merkle child")?,
        }
    };
    Ok(MerklePathCompressed { edges, terminal })
}

fn user_member_change(value: &Value) -> Result<UserMemberChange> {
    let fields = array(value, 2)?;
    let member = array(&fields[1], 3)?;
    let scoped = array(&member[0], 2)?;
    let key_fields = array(&member[2], 2)?;
    let key_type = unsigned(&key_fields[0])?;
    let keys = match key_type {
        0 => {
            if key_fields[1] != Value::Variant(None) {
                return Err(type_error("empty member-key variant", &key_fields[1]));
            }
            UserMemberKeys::None
        }
        1 => {
            let user = array(variant(&key_fields[1], "1")?, 2)?;
            UserMemberKeys::User {
                hepk_fingerprint: fixed_blob(&user[0], "device HEPK fingerprint")?,
                subkey: option(&user[1], entity)?,
            }
        }
        value => {
            return Err(Error::UnknownEnum {
                kind: "user member keys",
                value,
            })
        }
    };
    Ok(UserMemberChange {
        role: role(&fields[0])?,
        entity: device_entity(&scoped[0])?,
        scoped_host: option(&scoped[1], |value| entity(value)?.require_type(ENTITY_HOST))?,
        source_role: role(&member[1])?,
        keys,
    })
}

fn user_shared_key(value: &Value) -> Result<UserSharedKey> {
    let fields = array(value, 4)?;
    Ok(UserSharedKey {
        generation: unsigned(&fields[0])?,
        role: role(&fields[1])?,
        verify_key: entity(&fields[2])?.require_type(ENTITY_PUK_VERIFY)?,
        hepk_fingerprint: fixed_blob(&fields[3], "PUK HEPK fingerprint")?,
    })
}

fn array_any(value: &Value) -> Result<&[Value]> {
    match value {
        Value::Null => Ok(&[]),
        Value::Array(values) => Ok(values),
        _ => Err(type_error("array or null", value)),
    }
}

fn list_values(value: &Value) -> Result<Vec<&Value>> {
    Ok(array_any(value)?.iter().collect())
}

fn signed_blob(value: &Value) -> Result<SignedBlob> {
    let fields = array(value, 2)?;
    Ok(SignedBlob {
        inner: binary(&fields[0])?.to_vec(),
        signature: signature(&fields[1])?,
    })
}

fn signature(value: &Value) -> Result<Signature> {
    let fields = array(value, 2)?;
    let signature_type = unsigned(&fields[0])?;
    match signature_type {
        0 => Ok(Signature::Ed25519(fixed_blob(
            variant(&fields[1], "0")?,
            "Ed25519 signature",
        )?)),
        1 => Ok(Signature::Ecdsa(
            binary(variant(&fields[1], "1")?)?.to_vec(),
        )),
        value => Err(Error::UnknownEnum {
            kind: "signature type",
            value,
        }),
    }
}

fn hostchain_link(value: &Value) -> Result<HostchainLink> {
    let fields = array(value, 2)?;
    expect_unsigned(&fields[0], "hostchain link version", 1)?;
    let v1 = array(variant(&fields[1], "1")?, 2)?;
    Ok(HostchainLink {
        inner: binary(&v1[0])?.to_vec(),
        signatures: list(&v1[1], signature)?,
    })
}

fn signature_list(signatures: &[Signature]) -> Value {
    if signatures.is_empty() {
        Value::Null
    } else {
        Value::Array(signatures.iter().map(Signature::to_value).collect())
    }
}

fn hostchain_change(value: &Value) -> Result<HostchainChange> {
    let fields = array(value, 4)?;
    let chainer = array(&fields[0], 4)?;
    let root = array(&chainer[2], 2)?;
    Ok(HostchainChange {
        chainer: BaseChainer {
            seqno: unsigned(&chainer[0])?,
            previous: option(&chainer[1], |value| fixed_blob(value, "previous link hash"))?,
            root: TreeRoot {
                epoch: unsigned(&root[0])?,
                hash: fixed_blob(&root[1], "Merkle root hash")?,
            },
            time: unsigned(&chainer[3])?,
        },
        host: entity(&fields[1])?.require_type(ENTITY_HOST)?,
        signer: entity(&fields[2])?.require_type(ENTITY_HOST)?,
        changes: list(&fields[3], change_item)?,
    })
}

fn change_item(value: &Value) -> Result<HostchainChangeItem> {
    let fields = array(value, 2)?;
    let change_type = unsigned(&fields[0])?;
    match change_type {
        1 => Ok(HostchainChangeItem::Revoke(entity(variant(
            &fields[1], "1",
        )?)?)),
        2 => Ok(HostchainChangeItem::Key(entity(variant(&fields[1], "2")?)?)),
        3 => {
            let ca = array(variant(&fields[1], "3")?, 2)?;
            Ok(HostchainChangeItem::TlsCa {
                id: entity(&ca[0])?.require_type(ENTITY_HOST_TLS_CA)?,
                certificate: binary(&ca[1])?.to_vec(),
            })
        }
        value => Err(Error::UnknownEnum {
            kind: "hostchain change type",
            value,
        }),
    }
}

fn array(value: &Value, expected: usize) -> Result<&[Value]> {
    let Value::Array(values) = value else {
        return Err(type_error("array", value));
    };
    if values.len() != expected {
        return Err(Error::FieldCount {
            expected,
            found: values.len(),
        });
    }
    Ok(values)
}

fn list<T>(value: &Value, parser: fn(&Value) -> Result<T>) -> Result<Vec<T>> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(values) => values.iter().map(parser).collect(),
        _ => Err(type_error("list or null", value)),
    }
}

fn option<T>(value: &Value, parser: impl FnOnce(&Value) -> Result<T>) -> Result<Option<T>> {
    if value == &Value::Null {
        Ok(None)
    } else {
        parser(value).map(Some)
    }
}

fn variant<'a>(value: &'a Value, expected: &str) -> Result<&'a Value> {
    let Value::Variant(Some((tag, payload))) = value else {
        return Err(type_error("one-case variant", value));
    };
    if tag != expected.as_bytes() {
        return Err(Error::VariantTag {
            expected: expected.to_owned(),
            found: tag.clone(),
        });
    }
    Ok(payload)
}

fn unsigned(value: &Value) -> Result<u64> {
    match value {
        Value::Unsigned(value) => Ok(*value),
        _ => Err(type_error("unsigned integer", value)),
    }
}

fn integer(value: &Value) -> Result<i64> {
    match value {
        Value::Unsigned(value) => i64::try_from(*value).map_err(|_| Error::IntegerRange("i64")),
        Value::Negative(value) => Ok(*value),
        _ => Err(type_error("integer", value)),
    }
}

fn expect_unsigned(value: &Value, kind: &'static str, expected: u64) -> Result<()> {
    let found = unsigned(value)?;
    if found != expected {
        return Err(Error::UnknownEnum { kind, value: found });
    }
    Ok(())
}

fn binary(value: &Value) -> Result<&[u8]> {
    match value {
        Value::Binary(bytes) => Ok(bytes),
        _ => Err(type_error("binary", value)),
    }
}

fn fixed_blob<const N: usize>(value: &Value, kind: &'static str) -> Result<[u8; N]> {
    let bytes = binary(value)?;
    bytes.try_into().map_err(|_| Error::Length {
        kind,
        expected: N,
        found: bytes.len(),
    })
}

fn entity(value: &Value) -> Result<EntityId> {
    EntityId::from_bytes(binary(value)?.to_vec())
}

fn device_entity(value: &Value) -> Result<EntityId> {
    let entity = entity(value)?;
    match entity.entity_type() {
        ENTITY_DEVICE | ENTITY_YUBI => Ok(entity),
        found => Err(Error::WrongEntityType {
            expected: ENTITY_DEVICE,
            found,
        }),
    }
}

fn text(value: &Value) -> Result<String> {
    let Value::Text(bytes) = value else {
        return Err(type_error("text", value));
    };
    String::from_utf8(bytes.clone()).map_err(|_| Error::Utf8)
}

fn type_error(expected: &'static str, value: &Value) -> Error {
    Error::Type {
        expected,
        found: value_name(value),
    }
}

fn value_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Unsigned(_) => "unsigned integer",
        Value::Negative(_) => "negative integer",
        Value::Binary(_) => "binary",
        Value::Text(_) => "text",
        Value::Array(_) => "array",
        Value::Variant(_) => "variant",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );

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
}
