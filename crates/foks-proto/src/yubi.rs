//! Exact FOKS v0.1.9 YubiKey wire values.

use std::fmt;

use crate::{
    array, binary, decode, encode, fixed_blob, text, unsigned, EntityId, Error, Result, Role,
    SecretBox, Value, ENTITY_YUBI,
};
use zeroize::Zeroize as _;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YubiCardId {
    pub name: Vec<u8>,
    pub serial: u64,
}

impl YubiCardId {
    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 2)?;
        let name = text(&fields[0])?.into_bytes();
        if name.is_empty() || name.len() > 255 || name.contains(&0) {
            return Err(Error::IntegerRange("Yubi card name"));
        }
        Ok(Self {
            name,
            serial: unsigned(&fields[1])?,
        })
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Text(self.name.clone()),
            Value::Unsigned(self.serial),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YubiSlotAndPqKeyId {
    pub slot: u64,
    pub id: [u8; 32],
}

impl YubiSlotAndPqKeyId {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_value(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 2)?;
        let slot = unsigned(&fields[0])?;
        if !(0x82..=0x95).contains(&slot) {
            return Err(Error::IntegerRange("Yubi PIV slot"));
        }
        Ok(Self {
            slot,
            id: fixed_blob(&fields[1], "Yubi PQ key ID")?,
        })
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Unsigned(self.slot),
            Value::Binary(self.id.to_vec()),
        ])
    }
}

#[derive(Eq, PartialEq)]
pub struct YubiManagementKeyBoxPayload {
    pub management_key: [u8; 24],
    pub card: YubiCardId,
    pub slot: u64,
    pub yubi_id: EntityId,
}

impl Drop for YubiManagementKeyBoxPayload {
    fn drop(&mut self) {
        self.management_key.zeroize();
    }
}

impl fmt::Debug for YubiManagementKeyBoxPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("YubiManagementKeyBoxPayload")
            .field("management_key", &"<redacted>")
            .field("card", &self.card)
            .field("slot", &self.slot)
            .field("yubi_id", &self.yubi_id)
            .finish()
    }
}

impl YubiManagementKeyBoxPayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 4)?;
        let result = Self {
            management_key: fixed_blob(&fields[0], "Yubi management key")?,
            card: YubiCardId::from_value(&fields[1])?,
            slot: unsigned(&fields[2])?,
            yubi_id: EntityId::from_bytes(binary(&fields[3])?.to_vec())?,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.management_key.to_vec()),
            self.card.to_value(),
            Value::Unsigned(self.slot),
            Value::Binary(self.yubi_id.as_bytes().to_vec()),
        ]))?)
    }

    fn validate(&self) -> Result<()> {
        self.yubi_id.clone().require_type(ENTITY_YUBI)?;
        if !(0x82..=0x95).contains(&self.slot)
            || self.card.serial == 0
            || self.card.name.is_empty()
            || self.card.name.len() > 255
            || self.card.name.contains(&0)
        {
            return Err(Error::IntegerRange("Yubi management-key binding"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YubiEncryptedManagementKey {
    pub yubi_id: EntityId,
    pub secret_box: SecretBox,
    pub generation: u64,
    pub role: Role,
}

impl YubiEncryptedManagementKey {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_value(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value()?)?)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 4)?;
        let result = Self {
            yubi_id: EntityId::from_bytes(binary(&fields[0])?.to_vec())?,
            secret_box: SecretBox::decode(&encode(&fields[1])?)?,
            generation: unsigned(&fields[2])?,
            role: crate::identity::role(&fields[3])?,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn to_value(&self) -> Result<Value> {
        self.validate()?;
        Ok(Value::Array(vec![
            Value::Binary(self.yubi_id.as_bytes().to_vec()),
            self.secret_box.to_value(),
            Value::Unsigned(self.generation),
            self.role.to_value(),
        ]))
    }

    fn validate(&self) -> Result<()> {
        self.yubi_id.clone().require_type(ENTITY_YUBI)?;
        if self.generation == 0 || self.role == Role::NONE {
            return Err(Error::IntegerRange("Yubi management-key box"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pq_hint_round_trips_and_rejects_non_retired_slots() {
        let hint = YubiSlotAndPqKeyId {
            slot: 0x83,
            id: [9; 32],
        };
        assert_eq!(
            YubiSlotAndPqKeyId::decode(&hint.encoded().unwrap()).unwrap(),
            hint
        );
        let bad = encode(&Value::Array(vec![
            Value::Unsigned(0x9a),
            Value::Binary(vec![0; 32]),
        ]))
        .unwrap();
        assert!(YubiSlotAndPqKeyId::decode(&bad).is_err());
    }

    #[test]
    fn cleartext_management_key_debug_is_redacted() {
        let payload = YubiManagementKeyBoxPayload {
            management_key: [7; 24],
            card: YubiCardId {
                name: b"mock".to_vec(),
                serial: 1,
            },
            slot: 0x82,
            yubi_id: EntityId::from_bytes(
                std::iter::once(ENTITY_YUBI)
                    .chain([2; 33])
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        };
        let debug = format!("{payload:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("7, 7"));
    }
}
