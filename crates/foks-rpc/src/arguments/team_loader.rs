use foks_proto::{EntityId, Signature, TeamViewChallenge, TeamViewRequest};
use foks_snowpack::{decode, encode, Value};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivateTeamViewArgument {
    pub challenge: TeamViewChallenge,
    pub signature: Signature,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadTeamChainArgument {
    pub team: EntityId,
    pub host: EntityId,
    pub token: [u8; 16],
    pub start: u64,
    pub name_cursor: Option<(Vec<u8>, u64)>,
}

pub fn decode_team_view_request(bytes: &[u8]) -> Result<TeamViewRequest> {
    Ok(TeamViewRequest::decode(bytes)?)
}

pub fn decode_activate_team_view(bytes: &[u8]) -> Result<ActivateTeamViewArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team-view activation struct"));
    };
    let [challenge, signature] = fields.as_slice() else {
        return Err(shape("team-view activation fields"));
    };
    Ok(ActivateTeamViewArgument {
        challenge: TeamViewChallenge::decode(&encode(challenge)?)?,
        signature: Signature::decode(&encode(signature)?)?,
    })
}

pub fn decode_load_team_chain(bytes: &[u8]) -> Result<LoadTeamChainArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team-chain load struct"));
    };
    let [team_host, token, Value::Unsigned(start), cursor, Value::Null, Value::Bool(false), Value::Bool(false)] =
        fields.as_slice()
    else {
        return Err(shape("supported local team-chain load fields"));
    };
    let [Value::Binary(team), Value::Binary(host)] = team_host_array(team_host)? else {
        return Err(shape("team and host IDs"));
    };
    let token = decode_view_token(token)?;
    let name_cursor = match cursor {
        Value::Null => None,
        Value::Array(values) => match values.as_slice() {
            [Value::Text(name), Value::Unsigned(sequence)] => Some((name.clone(), *sequence)),
            _ => return Err(shape("team name cursor")),
        },
        _ => return Err(shape("team name cursor")),
    };
    Ok(LoadTeamChainArgument {
        team: EntityId::from_bytes(team.clone())?,
        host: EntityId::from_bytes(host.clone())?,
        token,
        start: *start,
        name_cursor,
    })
}

fn team_host_array(value: &Value) -> Result<&[Value]> {
    match value {
        Value::Array(values) if values.len() == 2 => Ok(values),
        _ => Err(shape("team and host struct")),
    }
}

fn decode_view_token(value: &Value) -> Result<[u8; 16]> {
    let Value::Array(fields) = value else {
        return Err(shape("team-view token union"));
    };
    let [Value::Unsigned(1), Value::Variant(Some((tag, value)))] = fields.as_slice() else {
        return Err(shape("team-view token union"));
    };
    let Value::Binary(token) = value.as_ref() else {
        return Err(shape("team-view token bytes"));
    };
    if tag != b"0" || token.len() != 16 {
        return Err(shape("local team-view token"));
    }
    token
        .as_slice()
        .try_into()
        .map_err(|_| shape("16-byte team-view token"))
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}
