use foks_proto::{
    EntityId, PermissionToken, ENTITY_BACKUP_KEY, ENTITY_BOT_TOKEN_KEY, ENTITY_DEVICE, ENTITY_USER,
    ENTITY_YUBI,
};
use foks_snowpack::{decode, encode, Value};

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveUsernameAuthorization {
    LocalUser,
    OpenHost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolveUsernameArgument {
    pub name: Vec<u8>,
    pub authorization: ResolveUsernameAuthorization,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeKeyExistsArgument {
    pub uid: EntityId,
    pub device_id: EntityId,
    pub self_token: PermissionToken,
}

pub fn decode_check_name_exists(bytes: &[u8]) -> Result<Vec<u8>> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("check-name argument wrapper"));
    };
    let [Value::Text(name)] = fields.as_slice() else {
        return Err(shape("check-name argument"));
    };
    Ok(name.clone())
}

pub fn decode_resolve_username(bytes: &[u8]) -> Result<ResolveUsernameArgument> {
    let Value::Array(outer) = decode(bytes)? else {
        return Err(shape("resolve-username argument wrapper"));
    };
    let [Value::Array(fields)] = outer.as_slice() else {
        return Err(shape("resolve-username argument"));
    };
    let [Value::Text(name), Value::Array(authorization)] = fields.as_slice() else {
        return Err(shape("resolve-username fields"));
    };
    let [Value::Unsigned(tag), Value::Variant(value)] = authorization.as_slice() else {
        return Err(shape("resolve-username authorization"));
    };
    let authorization = match (*tag, value) {
        (0, None) => ResolveUsernameAuthorization::LocalUser,
        (4, None) => ResolveUsernameAuthorization::OpenHost,
        _ => return Err(shape("supported resolve-username authorization")),
    };
    Ok(ResolveUsernameArgument {
        name: name.clone(),
        authorization,
    })
}

pub fn decode_probe_key_exists(bytes: &[u8]) -> Result<ProbeKeyExistsArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("probe-key argument wrapper"));
    };
    let [Value::Binary(uid), Value::Binary(device_id), token] = fields.as_slice() else {
        return Err(shape("probe-key argument"));
    };
    let uid = EntityId::from_bytes(uid.clone())?.require_type(ENTITY_USER)?;
    let device_id = EntityId::from_bytes(device_id.clone())?;
    if !matches!(
        device_id.entity_type(),
        ENTITY_DEVICE | ENTITY_YUBI | ENTITY_BACKUP_KEY | ENTITY_BOT_TOKEN_KEY
    ) {
        return Err(shape("device identifier"));
    }
    Ok(ProbeKeyExistsArgument {
        uid,
        device_id,
        self_token: PermissionToken::decode(&encode(token)?)?,
    })
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_arguments_are_positional_and_strict() {
        let check = encode(&Value::Array(vec![Value::Text(b"alice".to_vec())])).unwrap();
        assert_eq!(decode_check_name_exists(&check).unwrap(), b"alice");

        let resolve = encode(&Value::Array(vec![Value::Array(vec![
            Value::Text(b"alice".to_vec()),
            Value::Array(vec![Value::Unsigned(4), Value::Variant(None)]),
        ])]))
        .unwrap();
        assert_eq!(
            decode_resolve_username(&resolve).unwrap(),
            ResolveUsernameArgument {
                name: b"alice".to_vec(),
                authorization: ResolveUsernameAuthorization::OpenHost,
            }
        );

        let mut uid = vec![1; 33];
        uid[0] = ENTITY_USER;
        let mut device = vec![2; 33];
        device[0] = ENTITY_DEVICE;
        let probe = encode(&Value::Array(vec![
            Value::Binary(uid),
            Value::Binary(device),
            Value::Binary(vec![3; 17]),
        ]))
        .unwrap();
        assert_eq!(
            decode_probe_key_exists(&probe).unwrap().self_token.expose(),
            &[3; 17]
        );
        for entity_type in [ENTITY_BACKUP_KEY, ENTITY_BOT_TOKEN_KEY] {
            let mut credential = vec![2; 33];
            credential[0] = entity_type;
            let probe = encode(&Value::Array(vec![
                Value::Binary([vec![ENTITY_USER], vec![1; 32]].concat()),
                Value::Binary(credential),
                Value::Binary(vec![3; 17]),
            ]))
            .unwrap();
            assert_eq!(
                decode_probe_key_exists(&probe)
                    .unwrap()
                    .device_id
                    .entity_type(),
                entity_type
            );
        }
    }
}
