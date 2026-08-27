use foks_proto::{EntityId, RegistrationChallenge, Signature};
use foks_snowpack::{decode, encode, Value};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupUidByDeviceArgument {
    pub entity: EntityId,
    pub challenge: RegistrationChallenge,
    pub signature: Signature,
}

pub fn decode_uid_lookup_challenge(bytes: &[u8]) -> Result<EntityId> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("UID lookup challenge struct"));
    };
    let [Value::Binary(entity)] = fields.as_slice() else {
        return Err(shape("UID lookup challenge entity"));
    };
    Ok(EntityId::from_bytes(entity.clone())?)
}

pub fn decode_lookup_uid_by_device(bytes: &[u8]) -> Result<LookupUidByDeviceArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("UID lookup struct"));
    };
    let [Value::Binary(entity), challenge, signature] = fields.as_slice() else {
        return Err(shape("UID lookup fields"));
    };
    Ok(LookupUidByDeviceArgument {
        entity: EntityId::from_bytes(entity.clone())?,
        challenge: RegistrationChallenge::decode(&encode(challenge)?)?,
        signature: Signature::decode(&encode(signature)?)?,
    })
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}
