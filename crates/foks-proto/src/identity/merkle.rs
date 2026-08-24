//! Identity-chain Merkle paths and disclosed name metadata.

use crate::{
    array, binary, decode, encode, fixed_blob, list, option, text, type_error, unsigned, variant,
    Error, MerkleRoot, Result, Value,
};

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

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u64)]
pub enum DeviceType {
    Computer = 0,
    Mobile = 1,
    YubiKey = 2,
    Backup = 3,
    BotToken = 4,
}

impl DeviceType {
    pub const fn protocol_value(self) -> u64 {
        self as u64
    }
}

impl TryFrom<u64> for DeviceType {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self> {
        match value {
            0 => Ok(Self::Computer),
            1 => Ok(Self::Mobile),
            2 => Ok(Self::YubiKey),
            3 => Ok(Self::Backup),
            4 => Ok(Self::BotToken),
            _ => Err(Error::UnknownEnum {
                kind: "device type",
                value,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceLabel {
    pub device_type: DeviceType,
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

pub(crate) fn name_commitment_and_key(value: &Value) -> Result<NameCommitmentAndKey> {
    let fields = array(value, 2)?;
    let commitment = array(&fields[0], 2)?;
    Ok(NameCommitmentAndKey {
        name: text(&commitment[0])?.into_bytes(),
        sequence: unsigned(&commitment[1])?,
        commitment_key: fixed_blob(&fields[1], "username commitment key")?,
    })
}

pub(crate) fn device_label_name_and_commitment_key(
    value: &Value,
) -> Result<DeviceLabelNameAndCommitmentKey> {
    let fields = array(value, 2)?;
    let disclosed = array(&fields[0], 3)?;
    let label = array(&disclosed[0], 3)?;
    Ok(DeviceLabelNameAndCommitmentKey {
        label: DeviceLabel {
            device_type: DeviceType::try_from(unsigned(&label[0])?)?,
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

pub(crate) fn user_merkle_paths(value: &Value) -> Result<UserMerklePaths> {
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
