use foks_proto::{
    EntityId, FqParty, FqTeam, PermissionToken, RemoteViewPermissionPayload, Role, Signature,
    ENTITY_USER,
};
use foks_snowpack::{decode, encode, Value};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserChainAuthorization {
    LocalUser,
    RemoteToken(PermissionToken),
    OpenHost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadUserChainArgument {
    pub uid: EntityId,
    pub start: u64,
    pub name_cursor: Option<(Vec<u8>, u64)>,
    pub authorization: UserChainAuthorization,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedKeySignature {
    pub signature: Signature,
    pub generation: u64,
    pub role: Role,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantRemoteViewPermissionForTeamArgument {
    pub payload: RemoteViewPermissionPayload,
    pub authorization: SharedKeySignature,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadTeamRemoteViewTokensArgument {
    pub team: FqTeam,
    pub token: [u8; 16],
    pub members: Vec<FqParty>,
}

pub fn decode_load_user_chain_argument(bytes: &[u8]) -> Result<LoadUserChainArgument> {
    let Value::Array(outer) = decode(bytes)? else {
        return Err(shape("user-chain argument wrapper"));
    };
    let [Value::Array(fields)] = outer.as_slice() else {
        return Err(shape("user-chain argument"));
    };
    let [Value::Binary(uid), Value::Unsigned(start), name, authorization] = fields.as_slice()
    else {
        return Err(shape("user-chain argument fields"));
    };
    let uid = EntityId::from_bytes(uid.clone())?.require_type(ENTITY_USER)?;
    let name_cursor = match name {
        Value::Null => None,
        Value::Array(fields) => match fields.as_slice() {
            [Value::Text(name), Value::Unsigned(sequence)] => Some((name.clone(), *sequence)),
            _ => return Err(shape("user-chain name cursor")),
        },
        _ => return Err(shape("user-chain name cursor")),
    };
    let Value::Array(fields) = authorization else {
        return Err(shape("user-chain authorization"));
    };
    let [Value::Unsigned(tag), Value::Variant(value)] = fields.as_slice() else {
        return Err(shape("user-chain authorization fields"));
    };
    let authorization = match (*tag, value) {
        (0, None) => UserChainAuthorization::LocalUser,
        (1, Some((case, value))) if case == b"1" => {
            UserChainAuthorization::RemoteToken(PermissionToken::decode(&encode(value.as_ref())?)?)
        }
        (4, None) => UserChainAuthorization::OpenHost,
        _ => return Err(shape("supported user-chain authorization")),
    };
    Ok(LoadUserChainArgument {
        uid,
        start: *start,
        name_cursor,
        authorization,
    })
}

pub fn decode_grant_remote_view_permission_for_user(
    bytes: &[u8],
) -> Result<RemoteViewPermissionPayload> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("remote-view user grant wrapper"));
    };
    let [payload] = fields.as_slice() else {
        return Err(shape("remote-view user grant payload"));
    };
    Ok(RemoteViewPermissionPayload::decode(&encode(payload)?)?)
}

pub fn decode_grant_remote_view_permission_for_team(
    bytes: &[u8],
) -> Result<GrantRemoteViewPermissionForTeamArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("remote-view team grant wrapper"));
    };
    let [payload, Value::Array(signature)] = fields.as_slice() else {
        return Err(shape("remote-view team grant fields"));
    };
    let [signature, Value::Unsigned(generation), role] = signature.as_slice() else {
        return Err(shape("remote-view team grant signature"));
    };
    let role = Role::decode(&encode(role)?)?;
    if *generation == 0 || role == Role::NONE {
        return Err(shape("current team shared-key authorization"));
    }
    Ok(GrantRemoteViewPermissionForTeamArgument {
        payload: RemoteViewPermissionPayload::decode(&encode(payload)?)?,
        authorization: SharedKeySignature {
            signature: Signature::decode(&encode(signature)?)?,
            generation: *generation,
            role,
        },
    })
}

pub fn decode_load_team_remote_view_tokens(
    bytes: &[u8],
) -> Result<LoadTeamRemoteViewTokensArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("remote team-view token request"));
    };
    let [team, Value::Binary(token), members] = fields.as_slice() else {
        return Err(shape("remote team-view token fields"));
    };
    let token = token
        .as_slice()
        .try_into()
        .map_err(|_| shape("16-byte team-view bearer token"))?;
    let members = match members {
        Value::Null => Vec::new(),
        Value::Array(values) => values
            .iter()
            .map(|value| FqParty::decode(&encode(value)?).map_err(Into::into))
            .collect::<Result<Vec<_>>>()?,
        _ => return Err(shape("remote team-view member list")),
    };
    if members.len() > 256 {
        return Err(shape("bounded remote team-view member list"));
    }
    Ok(LoadTeamRemoteViewTokensArgument {
        team: FqTeam::decode(&encode(team)?)?,
        token,
        members,
    })
}

pub(crate) fn load_user_chain_argument_value(
    uid: &EntityId,
    start: u64,
    current_name: Option<(&[u8], u64)>,
    authorization: Value,
) -> Value {
    let name = current_name.map_or(Value::Null, |(name, next_sequence)| {
        Value::Array(vec![
            Value::Text(name.to_vec()),
            Value::Unsigned(next_sequence),
        ])
    });
    Value::Array(vec![Value::Array(vec![
        Value::Binary(uid.as_bytes().to_vec()),
        Value::Unsigned(start),
        name,
        authorization,
    ])])
}

pub(crate) fn remote_token_authorization(token: &PermissionToken) -> Value {
    Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((b"1".to_vec(), Box::new(token.to_value())))),
    ])
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

    fn entity(kind: u8, fill: u8) -> EntityId {
        EntityId::from_bytes([vec![kind], vec![fill; 32]].concat()).unwrap()
    }

    #[test]
    fn remote_user_chain_argument_is_strict() {
        let uid = entity(foks_proto::ENTITY_USER, 1);
        let token = PermissionToken::new([2; 17]);
        let value =
            load_user_chain_argument_value(&uid, 1, None, remote_token_authorization(&token));
        let decoded = decode_load_user_chain_argument(&encode(&value).unwrap()).unwrap();
        assert_eq!(decoded.uid, uid);
        assert!(matches!(
            decoded.authorization,
            UserChainAuthorization::RemoteToken(found) if found == token
        ));
    }

    #[test]
    fn remote_member_lists_are_bounded() {
        let team = FqTeam::new(
            entity(foks_proto::ENTITY_NAMED_TEAM, 3),
            entity(foks_proto::ENTITY_HOST, 4),
        )
        .unwrap();
        let member = FqParty::new(
            entity(foks_proto::ENTITY_USER, 5),
            entity(foks_proto::ENTITY_HOST, 6),
        )
        .unwrap();
        let value = Value::Array(vec![
            team.to_value(),
            Value::Binary(vec![7; 16]),
            Value::Array(vec![member.to_value(); 257]),
        ]);
        assert!(decode_load_team_remote_view_tokens(&encode(&value).unwrap()).is_err());
    }
}
