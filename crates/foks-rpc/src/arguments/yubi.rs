use foks_proto::{
    EntityId, RegistrationChallenge, Signature, YubiEncryptedManagementKey, ENTITY_YUBI,
};
use foks_snowpack::{decode, encode, Value};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadSubkeyBoxArgument {
    pub parent: EntityId,
    pub challenge: RegistrationChallenge,
    pub signature: Signature,
}

pub fn decode_subkey_box_challenge(bytes: &[u8]) -> Result<EntityId> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("subkey challenge struct"));
    };
    let [Value::Binary(parent)] = fields.as_slice() else {
        return Err(shape("subkey challenge parent"));
    };
    Ok(EntityId::from_bytes(parent.clone())?.require_type(ENTITY_YUBI)?)
}

pub fn decode_load_subkey_box(bytes: &[u8]) -> Result<LoadSubkeyBoxArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("load-subkey-box struct"));
    };
    let [Value::Binary(parent), challenge, signature] = fields.as_slice() else {
        return Err(shape("load-subkey-box fields"));
    };
    Ok(LoadSubkeyBoxArgument {
        parent: EntityId::from_bytes(parent.clone())?.require_type(ENTITY_YUBI)?,
        challenge: RegistrationChallenge::decode(&encode(challenge)?)?,
        signature: Signature::decode(&encode(signature)?)?,
    })
}

pub fn decode_put_yubi_management_key(bytes: &[u8]) -> Result<YubiEncryptedManagementKey> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("put-Yubi-management-key struct"));
    };
    let [value] = fields.as_slice() else {
        return Err(shape("put-Yubi-management-key value"));
    };
    Ok(YubiEncryptedManagementKey::decode(&encode(value)?)?)
}

pub fn decode_get_yubi_management_key(bytes: &[u8]) -> Result<EntityId> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("get-Yubi-management-key struct"));
    };
    let [Value::Binary(parent)] = fields.as_slice() else {
        return Err(shape("get-Yubi-management-key parent"));
    };
    Ok(EntityId::from_bytes(parent.clone())?.require_type(ENTITY_YUBI)?)
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}
