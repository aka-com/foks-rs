//! Registration objects used to locate an account from an enrolled backup key.

use crate::{
    array, decode, encode, entity, fixed_blob, role, text, unsigned, EntityId, Result, Role, Value,
    ENTITY_HOST, ENTITY_USER,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistrationChallengePayload {
    pub hmac_key_id: [u8; 16],
    pub entity: EntityId,
    pub host: EntityId,
    pub random: [u8; 16],
    pub time: u64,
}

impl RegistrationChallengePayload {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub(crate) fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.hmac_key_id.to_vec()),
            Value::Binary(self.entity.as_bytes().to_vec()),
            Value::Binary(self.host.as_bytes().to_vec()),
            Value::Binary(self.random.to_vec()),
            Value::Unsigned(self.time),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistrationChallenge {
    pub payload: RegistrationChallengePayload,
    pub mac: [u8; 32],
}

impl RegistrationChallenge {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 2)?;
        let payload = array(&fields[0], 5)?;
        Ok(Self {
            payload: RegistrationChallengePayload {
                hmac_key_id: fixed_blob(&payload[0], "registration challenge HMAC key ID")?,
                entity: entity(&payload[1])?,
                host: entity(&payload[2])?.require_type(ENTITY_HOST)?,
                random: fixed_blob(&payload[3], "registration challenge random")?,
                time: unsigned(&payload[4])?,
            },
            mac: fixed_blob(&fields[1], "registration challenge MAC")?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.payload.to_value(),
            Value::Binary(self.mac.to_vec()),
        ]))?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupUserResult {
    pub uid: EntityId,
    pub host: EntityId,
    pub username: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub role: Role,
    /// Exact optional Yubi hint, retained because it is an opaque union here.
    pub yubi_pq_hint: Option<Vec<u8>>,
}

impl LookupUserResult {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 5)?;
        let fqu = array(&fields[0], 2)?;
        Ok(Self {
            uid: entity(&fqu[0])?.require_type(ENTITY_USER)?,
            host: entity(&fqu[1])?.require_type(ENTITY_HOST)?,
            username: text(&fields[1])?.into_bytes(),
            username_utf8: text(&fields[2])?.into_bytes(),
            role: role(&fields[3])?,
            yubi_pq_hint: match &fields[4] {
                Value::Null => None,
                value => Some(encode(value)?),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{DIR}/{name}")).unwrap()
    }

    #[test]
    fn official_lookup_objects_round_trip() {
        let challenge_bytes = fixture("backup-lookup-challenge.snowp");
        let challenge = RegistrationChallenge::decode(&challenge_bytes).unwrap();
        assert_eq!(challenge.encoded().unwrap(), challenge_bytes);
        assert_eq!(
            challenge.payload.entity.entity_type(),
            crate::ENTITY_BACKUP_KEY
        );

        let result = LookupUserResult::decode(&fixture("backup-lookup-result.snowp")).unwrap();
        assert_eq!(result.uid.entity_type(), ENTITY_USER);
        assert_eq!(result.host, challenge.payload.host);
        assert_eq!(result.role, Role::OWNER);
        assert!(result.yubi_pq_hint.is_none());
    }
}
