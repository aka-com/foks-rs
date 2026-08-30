//! Interactive software-device key exchange values from FOKS v0.1.9.

use crate::{
    array, decode, encode, entity, fixed_blob, option, text, unsigned, DeviceLabel, DeviceType,
    EntityId, Hepk, Result, SecretBox, Signature, StretchVersion, UserLink, Value,
};
use zeroize::Zeroize as _;

pub const KEX_SECRET_BYTES: usize = 16;
pub const KEX_SESSION_ID_BYTES: usize = 32;
pub const KEX_PERMISSION_TOKEN_BYTES: usize = 17;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KexActorType {
    Provisioner,
    Provisionee,
}

impl KexActorType {
    pub const fn protocol_value(self) -> u64 {
        match self {
            Self::Provisioner => 1,
            Self::Provisionee => 2,
        }
    }

    fn from_value(value: &Value) -> Result<Self> {
        match unsigned(value)? {
            1 => Ok(Self::Provisioner),
            2 => Ok(Self::Provisionee),
            value => Err(crate::Error::UnknownEnum {
                kind: "KEX actor type",
                value,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KexWrapperMessage {
    pub session_id: [u8; KEX_SESSION_ID_BYTES],
    pub sender: EntityId,
    pub sequence: u64,
    pub payload: SecretBox,
}

impl KexWrapperMessage {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_value(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>> {
        self.encoded()
    }

    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 4)?;
        Ok(Self {
            session_id: fixed_blob(&fields[0], "KEX session ID")?,
            sender: entity(&fields[1])?,
            sequence: unsigned(&fields[2])?,
            payload: SecretBox::decode(&encode(&fields[3])?)?,
        })
    }

    pub(crate) fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.session_id.to_vec()),
            Value::Binary(self.sender.as_bytes().to_vec()),
            Value::Unsigned(self.sequence),
            self.payload.to_value(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KexSendArgument {
    pub message: KexWrapperMessage,
    pub signature: Signature,
    pub actor: KexActorType,
}

impl KexSendArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        Ok(Self {
            message: KexWrapperMessage::from_value(&fields[0])?,
            signature: crate::host::signature(&fields[1])?,
            actor: KexActorType::from_value(&fields[2])?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.message.to_value(),
            self.signature.to_value(),
            Value::Unsigned(self.actor.protocol_value()),
        ]))?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KexReceiveArgument {
    pub session_id: [u8; KEX_SESSION_ID_BYTES],
    pub receiver: EntityId,
    pub sequence: u64,
    pub poll_wait_milliseconds: u64,
    pub actor: KexActorType,
}

impl KexReceiveArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 5)?;
        Ok(Self {
            session_id: fixed_blob(&fields[0], "KEX session ID")?,
            receiver: entity(&fields[1])?,
            sequence: unsigned(&fields[2])?,
            poll_wait_milliseconds: unsigned(&fields[3])?,
            actor: KexActorType::from_value(&fields[4])?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.session_id.to_vec()),
            Value::Binary(self.receiver.as_bytes().to_vec()),
            Value::Unsigned(self.sequence),
            Value::Unsigned(self.poll_wait_milliseconds),
            Value::Unsigned(self.actor.protocol_value()),
        ]))?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KexDeviceLabelAndName {
    pub label: DeviceLabel,
    pub normalization_version: u64,
    pub display_name: Vec<u8>,
}

impl KexDeviceLabelAndName {
    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 3)?;
        let label = array(&fields[0], 3)?;
        Ok(Self {
            label: DeviceLabel {
                device_type: DeviceType::try_from(unsigned(&label[0])?)?,
                normalized_name: text(&label[1])?.into_bytes(),
                serial: unsigned(&label[2])?,
            },
            normalization_version: unsigned(&fields[1])?,
            display_name: text(&fields[2])?.into_bytes(),
        })
    }

    fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Array(vec![
                Value::Unsigned(self.label.device_type.protocol_value()),
                Value::Text(self.label.normalized_name.clone()),
                Value::Unsigned(self.label.serial),
            ]),
            Value::Unsigned(self.normalization_version),
            Value::Text(self.display_name.clone()),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KexHelloMessage {
    pub entity: EntityId,
    pub hepk: Hepk,
    pub device_name: KexDeviceLabelAndName,
}

#[derive(Clone, Eq, PartialEq)]
pub struct KexPpe {
    pub skmwk: [u8; 32],
    pub passphrase_generation: u64,
    pub salt: [u8; 16],
    pub stretch_version: StretchVersion,
}

impl std::fmt::Debug for KexPpe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KexPpe")
            .field("skmwk", &"[REDACTED]")
            .field("passphrase_generation", &self.passphrase_generation)
            .field("salt", &self.salt)
            .field("stretch_version", &self.stretch_version)
            .finish()
    }
}

impl Drop for KexPpe {
    fn drop(&mut self) {
        self.skmwk.zeroize();
    }
}

impl KexPpe {
    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 4)?;
        Ok(Self {
            skmwk: fixed_blob(&fields[0], "KEX SKMWK")?,
            passphrase_generation: unsigned(&fields[1])?,
            salt: fixed_blob(&fields[2], "KEX passphrase salt")?,
            stretch_version: StretchVersion::from_value(&fields[3])?,
        })
    }

    fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.skmwk.to_vec()),
            Value::Unsigned(self.passphrase_generation),
            Value::Binary(self.salt.to_vec()),
            self.stretch_version.to_value(),
        ])
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct KexPleaseSign {
    pub link: UserLink,
    pub ppe: Option<KexPpe>,
    pub self_token: [u8; KEX_PERMISSION_TOKEN_BYTES],
}

impl std::fmt::Debug for KexPleaseSign {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KexPleaseSign")
            .field("link", &self.link)
            .field("ppe", &self.ppe)
            .field("self_token", &"[REDACTED]")
            .finish()
    }
}

impl Drop for KexPleaseSign {
    fn drop(&mut self) {
        self.self_token.zeroize();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KexMessage {
    Error(Value),
    Start,
    Hello(KexHelloMessage),
    PleaseSign(KexPleaseSign),
    OkSigned(Signature),
    Done,
}

impl KexMessage {
    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 2)?;
        let kind = unsigned(&fields[0])?;
        match kind {
            0 => Ok(Self::Error(crate::variant(&fields[1], "0")?.clone())),
            1 if fields[1] == Value::Variant(None) => Ok(Self::Start),
            2 => {
                let hello = array(crate::variant(&fields[1], "1")?, 2)?;
                let key_suite = array(&hello[0], 2)?;
                Ok(Self::Hello(KexHelloMessage {
                    entity: entity(&key_suite[0])?,
                    hepk: Hepk::decode(&encode(&key_suite[1])?)?,
                    device_name: KexDeviceLabelAndName::from_value(&hello[1])?,
                }))
            }
            3 => {
                let request = array(crate::variant(&fields[1], "2")?, 3)?;
                Ok(Self::PleaseSign(KexPleaseSign {
                    link: UserLink::decode(&encode(&request[0])?)?,
                    ppe: option(&request[1], KexPpe::from_value)?,
                    self_token: fixed_blob(&request[2], "KEX self token")?,
                }))
            }
            4 => {
                let signed = array(crate::variant(&fields[1], "3")?, 1)?;
                Ok(Self::OkSigned(crate::host::signature(&signed[0])?))
            }
            5 if fields[1] == Value::Variant(None) => Ok(Self::Done),
            value => Err(crate::Error::UnknownEnum {
                kind: "KEX message type",
                value,
            }),
        }
    }

    fn to_value(&self) -> Result<Value> {
        let (kind, arm) = match self {
            Self::Error(status) => (
                0,
                Value::Variant(Some((b"0".to_vec(), Box::new(status.clone())))),
            ),
            Self::Start => (1, Value::Variant(None)),
            Self::Hello(hello) => (
                2,
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Array(vec![
                        Value::Array(vec![
                            Value::Binary(hello.entity.as_bytes().to_vec()),
                            decode(&hello.hepk.encoded()?)?,
                        ]),
                        hello.device_name.to_value(),
                    ])),
                ))),
            ),
            Self::PleaseSign(request) => (
                3,
                Value::Variant(Some((
                    b"2".to_vec(),
                    Box::new(Value::Array(vec![
                        decode(&request.link.encoded()?)?,
                        request.ppe.as_ref().map_or(Value::Null, KexPpe::to_value),
                        Value::Binary(request.self_token.to_vec()),
                    ])),
                ))),
            ),
            Self::OkSigned(signature) => (
                4,
                Value::Variant(Some((
                    b"3".to_vec(),
                    Box::new(Value::Array(vec![signature.to_value()])),
                ))),
            ),
            Self::Done => (5, Value::Variant(None)),
        };
        Ok(Value::Array(vec![Value::Unsigned(kind), arm]))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KexCleartext {
    pub session_id: [u8; KEX_SESSION_ID_BYTES],
    pub sender: EntityId,
    pub sequence: u64,
    pub message: KexMessage,
}

impl KexCleartext {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 4)?;
        Ok(Self {
            session_id: fixed_blob(&fields[0], "KEX cleartext session ID")?,
            sender: entity(&fields[1])?,
            sequence: unsigned(&fields[2])?,
            message: KexMessage::from_value(&fields[3])?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.session_id.to_vec()),
            Value::Binary(self.sender.as_bytes().to_vec()),
            Value::Unsigned(self.sequence),
            self.message.to_value()?,
        ]))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receive_argument_round_trips() {
        let argument = KexReceiveArgument {
            session_id: [1; 32],
            receiver: EntityId::from_bytes([vec![crate::ENTITY_DEVICE], vec![2; 32]].concat())
                .unwrap(),
            sequence: 3,
            poll_wait_milliseconds: 60_000,
            actor: KexActorType::Provisionee,
        };
        assert_eq!(
            KexReceiveArgument::decode(&argument.encoded().unwrap()).unwrap(),
            argument
        );
    }
}
