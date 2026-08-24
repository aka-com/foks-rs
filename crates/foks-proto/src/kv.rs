//! KV namespace, content, version-vector, and ciphertext wire objects.

use crate::codec::value_name;
use crate::{
    array, binary, boolean, decode, encode, expect_unsigned, fixed_blob, list, option, role,
    unsigned, variant, EntityId, Error, Result, Role, RoleAndGeneration, SecretBox, Value,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum KvNodeType {
    None = 0,
    Directory = 1,
    File = 2,
    SmallFile = 3,
    Symlink = 4,
}

impl TryFrom<u8> for KvNodeType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Directory),
            2 => Ok(Self::File),
            3 => Ok(Self::SmallFile),
            4 => Ok(Self::Symlink),
            value => Err(Error::UnknownEnum {
                kind: "KV node type",
                value: u64::from(value),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KvNodeId(pub [u8; 17]);

impl KvNodeId {
    pub fn node_type(self) -> Result<KvNodeType> {
        self.0[0].try_into()
    }

    pub fn object_id(self) -> [u8; 16] {
        self.0[1..].try_into().expect("KV node IDs are fixed width")
    }

    pub fn to_value(self) -> Value {
        Value::Binary(self.0.to_vec())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvParty {
    pub party: EntityId,
    pub host: EntityId,
}

impl KvParty {
    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.party.as_bytes().to_vec()),
            Value::Binary(self.host.as_bytes().to_vec()),
        ])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KvDirentVersion {
    pub id: [u8; 16],
    pub version: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvDirectoryVersion {
    pub id: [u8; 16],
    pub version: u64,
    pub entries: Vec<KvDirentVersion>,
}

/// Versions of cached KV objects used by one operation. FOKS servers return
/// this same type in `KV_STALE_CACHE_ERROR`, containing only objects whose
/// current versions are newer than the submitted values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvPathVersionVector {
    pub root_version: u64,
    pub directories: Vec<KvDirectoryVersion>,
}

impl KvPathVersionVector {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_value(&decode(bytes)?)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 2)?;
        Ok(Self {
            root_version: unsigned(&fields[0])?,
            directories: list(&fields[1], kv_directory_version)?,
        })
    }

    pub fn to_value(&self) -> Value {
        let directories = self
            .directories
            .iter()
            .map(|directory| {
                let entries = directory
                    .entries
                    .iter()
                    .map(|entry| {
                        Value::Array(vec![
                            Value::Binary(entry.id.to_vec()),
                            Value::Unsigned(entry.version),
                        ])
                    })
                    .collect::<Vec<_>>();
                Value::Array(vec![
                    Value::Binary(directory.id.to_vec()),
                    Value::Unsigned(directory.version),
                    if entries.is_empty() {
                        Value::Null
                    } else {
                        Value::Array(entries)
                    },
                ])
            })
            .collect::<Vec<_>>();
        Value::Array(vec![
            Value::Unsigned(self.root_version),
            if directories.is_empty() {
                Value::Null
            } else {
                Value::Array(directories)
            },
        ])
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvRoot {
    pub root: [u8; 16],
    pub version: u64,
    pub key: RoleAndGeneration,
    pub binding_mac: [u8; 32],
    exact: Vec<u8>,
}

impl KvRoot {
    pub fn new(
        root: [u8; 16],
        version: u64,
        key: RoleAndGeneration,
        binding_mac: [u8; 32],
    ) -> Result<Self> {
        let mut value = Self {
            root,
            version,
            key,
            binding_mac,
            exact: Vec::new(),
        };
        value.exact = encode(&value.to_value())?;
        Ok(value)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 4)?;
        Ok(Self {
            root: fixed_blob(&fields[0], "KV root directory ID")?,
            version: unsigned(&fields[1])?,
            key: role_and_generation(&fields[2])?,
            binding_mac: fixed_blob(&fields[3], "KV root binding MAC")?,
            exact: bytes.to_vec(),
        })
    }

    pub fn encoded(&self) -> &[u8] {
        &self.exact
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.root.to_vec()),
            Value::Unsigned(self.version),
            self.key.to_value(),
            Value::Binary(self.binding_mac.to_vec()),
        ])
    }

    pub fn binding_payload(&self, party: &KvParty) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            party.to_value(),
            self.key.to_value(),
            Value::Binary(self.root.to_vec()),
            Value::Unsigned(self.version),
        ]))?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvDirectoryStatus {
    Active,
    Encrypting,
    Dead,
}

impl KvDirectoryStatus {
    pub fn to_value(self) -> Value {
        Value::Unsigned(match self {
            Self::Active => 0,
            Self::Encrypting => 1,
            Self::Dead => 2,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvDirectory {
    pub id: [u8; 16],
    pub version: u64,
    pub key: RoleAndGeneration,
    pub seed_ciphertext: Vec<u8>,
    pub write_role: Role,
    pub status: KvDirectoryStatus,
}

impl KvDirectory {
    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.id.to_vec()),
            Value::Unsigned(self.version),
            Value::Array(vec![
                self.key.to_value(),
                Value::Binary(self.seed_ciphertext.clone()),
            ]),
            self.write_role.to_value(),
            self.status.to_value(),
        ])
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvDirectoryPair {
    pub active: KvDirectory,
    pub encrypting: Option<KvDirectory>,
    exact: Vec<u8>,
}

impl KvDirectoryPair {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 2)?;
        Ok(Self {
            active: kv_directory(&fields[0])?,
            encrypting: option(&fields[1], kv_directory)?,
            exact: bytes.to_vec(),
        })
    }

    pub fn encoded(&self) -> &[u8] {
        &self.exact
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvDirent {
    pub parent: [u8; 16],
    pub id: [u8; 16],
    pub value: KvNodeId,
    pub version: u64,
    pub directory_version: u64,
    pub write_role: Role,
    pub name_mac: [u8; 32],
    pub name_box: SecretBox,
    pub directory_status: KvDirectoryStatus,
    pub binding_mac: [u8; 32],
    pub creation_time: u64,
    exact: Vec<u8>,
}

impl KvDirent {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        kv_dirent(&decode(bytes)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        parent: [u8; 16],
        id: [u8; 16],
        value: KvNodeId,
        version: u64,
        directory_version: u64,
        write_role: Role,
        name_mac: [u8; 32],
        name_box: SecretBox,
        directory_status: KvDirectoryStatus,
        binding_mac: [u8; 32],
        creation_time: u64,
    ) -> Result<Self> {
        value.node_type()?;
        let mut output = Self {
            parent,
            id,
            value,
            version,
            directory_version,
            write_role,
            name_mac,
            name_box,
            directory_status,
            binding_mac,
            creation_time,
            exact: Vec::new(),
        };
        output.exact = encode(&output.to_value())?;
        Ok(output)
    }

    pub fn encoded(&self) -> &[u8] {
        &self.exact
    }

    /// `kvList` omits the parent directory and relies on the request context.
    /// Reconstruct it before verifying the name and binding MAC, matching the
    /// v0.1.9 Go client. A nonzero conflicting parent is never accepted.
    pub fn bind_list_parent(&mut self, parent: [u8; 16]) -> Result<()> {
        if self.parent != [0; 16] && self.parent != parent {
            return Err(Error::IntegerRange("KV dirent parent"));
        }
        self.parent = parent;
        self.exact = encode(&self.to_value())?;
        Ok(())
    }

    pub fn binding_payload(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.parent.to_vec()),
            Value::Binary(self.id.to_vec()),
            self.value.to_value(),
            Value::Unsigned(self.version),
            Value::Unsigned(self.directory_version),
            self.write_role.to_value(),
            Value::Binary(self.name_mac.to_vec()),
            self.name_box.to_value(),
        ]))?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.parent.to_vec()),
            Value::Binary(self.id.to_vec()),
            self.value.to_value(),
            Value::Unsigned(self.version),
            Value::Unsigned(self.directory_version),
            self.write_role.to_value(),
            Value::Binary(self.name_mac.to_vec()),
            self.name_box.to_value(),
            self.directory_status.to_value(),
            Value::Binary(self.binding_mac.to_vec()),
            Value::Unsigned(self.creation_time),
        ])
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvExtendedDirent {
    pub position: u64,
    pub small_file: KvSmallFileBox,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvListResponse {
    pub entries: Vec<KvDirent>,
    pub final_page: bool,
    pub extended: Vec<KvExtendedDirent>,
    exact: Vec<u8>,
}

impl KvListResponse {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        Ok(Self {
            entries: list(&fields[0], kv_dirent)?,
            final_page: boolean(&fields[1])?,
            extended: list(&fields[2], kv_extended_dirent)?,
            exact: bytes.to_vec(),
        })
    }

    pub fn encoded(&self) -> &[u8] {
        &self.exact
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvSmallFileBox {
    pub key: RoleAndGeneration,
    pub ciphertext: Vec<u8>,
}

impl KvSmallFileBox {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        kv_small_file_box(&decode(bytes)?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            self.key.to_value(),
            Value::Binary(self.ciphertext.clone()),
        ])
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvLargeFileMetadata {
    pub key: RoleAndGeneration,
    pub key_seed: SecretBox,
    pub version: u64,
    pub custom_metadata: Option<SecretBox>,
}

impl KvLargeFileMetadata {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        kv_large_file_metadata(&decode(bytes)?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            self.key.to_value(),
            self.key_seed.to_value(),
            Value::Unsigned(self.version),
            self.custom_metadata
                .as_ref()
                .map_or(Value::Null, SecretBox::to_value),
        ])
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvUploadFinal {
    pub size: u64,
    pub chunk_sum: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvUploadChunk {
    pub ciphertext: Vec<u8>,
    pub offset: u64,
    pub final_upload: Option<KvUploadFinal>,
}

impl KvUploadChunk {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        let final_upload = option(&fields[2], |value| {
            let final_fields = array(value, 2)?;
            Ok(KvUploadFinal {
                size: unsigned(&final_fields[0])?,
                chunk_sum: fixed_blob(&final_fields[1], "KV upload chunk sum")?,
            })
        })?;
        Ok(Self {
            ciphertext: binary(&fields[0])?.to_vec(),
            offset: unsigned(&fields[1])?,
            final_upload,
        })
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.ciphertext.clone()),
            Value::Unsigned(self.offset),
            self.final_upload
                .as_ref()
                .map_or(Value::Null, |final_upload| {
                    Value::Array(vec![
                        Value::Unsigned(final_upload.size),
                        Value::Binary(final_upload.chunk_sum.to_vec()),
                    ])
                }),
        ])
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KvNode {
    Directory(KvDirectoryPair),
    File(KvLargeFileMetadata),
    SmallFile(KvSmallFileBox),
    Symlink(KvSmallFileBox),
}

impl KvNode {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 2)?;
        let tag = unsigned(&fields[0])?;
        match tag {
            2 => Ok(Self::File(kv_large_file_metadata(variant(
                &fields[1], "0",
            )?)?)),
            3 => Ok(Self::SmallFile(kv_small_file_box(variant(
                &fields[1], "2",
            )?)?)),
            4 => Ok(Self::Symlink(kv_small_file_box(variant(&fields[1], "3")?)?)),
            1 => {
                let encoded = encode(variant(&fields[1], "4")?)?;
                Ok(Self::Directory(KvDirectoryPair::decode(&encoded)?))
            }
            value => Err(Error::UnknownEnum {
                kind: "KV node response type",
                value,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvEncryptedChunk {
    pub ciphertext: Vec<u8>,
    pub offset: u64,
    pub final_chunk: bool,
}

impl KvEncryptedChunk {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        Ok(Self {
            ciphertext: binary(&fields[0])?.to_vec(),
            offset: unsigned(&fields[1])?,
            final_chunk: boolean(&fields[2])?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvDirentName {
    pub parent: [u8; 16],
    pub directory_version: u64,
    pub name: Vec<u8>,
}

impl KvDirentName {
    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.parent.to_vec()),
            Value::Unsigned(self.directory_version),
            Value::Text(self.name.clone()),
        ])
    }

    pub fn decode_value(value: Value) -> Result<Self> {
        let fields = match value {
            Value::Array(fields) => fields,
            value => {
                return Err(Error::Type {
                    expected: "array",
                    found: value_name(&value),
                })
            }
        };
        let [parent, directory_version, name]: [Value; 3] =
            fields
                .try_into()
                .map_err(|fields: Vec<Value>| Error::FieldCount {
                    expected: 3,
                    found: fields.len(),
                })?;
        let name = match name {
            Value::Text(name) => name,
            value => {
                return Err(Error::Type {
                    expected: "text",
                    found: value_name(&value),
                })
            }
        };
        Ok(Self {
            parent: fixed_blob(&parent, "KV name parent directory")?,
            directory_version: unsigned(&directory_version)?,
            name,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KvSmallFilePlaintext {
    File(Vec<u8>),
    Symlink(Vec<u8>),
}

impl KvSmallFilePlaintext {
    pub fn to_value(&self) -> Value {
        match self {
            Self::File(bytes) => Value::Array(vec![
                Value::Unsigned(3),
                Value::Variant(Some((
                    b"0".to_vec(),
                    Box::new(Value::Binary(bytes.clone())),
                ))),
            ]),
            Self::Symlink(path) => Value::Array(vec![
                Value::Unsigned(4),
                Value::Variant(Some((b"1".to_vec(), Box::new(Value::Text(path.clone()))))),
            ]),
        }
    }

    pub fn decode_value(value: Value) -> Result<Self> {
        let fields = match value {
            Value::Array(fields) => fields,
            value => {
                return Err(Error::Type {
                    expected: "array",
                    found: value_name(&value),
                })
            }
        };
        let [kind, payload]: [Value; 2] =
            fields
                .try_into()
                .map_err(|fields: Vec<Value>| Error::FieldCount {
                    expected: 2,
                    found: fields.len(),
                })?;
        let kind = unsigned(&kind)?;
        let (tag, payload) = match payload {
            Value::Variant(Some(payload)) => payload,
            value => {
                return Err(Error::Type {
                    expected: "populated variant",
                    found: value_name(&value),
                })
            }
        };
        let payload = *payload;
        match kind {
            3 if tag.as_slice() != b"0" => Err(Error::VariantTag {
                expected: "0".to_owned(),
                found: tag,
            }),
            3 => match payload {
                Value::Binary(bytes) => Ok(Self::File(bytes)),
                value => Err(Error::Type {
                    expected: "binary",
                    found: value_name(&value),
                }),
            },
            4 if tag.as_slice() != b"1" => Err(Error::VariantTag {
                expected: "1".to_owned(),
                found: tag,
            }),
            4 => match payload {
                Value::Text(path) => Ok(Self::Symlink(path)),
                value => Err(Error::Type {
                    expected: "text",
                    found: value_name(&value),
                }),
            },
            value => Err(Error::UnknownEnum {
                kind: "small-file plaintext type",
                value,
            }),
        }
    }
}

fn role_and_generation(value: &Value) -> Result<RoleAndGeneration> {
    let fields = array(value, 2)?;
    Ok(RoleAndGeneration {
        role: role(&fields[0])?,
        generation: unsigned(&fields[1])?,
    })
}

pub(crate) fn secret_box(value: &Value) -> Result<SecretBox> {
    let fields = array(value, 2)?;
    expect_unsigned(&fields[0], "secret-box type", 0)?;
    let nacl = array(variant(&fields[1], "0")?, 2)?;
    Ok(SecretBox {
        nonce: fixed_blob(&nacl[0], "secret-box nonce")?,
        ciphertext: binary(&nacl[1])?.to_vec(),
    })
}

fn kv_directory_status(value: &Value) -> Result<KvDirectoryStatus> {
    match unsigned(value)? {
        0 => Ok(KvDirectoryStatus::Active),
        1 => Ok(KvDirectoryStatus::Encrypting),
        2 => Ok(KvDirectoryStatus::Dead),
        value => Err(Error::UnknownEnum {
            kind: "KV directory status",
            value,
        }),
    }
}

fn kv_dirent_version(value: &Value) -> Result<KvDirentVersion> {
    let fields = array(value, 2)?;
    Ok(KvDirentVersion {
        id: fixed_blob(&fields[0], "KV dirent version ID")?,
        version: unsigned(&fields[1])?,
    })
}

fn kv_directory_version(value: &Value) -> Result<KvDirectoryVersion> {
    let fields = array(value, 3)?;
    Ok(KvDirectoryVersion {
        id: fixed_blob(&fields[0], "KV directory version ID")?,
        version: unsigned(&fields[1])?,
        entries: list(&fields[2], kv_dirent_version)?,
    })
}

fn kv_directory(value: &Value) -> Result<KvDirectory> {
    let fields = array(value, 5)?;
    let seed_box = array(&fields[2], 2)?;
    Ok(KvDirectory {
        id: fixed_blob(&fields[0], "KV directory ID")?,
        version: unsigned(&fields[1])?,
        key: role_and_generation(&seed_box[0])?,
        seed_ciphertext: binary(&seed_box[1])?.to_vec(),
        write_role: role(&fields[3])?,
        status: kv_directory_status(&fields[4])?,
    })
}

pub(crate) fn kv_node_id(value: &Value) -> Result<KvNodeId> {
    let bytes = fixed_blob(value, "KV node ID")?;
    let id = KvNodeId(bytes);
    id.node_type()?;
    Ok(id)
}

fn kv_dirent(value: &Value) -> Result<KvDirent> {
    let fields = array(value, 11)?;
    Ok(KvDirent {
        parent: fixed_blob(&fields[0], "KV dirent parent")?,
        id: fixed_blob(&fields[1], "KV dirent ID")?,
        value: kv_node_id(&fields[2])?,
        version: unsigned(&fields[3])?,
        directory_version: unsigned(&fields[4])?,
        write_role: role(&fields[5])?,
        name_mac: fixed_blob(&fields[6], "KV name MAC")?,
        name_box: secret_box(&fields[7])?,
        directory_status: kv_directory_status(&fields[8])?,
        binding_mac: fixed_blob(&fields[9], "KV dirent binding MAC")?,
        creation_time: unsigned(&fields[10])?,
        exact: encode(value)?,
    })
}

fn kv_small_file_box(value: &Value) -> Result<KvSmallFileBox> {
    let fields = array(value, 2)?;
    Ok(KvSmallFileBox {
        key: role_and_generation(&fields[0])?,
        ciphertext: binary(&fields[1])?.to_vec(),
    })
}

fn kv_extended_dirent(value: &Value) -> Result<KvExtendedDirent> {
    let fields = array(value, 2)?;
    Ok(KvExtendedDirent {
        position: unsigned(&fields[0])?,
        small_file: kv_small_file_box(&fields[1])?,
    })
}

fn kv_large_file_metadata(value: &Value) -> Result<KvLargeFileMetadata> {
    let fields = array(value, 4)?;
    Ok(KvLargeFileMetadata {
        key: role_and_generation(&fields[0])?,
        key_seed: secret_box(&fields[1])?,
        version: unsigned(&fields[2])?,
        custom_metadata: option(&fields[3], secret_box)?,
    })
}
