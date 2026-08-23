//! Authenticated key-distribution boxes and opaque secret payloads.

use crate::{
    array, binary, decode, dh_public, encode, entity, expect_unsigned, fixed_blob, list,
    list_values, option, role, secret_box, signature, unsigned, variant, DhPublicKey, EntityId,
    Error, Result, Role, Signature, Value, ENTITY_DEVICE, ENTITY_HOST, ENTITY_SUBKEY,
};
use zeroize::Zeroizing;

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

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    fn to_value(&self) -> Value {
        let sender = self.sender_dh.as_ref().map_or(Value::Null, dh_public_value);
        Value::Array(vec![
            Value::Unsigned(2),
            Value::Variant(Some((
                b"2".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Unsigned(1),
                    Value::Variant(Some((
                        b"1".to_vec(),
                        Box::new(Value::Array(vec![
                            Value::Binary(self.kem_ciphertext.clone()),
                            Value::Unsigned(self.dh_type),
                            sender,
                            Value::Array(vec![
                                Value::Unsigned(0),
                                Value::Variant(Some((
                                    b"0".to_vec(),
                                    Box::new(Value::Array(vec![
                                        Value::Binary(self.nonce.to_vec()),
                                        Value::Binary(self.ciphertext.clone()),
                                    ])),
                                ))),
                            ]),
                        ])),
                    ))),
                ])),
            ))),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedKeyBoxTarget {
    pub entity: EntityId,
    pub host: Option<EntityId>,
    pub role: Role,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedKeyBox {
    pub generation: u64,
    pub role: Role,
    pub hybrid: HybridBox,
    pub target: SharedKeyBoxTarget,
}

/// Exact PUK/PTK box set used by signup and subsequent key mutations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedKeyBoxSet {
    pub box_id: [u8; 16],
    pub boxes: Vec<SharedKeyBox>,
    pub temp_dh_key: Option<TempDhKeySigned>,
    exact: Vec<u8>,
}

impl SharedKeyBoxSet {
    pub fn software_initial(box_id: [u8; 16], target: EntityId, hybrid: HybridBox) -> Result<Self> {
        target.clone().require_type(ENTITY_DEVICE)?;
        Self::new(
            box_id,
            vec![SharedKeyBox {
                generation: 1,
                role: Role::OWNER,
                hybrid,
                target: SharedKeyBoxTarget {
                    entity: target,
                    host: None,
                    role: Role::NONE,
                    generation: 0,
                },
            }],
            None,
        )
    }

    pub fn new(
        box_id: [u8; 16],
        boxes: Vec<SharedKeyBox>,
        temp_dh_key: Option<TempDhKeySigned>,
    ) -> Result<Self> {
        if boxes.is_empty() {
            return Err(Error::FieldCount {
                expected: 1,
                found: 0,
            });
        }
        let value = Value::Array(vec![
            Value::Binary(box_id.to_vec()),
            Value::Array(boxes.iter().map(shared_key_box_value).collect()),
            temp_dh_key.as_ref().map_or(Value::Null, |temporary| {
                Value::Array(vec![
                    Value::Array(vec![
                        dh_public_value(&temporary.key),
                        Value::Unsigned(temporary.time),
                    ]),
                    temporary.signature.to_value(),
                ])
            }),
        ]);
        Ok(Self {
            box_id,
            boxes,
            temp_dh_key,
            exact: encode(&value)?,
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        let boxes = list_values(&fields[1])?;
        if boxes.is_empty() {
            return Err(Error::FieldCount {
                expected: 1,
                found: 0,
            });
        }
        Ok(Self {
            box_id: fixed_blob(&fields[0], "box set ID")?,
            boxes: boxes
                .into_iter()
                .map(shared_key_box)
                .collect::<Result<Vec<_>>>()?,
            temp_dh_key: option(&fields[2], temp_dh_key_signed)?,
            exact: bytes.to_vec(),
        })
    }

    pub fn encoded(&self) -> Vec<u8> {
        self.exact.clone()
    }
}

fn shared_key_box(value: &Value) -> Result<SharedKeyBox> {
    let fields = array(value, 4)?;
    let target = array(&fields[3], 4)?;
    Ok(SharedKeyBox {
        generation: unsigned(&fields[0])?,
        role: role(&fields[1])?,
        hybrid: hybrid_box(&fields[2])?,
        target: SharedKeyBoxTarget {
            entity: entity(&target[0])?,
            host: option(&target[1], |value| entity(value)?.require_type(ENTITY_HOST))?,
            role: role(&target[2])?,
            generation: unsigned(&target[3])?,
        },
    })
}

fn shared_key_box_value(shared: &SharedKeyBox) -> Value {
    Value::Array(vec![
        Value::Unsigned(shared.generation),
        shared.role.to_value(),
        shared.hybrid.to_value(),
        Value::Array(vec![
            Value::Binary(shared.target.entity.as_bytes().to_vec()),
            shared
                .target
                .host
                .as_ref()
                .map_or(Value::Null, |host| Value::Binary(host.as_bytes().to_vec())),
            shared.target.role.to_value(),
            Value::Unsigned(shared.target.generation),
        ]),
    ])
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
    pub seed_chain: Vec<SeedChainBox>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeedChainBox {
    pub generation: u64,
    pub role: Role,
    pub secret_box: SecretBox,
}

impl PukParcel {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        puk_parcel(&wire)
    }
}

pub(crate) fn puk_parcel(value: &Value) -> Result<PukParcel> {
    let fields = array(value, 5)?;
    let shared_box = array(&fields[0], 4)?;
    let role = role(&shared_box[1])?;
    let target = array(&shared_box[3], 4)?;
    Ok(PukParcel {
        generation: unsigned(&shared_box[0])?,
        role,
        hybrid: hybrid_box(&shared_box[2])?,
        target: entity(&target[0])?,
        sender: entity(&fields[1])?,
        box_id: fixed_blob(&fields[2], "box set ID")?,
        temp_dh_key: option(&fields[3], temp_dh_key_signed)?,
        seed_chain: list(&fields[4], seed_chain_box)?,
    })
}

fn seed_chain_box(value: &Value) -> Result<SeedChainBox> {
    let fields = array(value, 3)?;
    Ok(SeedChainBox {
        generation: unsigned(&fields[0])?,
        role: role(&fields[1])?,
        secret_box: secret_box(&fields[2])?,
    })
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoleAndGeneration {
    pub role: Role,
    pub generation: u64,
}

impl RoleAndGeneration {
    pub fn to_value(self) -> Value {
        Value::Array(vec![self.role.to_value(), Value::Unsigned(self.generation)])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretBox {
    pub nonce: [u8; 16],
    pub ciphertext: Vec<u8>,
}

impl SecretBox {
    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Unsigned(0),
            Value::Variant(Some((
                b"0".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Binary(self.nonce.to_vec()),
                    Value::Binary(self.ciphertext.clone()),
                ])),
            ))),
        ])
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
