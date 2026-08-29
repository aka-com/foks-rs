//! Hybrid encryption public keys used by user and team chains.

use crate::{
    array, binary, decode, encode, expect_unsigned, fixed_blob, unsigned, variant, Error, Result,
    Value,
};

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

    /// Constructs the exact v0.1.9 P-256 + ML-KEM-768 HEPK used by a Yubi
    /// credential. Both public keys are validated by decoding the canonical
    /// representation before it is returned.
    pub fn yubi(p256: [u8; 33], mlkem768: Vec<u8>) -> Result<Self> {
        let value = Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Array(vec![
                        Value::Unsigned(2),
                        Value::Variant(Some((
                            b"1".to_vec(),
                            Box::new(Value::Binary(p256.to_vec())),
                        ))),
                    ]),
                    Value::Array(vec![
                        Value::Unsigned(1),
                        Value::Variant(Some((b"1".to_vec(), Box::new(Value::Binary(mlkem768))))),
                    ]),
                ])),
            ))),
        ]);
        hepk(&value)
    }
}

pub(crate) fn hepk(value: &Value) -> Result<Hepk> {
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

pub(crate) fn dh_public(value: &Value) -> Result<DhPublicKey> {
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
