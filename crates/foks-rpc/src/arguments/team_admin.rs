use foks_proto::{
    DecodedAdHocTeamCreateArgument, DecodedNamedTeamCreateArgument, DecodedTeamEditArgument,
    EntityId, PostGenericLinkArgument, Role, Signature, TeamBearerToken, TeamBearerTokenChallenge,
};
use foks_snowpack::{decode, encode, Value};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MakeTeamBearerTokenArgument {
    pub team: EntityId,
    pub role: Role,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivateTeamBearerTokenArgument {
    pub challenge: TeamBearerTokenChallenge,
    pub signature: Signature,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadRemovalKeyBoxArgument {
    pub token: TeamBearerToken,
    pub member: EntityId,
    pub member_host: EntityId,
    pub source_role: Role,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostTeamMembershipLinkArgument {
    pub token: TeamBearerToken,
    pub link: PostGenericLinkArgument,
}

pub fn decode_check_team_bearer_token(bytes: &[u8]) -> Result<[u8; 16]> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team bearer-token check argument"));
    };
    let [Value::Binary(token)] = fields.as_slice() else {
        return Err(shape("team bearer-token check fields"));
    };
    token
        .as_slice()
        .try_into()
        .map_err(|_| shape("16-byte team bearer token"))
}

pub fn decode_post_team_membership_link(bytes: &[u8]) -> Result<PostTeamMembershipLinkArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team membership-link post argument"));
    };
    let [Value::Binary(token), link] = fields.as_slice() else {
        return Err(shape("team membership-link post fields"));
    };
    Ok(PostTeamMembershipLinkArgument {
        token: token
            .as_slice()
            .try_into()
            .map_err(|_| shape("16-byte team admin bearer token"))?,
        link: PostGenericLinkArgument::decode(&encode(link)?)?,
    })
}

pub fn decode_team_name_reservation_request(bytes: &[u8]) -> Result<Vec<u8>> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team-name reservation struct"));
    };
    let [Value::Text(name)] = fields.as_slice() else {
        return Err(shape("team name"));
    };
    Ok(name.clone())
}

pub fn decode_named_team_create(bytes: &[u8]) -> Result<DecodedNamedTeamCreateArgument> {
    Ok(DecodedNamedTeamCreateArgument::decode(bytes)?)
}

pub fn decode_adhoc_team_create(bytes: &[u8]) -> Result<DecodedAdHocTeamCreateArgument> {
    Ok(DecodedAdHocTeamCreateArgument::decode(bytes)?)
}

pub fn decode_team_edit(bytes: &[u8]) -> Result<DecodedTeamEditArgument> {
    Ok(DecodedTeamEditArgument::decode(bytes)?)
}

pub fn decode_make_team_bearer_token(bytes: &[u8]) -> Result<MakeTeamBearerTokenArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("inert team-token struct"));
    };
    let [Value::Binary(team), role, Value::Unsigned(generation)] = fields.as_slice() else {
        return Err(shape("inert team-token fields"));
    };
    Ok(MakeTeamBearerTokenArgument {
        team: EntityId::from_bytes(team.clone())?,
        role: Role::decode(&encode(role)?)?,
        generation: *generation,
    })
}

pub fn decode_activate_team_bearer_token(bytes: &[u8]) -> Result<ActivateTeamBearerTokenArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("team-token activation struct"));
    };
    let [Value::Binary(challenge), signature] = fields.as_slice() else {
        return Err(shape("team-token activation fields"));
    };
    Ok(ActivateTeamBearerTokenArgument {
        challenge: TeamBearerTokenChallenge::decode_payload(challenge)?,
        signature: Signature::decode(&encode(signature)?)?,
    })
}

pub fn decode_load_removal_key_box(bytes: &[u8]) -> Result<LoadRemovalKeyBoxArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("removal-key load struct"));
    };
    let [Value::Binary(token), member, role] = fields.as_slice() else {
        return Err(shape("removal-key load fields"));
    };
    let Value::Array(member) = member else {
        return Err(shape("removal-key member struct"));
    };
    let [Value::Binary(member_id), Value::Binary(member_host)] = member.as_slice() else {
        return Err(shape("removal-key member fields"));
    };
    Ok(LoadRemovalKeyBoxArgument {
        token: token
            .as_slice()
            .try_into()
            .map_err(|_| shape("16-byte team bearer token"))?,
        member: EntityId::from_bytes(member_id.clone())?,
        member_host: EntityId::from_bytes(member_host.clone())?,
        source_role: Role::decode(&encode(role)?)?,
    })
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}
