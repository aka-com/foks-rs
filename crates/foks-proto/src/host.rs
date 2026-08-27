//! Probe, hostchain, public-zone, and Merkle wire objects.

use crate::{
    array, binary, decode, encode, entity, expect_unsigned, fixed_blob, integer, list, option,
    text, unsigned, variant, EntityId, Error, Result, ServiceType, Signature, SignedBlob, Value,
    ENTITY_HOST, ENTITY_HOST_TLS_CA,
};

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

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            signed_blob_value(&self.merkle_root),
            signed_blob_value(&self.public_zone),
            list_value(self.hostchain.iter().map(HostchainLink::to_value).collect()),
        ]))?)
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

impl HostchainChange {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let changes = self
            .changes
            .iter()
            .map(hostchain_change_value)
            .collect::<Vec<_>>();
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Array(vec![
                        Value::Unsigned(self.chainer.seqno),
                        self.chainer
                            .previous
                            .map_or(Value::Null, |hash| Value::Binary(hash.to_vec())),
                        Value::Array(vec![
                            Value::Unsigned(self.chainer.root.epoch),
                            Value::Binary(self.chainer.root.hash.to_vec()),
                        ]),
                        Value::Unsigned(self.chainer.time),
                    ]),
                    Value::Binary(self.host.as_bytes().to_vec()),
                    Value::Binary(self.signer.as_bytes().to_vec()),
                    list_value(changes),
                ])),
            ))),
        ]))?)
    }
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
    pub fn entries(&self) -> [(ServiceType, &str); 6] {
        [
            (ServiceType::Probe, &self.probe),
            (ServiceType::Registration, &self.registration),
            (ServiceType::User, &self.user),
            (ServiceType::MerkleQuery, &self.merkle_query),
            (ServiceType::KvStore, &self.kv_store),
            (ServiceType::Realtime, &self.realtime),
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

    pub fn encoded(&self) -> Result<Vec<u8>> {
        let ttl = if self.ttl_seconds >= 0 {
            Value::Unsigned(self.ttl_seconds as u64)
        } else {
            Value::Negative(self.ttl_seconds)
        };
        Ok(encode(&Value::Array(vec![
            ttl,
            Value::Array(
                self.services
                    .entries()
                    .into_iter()
                    .map(|(_, endpoint)| Value::Text(endpoint.as_bytes().to_vec()))
                    .collect(),
            ),
        ]))?)
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

pub(crate) fn signed_blob(value: &Value) -> Result<SignedBlob> {
    let fields = array(value, 2)?;
    Ok(SignedBlob {
        inner: binary(&fields[0])?.to_vec(),
        signature: signature(&fields[1])?,
    })
}

fn signed_blob_value(blob: &SignedBlob) -> Value {
    Value::Array(vec![
        Value::Binary(blob.inner.clone()),
        blob.signature.to_value(),
    ])
}

fn list_value(values: Vec<Value>) -> Value {
    if values.is_empty() {
        Value::Null
    } else {
        Value::Array(values)
    }
}

fn hostchain_change_value(change: &HostchainChangeItem) -> Value {
    let (discriminant, tag, payload) = match change {
        HostchainChangeItem::Revoke(id) => {
            (1, b"1".to_vec(), Value::Binary(id.as_bytes().to_vec()))
        }
        HostchainChangeItem::Key(id) => (2, b"2".to_vec(), Value::Binary(id.as_bytes().to_vec())),
        HostchainChangeItem::TlsCa { id, certificate } => (
            3,
            b"3".to_vec(),
            Value::Array(vec![
                Value::Binary(id.as_bytes().to_vec()),
                Value::Binary(certificate.clone()),
            ]),
        ),
    };
    Value::Array(vec![
        Value::Unsigned(discriminant),
        Value::Variant(Some((tag, Box::new(payload)))),
    ])
}

pub(crate) fn signature(value: &Value) -> Result<Signature> {
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

pub(crate) fn signature_list(signatures: &[Signature]) -> Value {
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
