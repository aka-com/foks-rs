use foks_proto::{
    EntityId, FqTeam, PermissionToken, Role, RoleAndGeneration, Signature, TeamViewChallenge,
    TeamViewRequest,
};
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
    pub authorization: TeamChainAuthorization,
    pub start: u64,
    pub have_ptk_generations: Vec<RoleAndGeneration>,
    pub name_cursor: Option<(Vec<u8>, u64)>,
    pub load_removal_key: bool,
    pub load_remote_view_tokens: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TeamChainAuthorization {
    LocalView([u8; 16]),
    RemotePermission(PermissionToken),
    LocalParentTeam([u8; 16]),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadRemovalForMemberArgument {
    pub team: FqTeam,
    pub commitment: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadTeamMembershipChainArgument {
    pub team: FqTeam,
    pub token: [u8; 16],
    pub start: u64,
}

pub fn decode_load_team_membership_chain(bytes: &[u8]) -> Result<LoadTeamMembershipChainArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team membership-chain load argument"));
    };
    let [team, Value::Binary(token), Value::Unsigned(start)] = fields.as_slice() else {
        return Err(shape("team membership-chain load fields"));
    };
    if *start == 0 {
        return Err(shape("positive team membership-chain start"));
    }
    Ok(LoadTeamMembershipChainArgument {
        team: FqTeam::decode(&encode(team)?)?,
        token: token
            .as_slice()
            .try_into()
            .map_err(|_| shape("16-byte team-view bearer token"))?,
        start: *start,
    })
}

pub fn decode_load_removal_for_member(bytes: &[u8]) -> Result<LoadRemovalForMemberArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team removal-key load argument"));
    };
    let [team, Value::Binary(commitment)] = fields.as_slice() else {
        return Err(shape("team removal-key load fields"));
    };
    Ok(LoadRemovalForMemberArgument {
        team: FqTeam::decode(&encode(team)?)?,
        commitment: commitment
            .as_slice()
            .try_into()
            .map_err(|_| shape("32-byte removal-key commitment"))?,
    })
}

pub fn decode_team_view_request(bytes: &[u8]) -> Result<TeamViewRequest> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team-view challenge argument"));
    };
    let [request] = fields.as_slice() else {
        return Err(shape("team-view challenge fields"));
    };
    Ok(TeamViewRequest::decode(&encode(request)?)?)
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
    let [team_host, token, Value::Unsigned(start), have_ptk_generations, cursor, Value::Bool(load_removal_key), Value::Bool(load_remote_view_tokens)] =
        fields.as_slice()
    else {
        return Err(shape("supported local team-chain load fields"));
    };
    let [Value::Binary(team), Value::Binary(host)] = team_host_array(team_host)? else {
        return Err(shape("team and host IDs"));
    };
    let authorization = decode_view_token(token)?;
    let have_ptk_generations = decode_shared_key_generations(have_ptk_generations)?;
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
        authorization,
        start: *start,
        have_ptk_generations,
        name_cursor,
        load_removal_key: *load_removal_key,
        load_remote_view_tokens: *load_remote_view_tokens,
    })
}

fn decode_shared_key_generations(value: &Value) -> Result<Vec<RoleAndGeneration>> {
    let values = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(values) => values,
        _ => return Err(shape("PTK generation list")),
    };
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let Value::Array(fields) = value else {
            return Err(shape("PTK generation fields"));
        };
        let [role, Value::Unsigned(generation)] = fields.as_slice() else {
            return Err(shape("PTK generation fields"));
        };
        let role = Role::decode(&encode(role)?)?;
        result.push(RoleAndGeneration {
            role,
            generation: *generation,
        });
    }
    Ok(result)
}

fn team_host_array(value: &Value) -> Result<&[Value]> {
    match value {
        Value::Array(values) if values.len() == 2 => Ok(values),
        _ => Err(shape("team and host struct")),
    }
}

fn decode_view_token(value: &Value) -> Result<TeamChainAuthorization> {
    let Value::Array(fields) = value else {
        return Err(shape("team-view token union"));
    };
    let [Value::Unsigned(kind), Value::Variant(Some((tag, value)))] = fields.as_slice() else {
        return Err(shape("team-view token union"));
    };
    match (*kind, tag.as_slice()) {
        (1, b"0") => {
            let Value::Binary(token) = value.as_ref() else {
                return Err(shape("local team-view token bytes"));
            };
            Ok(TeamChainAuthorization::LocalView(
                token
                    .as_slice()
                    .try_into()
                    .map_err(|_| shape("16-byte team-view token"))?,
            ))
        }
        (2, b"1") => Ok(TeamChainAuthorization::RemotePermission(
            PermissionToken::decode(&encode(value.as_ref())?)?,
        )),
        (3, b"2") => {
            let Value::Binary(token) = value.as_ref() else {
                return Err(shape("local parent team-view token bytes"));
            };
            Ok(TeamChainAuthorization::LocalParentTeam(
                token
                    .as_slice()
                    .try_into()
                    .map_err(|_| shape("16-byte team-view token"))?,
            ))
        }
        _ => Err(shape("supported team-chain authorization")),
    }
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
    fn go_v019_local_parent_team_token_decodes() {
        let value = Value::Array(vec![
            Value::Unsigned(3),
            Value::Variant(Some((b"2".to_vec(), Box::new(Value::Binary(vec![7; 16]))))),
        ]);
        assert_eq!(
            decode_view_token(&value).unwrap(),
            TeamChainAuthorization::LocalParentTeam([7; 16])
        );
    }

    #[test]
    fn go_v019_removal_lookup_decodes_bare_positional_fields() {
        let mut team = vec![3; 33];
        team[0] = foks_proto::ENTITY_NAMED_TEAM;
        let mut host = vec![2; 33];
        host[0] = foks_proto::ENTITY_HOST;
        let argument = encode(&Value::Array(vec![
            Value::Array(vec![
                Value::Binary(team.clone()),
                Value::Binary(host.clone()),
            ]),
            Value::Binary(vec![0x44; 32]),
        ]))
        .unwrap();
        let decoded = decode_load_removal_for_member(&argument).unwrap();
        assert_eq!(decoded.team.team.as_bytes(), team);
        assert_eq!(decoded.team.host.as_bytes(), host);
        assert_eq!(decoded.commitment, [0x44; 32]);
    }
}
