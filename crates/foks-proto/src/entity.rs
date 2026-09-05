//! Entity identifiers and exact signed wire blobs.

use crate::{
    decode, encode, signed_blob, Error, Result, Value, ENTITY_AD_HOC_TEAM, ENTITY_NAMED_TEAM,
    ENTITY_PTK_VERIFY, ENTITY_PUK_VERIFY, ENTITY_USER,
};

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

    pub fn to_rolling_entity_id(&self) -> Self {
        let mut bytes = self.0.clone();
        bytes[0] = match self.entity_type() {
            ENTITY_USER => ENTITY_PUK_VERIFY,
            ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM => ENTITY_PTK_VERIFY,
            other => other,
        };
        Self(bytes)
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
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        crate::host::signature(&crate::decode(bytes)?)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(kind: u8) -> EntityId {
        EntityId::from_bytes([vec![kind], vec![0x42; 32]].concat()).unwrap()
    }

    #[test]
    fn persistent_user_and_team_ids_normalize_to_rolling_verify_ids() {
        for (persistent, rolling) in [
            (ENTITY_USER, ENTITY_PUK_VERIFY),
            (ENTITY_NAMED_TEAM, ENTITY_PTK_VERIFY),
            (ENTITY_AD_HOC_TEAM, ENTITY_PTK_VERIFY),
        ] {
            let persistent = entity(persistent);
            let normalized = persistent.to_rolling_entity_id();
            assert_eq!(normalized.entity_type(), rolling);
            assert_eq!(&normalized.as_bytes()[1..], &persistent.as_bytes()[1..]);
        }
    }

    #[test]
    fn rolling_and_unrelated_ids_are_unchanged() {
        for kind in [ENTITY_PUK_VERIFY, ENTITY_PTK_VERIFY, crate::ENTITY_HOST] {
            let id = entity(kind);
            assert_eq!(id.to_rolling_entity_id(), id);
        }
    }
}
