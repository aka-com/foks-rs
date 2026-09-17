//! Passphrase-encryption (PPE) values from the FOKS v0.1.9 protocol.

use crate::{
    array, encode, entity, fixed_blob, role, unsigned, DhPublicKey, EntityId, Error, Hepk,
    HybridBox, Result, Role, SecretBox, Value, ENTITY_HOST, ENTITY_PASSPHRASE_KEY, ENTITY_USER,
};
use zeroize::Zeroize;

pub const FIRST_PASSPHRASE_GENERATION: u64 = 1;
pub const SKMWK_LIST_TYPE_ID: u64 = 0x8131_91a3_c2c8_8094;
pub const PPE_PUK_BOX_PAYLOAD_TYPE_ID: u64 = 0x82a7_69eb_0726_24cd;
pub const PPE_PASSPHRASE_BOX_PAYLOAD_TYPE_ID: u64 = 0x978c_22a7_627d_777b;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StretchVersion {
    Test,
    V1,
}

impl StretchVersion {
    pub fn protocol_value(self) -> u64 {
        match self {
            Self::Test => 0,
            Self::V1 => 1,
        }
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        match unsigned(value)? {
            0 => Ok(Self::Test),
            1 => Ok(Self::V1),
            value => Err(Error::UnknownEnum {
                kind: "passphrase stretch version",
                value,
            }),
        }
    }

    pub fn to_value(self) -> Value {
        Value::Unsigned(self.protocol_value())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PpePassphraseBox {
    pub hybrid: HybridBox,
}

impl PpePassphraseBox {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = crate::decode(bytes)?;
        let fields = array(&value, 1)?;
        let result = Self {
            hybrid: crate::key_material::hybrid_box(&fields[0])?,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&self.to_value())?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![self.hybrid.to_value()])
    }

    fn validate(&self) -> Result<()> {
        if self.hybrid.kem_ciphertext.len() < 10
            || self.hybrid.dh_type != 1
            || !matches!(self.hybrid.sender_dh, Some(DhPublicKey::Curve25519(_)))
            || self.hybrid.ciphertext.len() < 16
        {
            return Err(Error::IntegerRange("PPE passphrase box"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PpePukBox {
    pub secret_box: SecretBox,
    pub puk_generation: u64,
    pub puk_role: Role,
}

impl PpePukBox {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = crate::decode(bytes)?;
        Self::from_value(&value)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&self.to_value())?)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 3)?;
        let result = Self {
            secret_box: crate::kv::secret_box(&fields[0])?,
            puk_generation: unsigned(&fields[1])?,
            puk_role: role(&fields[2])?,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            self.secret_box.to_value(),
            Value::Unsigned(self.puk_generation),
            self.puk_role.to_value(),
        ])
    }

    fn validate(&self) -> Result<()> {
        if self.puk_generation == 0
            || self.puk_role != Role::OWNER
            || self.secret_box.ciphertext.len() < 16
        {
            return Err(Error::IntegerRange("passphrase PUK backup"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PpeParcel {
    pub skmwk_box: SecretBox,
    pub generation: u64,
    pub passphrase_box: PpePassphraseBox,
    pub puk_box: Option<PpePukBox>,
    pub salt: [u8; 16],
    pub stretch_version: StretchVersion,
    pub verify_key: EntityId,
}

impl PpeParcel {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = crate::decode(bytes)?;
        Self::from_value(&value)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&self.to_value())?)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 7)?;
        let result = Self {
            skmwk_box: crate::kv::secret_box(&fields[0])?,
            generation: unsigned(&fields[1])?,
            passphrase_box: PpePassphraseBox::decode(&encode(&fields[2])?)?,
            puk_box: match &fields[3] {
                Value::Null => None,
                value => Some(PpePukBox::from_value(value)?),
            },
            salt: fixed_blob(&fields[4], "passphrase salt")?,
            stretch_version: StretchVersion::from_value(&fields[5])?,
            verify_key: entity(&fields[6])?.require_type(ENTITY_PASSPHRASE_KEY)?,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            self.skmwk_box.to_value(),
            Value::Unsigned(self.generation),
            self.passphrase_box.to_value(),
            self.puk_box
                .as_ref()
                .map_or(Value::Null, PpePukBox::to_value),
            Value::Binary(self.salt.to_vec()),
            self.stretch_version.to_value(),
            Value::Binary(self.verify_key.as_bytes().to_vec()),
        ])
    }

    fn validate(&self) -> Result<()> {
        if self.generation < FIRST_PASSPHRASE_GENERATION
            || self.salt == [0; 16]
            || self.skmwk_box.ciphertext.len() < 16
        {
            return Err(Error::IntegerRange("passphrase parcel"));
        }
        self.passphrase_box.validate()?;
        if let Some(puk_box) = &self.puk_box {
            puk_box.validate()?;
        }
        self.verify_key
            .clone()
            .require_type(ENTITY_PASSPHRASE_KEY)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PassphraseLoginResult {
    pub generation: u64,
    pub skmwk_box: SecretBox,
    pub passphrase_box: PpePassphraseBox,
}

/// Server-stored PPE material carried by `User.setPassphrase`,
/// `User.changePassphrase`, and the optional signup passphrase field.
///
/// Obsolete user-chain fields from the legacy schema are omitted here; canonical
/// zero values are emitted during serialization.
#[derive(Clone, Eq, PartialEq)]
pub struct PassphraseUpdateArgument {
    pub verify_key: EntityId,
    pub salt: [u8; 16],
    pub generation: u64,
    pub skmwk_box: SecretBox,
    pub passphrase_box: PpePassphraseBox,
    pub puk_box: Option<PpePukBox>,
    pub stretch_version: StretchVersion,
    pub user_settings_link: Option<crate::PostGenericLinkArgument>,
}

impl std::fmt::Debug for PassphraseUpdateArgument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PassphraseUpdateArgument")
            .field("verify_key", &self.verify_key)
            .field("generation", &self.generation)
            .field("stretch_version", &self.stretch_version)
            .field("ppe_boxes", &"[ENCRYPTED]")
            .finish()
    }
}

impl PassphraseUpdateArgument {
    pub fn decode_set(bytes: &[u8]) -> Result<Self> {
        Self::from_set_value(&crate::decode(bytes)?)
    }

    pub fn decode_change(bytes: &[u8], salt: [u8; 16]) -> Result<Self> {
        Self::from_change_value(&crate::decode(bytes)?, salt)
    }

    pub fn encoded_set(&self) -> Result<Vec<u8>> {
        self.validate()?;
        if self.generation != FIRST_PASSPHRASE_GENERATION {
            return Err(Error::IntegerRange("initial passphrase generation"));
        }
        Ok(encode(&self.to_set_value()?)?)
    }

    pub fn encoded_change(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&self.to_change_value()?)?)
    }

    pub fn to_set_value(&self) -> Result<Value> {
        Ok(Value::Array(vec![
            Value::Binary(self.verify_key.as_bytes().to_vec()),
            Value::Binary(self.salt.to_vec()),
            self.skmwk_box.to_value(),
            self.passphrase_box.to_value(),
            self.puk_box
                .as_ref()
                .map_or(Value::Null, PpePukBox::to_value),
            self.stretch_version.to_value(),
            // LinkOuter's zero union and TreeLocation's zero value. Both are
            // retained on the Go wire but ignored by v0.1.9's server.
            Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
            Value::Binary(vec![0; 32]),
            self.user_settings_link
                .as_ref()
                .map(crate::PostGenericLinkArgument::to_value)
                .transpose()?
                .unwrap_or(Value::Null),
        ]))
    }

    pub fn to_change_value(&self) -> Result<Value> {
        Ok(Value::Array(vec![
            Value::Binary(self.verify_key.as_bytes().to_vec()),
            self.skmwk_box.to_value(),
            self.passphrase_box.to_value(),
            self.puk_box
                .as_ref()
                .map_or(Value::Null, PpePukBox::to_value),
            self.stretch_version.to_value(),
            Value::Unsigned(self.generation),
            self.user_settings_link
                .as_ref()
                .map(crate::PostGenericLinkArgument::to_value)
                .transpose()?
                .unwrap_or(Value::Null),
        ]))
    }

    pub fn from_set_value(value: &Value) -> Result<Self> {
        let fields = array(value, 9)?;
        let legacy_link = array(&fields[6], 2)?;
        if !matches!(legacy_link, [Value::Unsigned(0), Value::Variant(None)]) {
            return Err(Error::Type {
                expected: "zero legacy passphrase link",
                found: "another value",
            });
        }
        let _: [u8; 32] = fixed_blob(&fields[7], "legacy passphrase tree location")?;
        let result = Self {
            verify_key: entity(&fields[0])?.require_type(ENTITY_PASSPHRASE_KEY)?,
            salt: fixed_blob(&fields[1], "passphrase salt")?,
            generation: FIRST_PASSPHRASE_GENERATION,
            skmwk_box: crate::kv::secret_box(&fields[2])?,
            passphrase_box: PpePassphraseBox::decode(&encode(&fields[3])?)?,
            puk_box: optional_puk_box(&fields[4])?,
            stretch_version: StretchVersion::from_value(&fields[5])?,
            user_settings_link: match &fields[8] {
                Value::Null => None,
                value => Some(crate::PostGenericLinkArgument::from_value(value)?),
            },
        };
        result.validate()?;
        Ok(result)
    }

    pub fn from_change_value(value: &Value, salt: [u8; 16]) -> Result<Self> {
        Self::from_change_value_inner(value, salt, true)
    }

    pub(crate) fn from_unbound_change_value(value: &Value) -> Result<Self> {
        Self::from_change_value_inner(value, [0; 16], false)
    }

    fn from_change_value_inner(value: &Value, salt: [u8; 16], require_salt: bool) -> Result<Self> {
        let fields = array(value, 7)?;
        let result = Self {
            verify_key: entity(&fields[0])?.require_type(ENTITY_PASSPHRASE_KEY)?,
            salt,
            generation: unsigned(&fields[5])?,
            skmwk_box: crate::kv::secret_box(&fields[1])?,
            passphrase_box: PpePassphraseBox::decode(&encode(&fields[2])?)?,
            puk_box: optional_puk_box(&fields[3])?,
            stretch_version: StretchVersion::from_value(&fields[4])?,
            user_settings_link: match &fields[6] {
                Value::Null => None,
                value => Some(crate::PostGenericLinkArgument::from_value(value)?),
            },
        };
        result.validate_with_salt(require_salt)?;
        Ok(result)
    }

    fn validate(&self) -> Result<()> {
        self.validate_with_salt(true)
    }

    fn validate_with_salt(&self, require_salt: bool) -> Result<()> {
        self.verify_key
            .clone()
            .require_type(ENTITY_PASSPHRASE_KEY)?;
        if (require_salt && self.salt == [0; 16]) || self.generation < FIRST_PASSPHRASE_GENERATION {
            return Err(Error::IntegerRange("passphrase update"));
        }
        if self.skmwk_box.ciphertext.len() < 16 {
            return Err(Error::IntegerRange("passphrase SKMWK box"));
        }
        self.passphrase_box.validate()?;
        if let Some(puk_box) = &self.puk_box {
            puk_box.validate()?;
        }
        if self.stretch_version != StretchVersion::V1 {
            return Err(Error::UnknownEnum {
                kind: "production passphrase stretch version",
                value: self.stretch_version.protocol_value(),
            });
        }
        Ok(())
    }
}

fn optional_puk_box(value: &Value) -> Result<Option<PpePukBox>> {
    match value {
        Value::Null => Ok(None),
        value => Ok(Some(PpePukBox::from_value(value)?)),
    }
}

impl PassphraseLoginResult {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = crate::decode(bytes)?;
        let fields = array(&value, 3)?;
        let generation = unsigned(&fields[0])?;
        let skmwk_box = crate::kv::secret_box(&fields[1])?;
        let passphrase_box = PpePassphraseBox::decode(&encode(&fields[2])?)?;
        if generation < FIRST_PASSPHRASE_GENERATION || skmwk_box.ciphertext.len() < 16 {
            return Err(Error::IntegerRange("passphrase generation"));
        }
        Ok(Self {
            generation,
            skmwk_box,
            passphrase_box,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        if self.generation < FIRST_PASSPHRASE_GENERATION || self.skmwk_box.ciphertext.len() < 16 {
            return Err(Error::IntegerRange("passphrase generation"));
        }
        self.passphrase_box.validate()?;
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(self.generation),
            self.skmwk_box.to_value(),
            self.passphrase_box.to_value(),
        ]))?)
    }
}

#[derive(Eq, PartialEq)]
pub struct SkmwkList {
    pub uid: EntityId,
    pub host: EntityId,
    pub keys: Vec<[u8; 32]>,
}

impl std::fmt::Debug for SkmwkList {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SkwmkList")
            .field("uid", &self.uid)
            .field("host", &self.host)
            .field("keys", &"[REDACTED]")
            .finish()
    }
}

impl Drop for SkmwkList {
    fn drop(&mut self) {
        self.keys.zeroize();
    }
}

impl SkmwkList {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = crate::decode(bytes)?;
        let fields = array(&value, 2)?;
        let fqu = array(&fields[0], 2)?;
        let key_values = match &fields[1] {
            Value::Null => &[][..],
            Value::Array(values) => values.as_slice(),
            _ => {
                return Err(Error::Type {
                    expected: "SKMWK list",
                    found: "another value",
                })
            }
        };
        Ok(Self {
            uid: entity(&fqu[0])?.require_type(ENTITY_USER)?,
            host: entity(&fqu[1])?.require_type(ENTITY_HOST)?,
            keys: key_values
                .iter()
                .map(|value| fixed_blob(value, "SKMWK"))
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        if self.keys.is_empty() {
            return Err(Error::FieldCount {
                expected: 1,
                found: 0,
            });
        }
        Ok(encode(&Value::Array(vec![
            Value::Array(vec![
                Value::Binary(self.uid.as_bytes().to_vec()),
                Value::Binary(self.host.as_bytes().to_vec()),
            ]),
            Value::Array(
                self.keys
                    .iter()
                    .map(|key| Value::Binary(key.to_vec()))
                    .collect(),
            ),
        ]))?)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PpePassphraseBoxPayload {
    pub generation: u64,
    pub session_key: [u8; 32],
}

impl std::fmt::Debug for PpePassphraseBoxPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PpePassphraseBoxPayload")
            .field("generation", &self.generation)
            .field("session_key", &"[REDACTED]")
            .finish()
    }
}

impl Drop for PpePassphraseBoxPayload {
    fn drop(&mut self) {
        self.session_key.zeroize();
    }
}

impl PpePassphraseBoxPayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = crate::decode(bytes)?;
        let fields = array(&value, 2)?;
        let generation = unsigned(&fields[0])?;
        if generation < FIRST_PASSPHRASE_GENERATION {
            return Err(Error::IntegerRange("passphrase generation"));
        }
        Ok(Self {
            generation,
            session_key: fixed_blob(&fields[1], "PPE session key")?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        if self.generation < FIRST_PASSPHRASE_GENERATION {
            return Err(Error::IntegerRange("passphrase generation"));
        }
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(self.generation),
            Value::Binary(self.session_key.to_vec()),
        ]))?)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PpePukBoxPayload {
    pub generation: u64,
    pub session_key: [u8; 32],
    pub passphrase_public_key: Hepk,
}

impl std::fmt::Debug for PpePukBoxPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PpePukBoxPayload")
            .field("generation", &self.generation)
            .field("session_key", &"[REDACTED]")
            .field("passphrase_public_key", &self.passphrase_public_key)
            .finish()
    }
}

impl Drop for PpePukBoxPayload {
    fn drop(&mut self) {
        self.session_key.zeroize();
    }
}

impl PpePukBoxPayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = crate::decode(bytes)?;
        let fields = array(&value, 3)?;
        let generation = unsigned(&fields[0])?;
        if generation < FIRST_PASSPHRASE_GENERATION {
            return Err(Error::IntegerRange("passphrase generation"));
        }
        Ok(Self {
            generation,
            session_key: fixed_blob(&fields[1], "PPE session key")?,
            passphrase_public_key: crate::hepk(&fields[2])?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        if self.generation < FIRST_PASSPHRASE_GENERATION {
            return Err(Error::IntegerRange("passphrase generation"));
        }
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(self.generation),
            Value::Binary(self.session_key.to_vec()),
            crate::decode(&self.passphrase_public_key.encoded()?)?,
        ]))?)
    }
}

pub fn decode_salt(bytes: &[u8]) -> Result<[u8; 16]> {
    fixed_blob(&crate::decode(bytes)?, "passphrase salt")
}

pub fn decode_generation(bytes: &[u8]) -> Result<u64> {
    let generation = unsigned(&crate::decode(bytes)?)?;
    if generation < FIRST_PASSPHRASE_GENERATION {
        return Err(Error::IntegerRange("passphrase generation"));
    }
    Ok(generation)
}

pub fn decode_stretch_version(bytes: &[u8]) -> Result<StretchVersion> {
    StretchVersion::from_value(&crate::decode(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_versions_are_exact_and_bounded() {
        assert_eq!(
            StretchVersion::from_value(&Value::Unsigned(0)).unwrap(),
            StretchVersion::Test
        );
        assert_eq!(
            StretchVersion::from_value(&Value::Unsigned(1)).unwrap(),
            StretchVersion::V1
        );
        assert!(StretchVersion::from_value(&Value::Unsigned(2)).is_err());
    }

    #[test]
    fn passphrase_boxes_require_the_v019_functional_hybrid_shape() {
        let boxed = PpePassphraseBox {
            hybrid: HybridBox {
                kem_ciphertext: vec![0; 9],
                dh_type: 1,
                sender_dh: Some(DhPublicKey::Curve25519([1; 32])),
                nonce: [2; 16],
                ciphertext: vec![3; 16],
            },
        };
        assert!(boxed.encoded().is_err());

        let boxed = PpePassphraseBox {
            hybrid: HybridBox {
                kem_ciphertext: vec![0; 10],
                sender_dh: None,
                ..boxed.hybrid
            },
        };
        assert!(boxed.encoded().is_err());
    }
}
