use foks_proto::{
    EntityId, PassphraseUpdateArgument, RegistrationChallenge, Signature, ENTITY_USER,
};
use foks_snowpack::{decode, encode, Value};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PassphraseLoginArgument {
    pub uid: EntityId,
    pub challenge: RegistrationChallenge,
    pub signature: Signature,
}

pub fn decode_login_challenge(bytes: &[u8]) -> Result<EntityId> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("one-field passphrase challenge argument"));
    };
    let [Value::Binary(uid)] = fields.as_slice() else {
        return Err(shape("passphrase challenge UID"));
    };
    Ok(EntityId::from_bytes(uid.clone())?.require_type(ENTITY_USER)?)
}

pub fn decode_passphrase_login(bytes: &[u8]) -> Result<PassphraseLoginArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("passphrase login argument"));
    };
    let [Value::Binary(uid), challenge, signature] = fields.as_slice() else {
        return Err(shape("passphrase login fields"));
    };
    Ok(PassphraseLoginArgument {
        uid: EntityId::from_bytes(uid.clone())?.require_type(ENTITY_USER)?,
        challenge: RegistrationChallenge::decode(&encode(challenge)?)?,
        signature: Signature::decode(&encode(signature)?)?,
    })
}

pub fn decode_set_passphrase(bytes: &[u8]) -> Result<PassphraseUpdateArgument> {
    Ok(PassphraseUpdateArgument::decode_set(bytes)?)
}

pub fn decode_change_passphrase(bytes: &[u8], salt: [u8; 16]) -> Result<PassphraseUpdateArgument> {
    Ok(PassphraseUpdateArgument::decode_change(bytes, salt)?)
}

pub fn decode_void(bytes: &[u8]) -> Result<()> {
    match decode(bytes)? {
        Value::Null => Ok(()),
        _ => Err(shape("empty argument struct")),
    }
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}
