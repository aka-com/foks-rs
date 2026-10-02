//! Team creation, membership, authorization, and chain request codecs.

use crate::call::{encode_call, encode_call_with_validated_argument};
use crate::codec::ValueKind;
use crate::generated::{
    TEAM_ACTIVATE_BEARER_TOKEN_METHOD_POSITION, TEAM_ACTIVATE_VIEW_METHOD_POSITION,
    TEAM_ADMIN_PROTOCOL_ID, TEAM_CREATE_AD_HOC_METHOD_POSITION, TEAM_CREATE_NAMED_METHOD_POSITION,
    TEAM_EDIT_METHOD_POSITION, TEAM_GET_CONFIG_METHOD_POSITION,
    TEAM_GET_SERVER_CONFIG_METHOD_POSITION, TEAM_GET_VIEW_CHALLENGE_METHOD_POSITION,
    TEAM_LOADER_PROTOCOL_ID, TEAM_LOAD_CHAIN_METHOD_POSITION,
    TEAM_LOAD_MEMBERSHIP_CHAIN_METHOD_POSITION, TEAM_LOAD_REMOTE_VIEW_TOKENS_METHOD_POSITION,
    TEAM_LOAD_REMOVAL_KEY_BOX_METHOD_POSITION, TEAM_MAKE_INERT_BEARER_TOKEN_METHOD_POSITION,
    TEAM_MEMBER_GRANT_REMOTE_VIEW_PERMISSION_METHOD_POSITION, TEAM_MEMBER_PROTOCOL_ID,
    TEAM_POST_MEMBERSHIP_LINK_METHOD_POSITION, TEAM_RESERVE_NAME_METHOD_POSITION,
    USER_GET_TEAM_LIST_SERVER_TRUST_METHOD_POSITION, USER_PROTOCOL_ID,
};
use crate::identity::encode_load_user_chain_with_authorization;
use crate::Result;
use foks_proto::{
    AdHocTeamCreateArgument, AddTeamMemberArgument, EntityId, FqParty, FqTeam,
    NamedTeamCreateArgument, PermissionToken, RemoteViewPermissionPayload,
    RemoveTeamMemberArgument, Role, RoleAndGeneration, Signature, TeamBearerToken,
    TeamBearerTokenChallenge, TeamEditResult, TeamMetadataEditArgument, TeamNameReservation,
    TeamRemovalKeyBox, TeamViewChallenge, TeamViewRequest,
};
use foks_snowpack::{decode, encode, Value};

/// Encodes the `AsLocalTeam` form used while hydrating a local team roster.
/// The token is the activated team-view token for the loading member.
pub fn encode_load_user_chain_as_local_team_request(
    uid: &[u8],
    start: u64,
    current_name: Option<(&[u8], u64)>,
    token: &[u8; 16],
) -> Result<Vec<u8>> {
    let as_local_team = Value::Array(vec![
        Value::Unsigned(3),
        Value::Variant(Some((
            b"3".to_vec(),
            Box::new(Value::Binary(token.to_vec())),
        ))),
    ]);
    encode_load_user_chain_with_authorization(uid, start, current_name, as_local_team)
}

pub fn encode_grant_remote_view_permission_for_team_request(
    payload: &RemoteViewPermissionPayload,
    signature: &Signature,
    generation: u64,
    role: Role,
) -> Result<Vec<u8>> {
    if generation == 0 || role == Role::NONE {
        return Err(foks_proto::Error::IntegerRange("team shared-key authorization").into());
    }
    encode_call(
        TEAM_MEMBER_PROTOCOL_ID,
        TEAM_MEMBER_GRANT_REMOTE_VIEW_PERMISSION_METHOD_POSITION,
        &encode(&Value::Array(vec![
            payload.to_value(),
            Value::Array(vec![
                signature.to_value(),
                Value::Unsigned(generation),
                role.to_value(),
            ]),
        ]))?,
        0,
    )
}

pub fn encode_load_team_remote_view_tokens_request(
    team: &FqTeam,
    token: &[u8; 16],
    members: &[FqParty],
) -> Result<Vec<u8>> {
    if members.len() > 256 {
        return Err(foks_proto::Error::IntegerRange("remote team-view member count").into());
    }
    let members = if members.is_empty() {
        Value::Null
    } else {
        Value::Array(members.iter().map(FqParty::to_value).collect())
    };
    encode_call(
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_LOAD_REMOTE_VIEW_TOKENS_METHOD_POSITION,
        &encode(&Value::Array(vec![
            team.to_value(),
            Value::Binary(token.to_vec()),
            members,
        ]))?,
        0,
    )
}

pub fn encode_load_team_membership_chain_request(
    team: &EntityId,
    host: &EntityId,
    token: &[u8; 16],
    start: u64,
) -> Result<Vec<u8>> {
    if start == 0 {
        return Err(foks_proto::Error::IntegerRange("team membership-chain request").into());
    }
    let argument = encode(&Value::Array(vec![
        Value::Array(vec![
            Value::Binary(team.as_bytes().to_vec()),
            Value::Binary(host.as_bytes().to_vec()),
        ]),
        Value::Binary(token.to_vec()),
        Value::Unsigned(start),
    ]))?;
    encode_call(
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_LOAD_MEMBERSHIP_CHAIN_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_post_team_membership_link_request(
    token: &TeamBearerToken,
    argument: &foks_proto::PostGenericLinkArgument,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        Value::Binary(token.to_vec()),
        decode(&argument.encoded()?)?,
    ]))?;
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_POST_MEMBERSHIP_LINK_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_get_team_list_server_trust_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        USER_PROTOCOL_ID,
        USER_GET_TEAM_LIST_SERVER_TRUST_METHOD_POSITION,
        &[0x90],
        0,
    )
}

pub fn encode_create_adhoc_team_request(argument: &AdHocTeamCreateArgument<'_>) -> Result<Vec<u8>> {
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_CREATE_AD_HOC_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_team_loader_server_config_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_GET_SERVER_CONFIG_METHOD_POSITION,
        &[0x90],
        0,
    )
}

pub fn encode_team_admin_config_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_GET_CONFIG_METHOD_POSITION,
        &[0x90],
        0,
    )
}

pub fn encode_reserve_team_name_request(name: &[u8]) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Text(name.to_vec())]))?;
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_RESERVE_NAME_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn decode_team_name_reservation(response: &[u8]) -> Result<TeamNameReservation> {
    TeamNameReservation::decode(response).map_err(Into::into)
}

pub fn encode_create_named_team_request(argument: &NamedTeamCreateArgument<'_>) -> Result<Vec<u8>> {
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_CREATE_NAMED_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_add_team_member_request(argument: &AddTeamMemberArgument<'_>) -> Result<Vec<u8>> {
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_EDIT_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_team_metadata_edit_request(
    argument: &TeamMetadataEditArgument<'_>,
) -> Result<Vec<u8>> {
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_EDIT_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_remove_team_member_request(
    argument: &RemoveTeamMemberArgument<'_>,
) -> Result<Vec<u8>> {
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_EDIT_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn decode_team_edit_result(response: &[u8]) -> Result<TeamEditResult> {
    TeamEditResult::decode(response).map_err(Into::into)
}

pub fn encode_make_team_bearer_token_request(
    team: &EntityId,
    role: Role,
    generation: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        Value::Binary(team.as_bytes().to_vec()),
        role.to_value(),
        Value::Unsigned(generation),
    ]))?;
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_MAKE_INERT_BEARER_TOKEN_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn decode_team_bearer_token(response: &[u8]) -> Result<TeamBearerToken> {
    match decode(response)? {
        Value::Binary(bytes) => bytes.try_into().map_err(|bytes: Vec<u8>| {
            foks_proto::Error::Length {
                kind: "team bearer token",
                expected: 16,
                found: bytes.len(),
            }
            .into()
        }),
        other => Err(foks_proto::Error::Type {
            expected: "binary",
            found: other.kind(),
        }
        .into()),
    }
}

pub fn encode_activate_team_bearer_token_request(
    challenge: &TeamBearerTokenChallenge,
    signature: &Signature,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        Value::Binary(challenge.encoded_payload()?),
        signature.to_value(),
    ]))?;
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_ACTIVATE_BEARER_TOKEN_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_load_team_removal_key_box_request(
    token: &TeamBearerToken,
    member: &EntityId,
    member_host: &EntityId,
    source_role: Role,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        Value::Binary(token.to_vec()),
        Value::Array(vec![
            Value::Binary(member.as_bytes().to_vec()),
            Value::Binary(member_host.as_bytes().to_vec()),
        ]),
        source_role.to_value(),
    ]))?;
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_LOAD_REMOVAL_KEY_BOX_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn decode_team_removal_key_box(response: &[u8]) -> Result<TeamRemovalKeyBox> {
    TeamRemovalKeyBox::decode(response).map_err(Into::into)
}

pub fn encode_team_view_challenge_request(request: &TeamViewRequest) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![request.to_value()]))?;
    encode_call(
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_GET_VIEW_CHALLENGE_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_activate_team_view_request(
    challenge: &TeamViewChallenge,
    signature: &Signature,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        decode(&challenge.encoded()?)?,
        signature.to_value(),
    ]))?;
    encode_call(
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_ACTIVATE_VIEW_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_load_team_chain_request(
    team: &EntityId,
    host: &EntityId,
    token: &[u8; 16],
    start: u64,
) -> Result<Vec<u8>> {
    encode_load_team_chain_request_from(team, host, token, start, None)
}

pub fn encode_load_team_chain_request_from(
    team: &EntityId,
    host: &EntityId,
    token: &[u8; 16],
    start: u64,
    current_name: Option<(&[u8], u64)>,
) -> Result<Vec<u8>> {
    encode_load_team_chain_request_with_options(
        team,
        host,
        token,
        start,
        TeamChainLoadOptions {
            current_name,
            ..TeamChainLoadOptions::default()
        },
    )
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TeamChainLoadOptions<'a> {
    pub have_ptk_generations: &'a [RoleAndGeneration],
    pub current_name: Option<(&'a [u8], u64)>,
    pub load_removal_key: bool,
    pub load_remote_view_tokens: bool,
}

pub fn encode_load_team_chain_request_with_options(
    team: &EntityId,
    host: &EntityId,
    token: &[u8; 16],
    start: u64,
    options: TeamChainLoadOptions<'_>,
) -> Result<Vec<u8>> {
    let token = Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((
            b"0".to_vec(),
            Box::new(Value::Binary(token.to_vec())),
        ))),
    ]);
    encode_load_team_chain_with_authorization(team, host, token, start, options)
}

/// Loads a local child team's chain using a view token held by one of its
/// parent teams. The request remains authenticated as the user represented by
/// that parent-team token.
pub fn encode_load_team_chain_for_local_parent_request(
    team: &EntityId,
    host: &EntityId,
    parent_token: &[u8; 16],
    start: u64,
    options: TeamChainLoadOptions<'_>,
) -> Result<Vec<u8>> {
    let authorization = Value::Array(vec![
        Value::Unsigned(3),
        Value::Variant(Some((
            b"2".to_vec(),
            Box::new(Value::Binary(parent_token.to_vec())),
        ))),
    ]);
    encode_load_team_chain_with_authorization(team, host, authorization, start, options)
}

/// Loads a team chain from the public TeamLoader service with a federation
/// permission granted by the authoritative remote team.
pub fn encode_load_remote_team_chain_request(
    team: &EntityId,
    host: &EntityId,
    token: &PermissionToken,
    start: u64,
    current_name: Option<(&[u8], u64)>,
) -> Result<Vec<u8>> {
    encode_load_remote_team_chain_request_with_options(
        team,
        host,
        token,
        start,
        TeamChainLoadOptions {
            current_name,
            ..TeamChainLoadOptions::default()
        },
    )
}

pub fn encode_load_remote_team_chain_request_with_options(
    team: &EntityId,
    host: &EntityId,
    token: &PermissionToken,
    start: u64,
    options: TeamChainLoadOptions<'_>,
) -> Result<Vec<u8>> {
    let authorization = Value::Array(vec![
        Value::Unsigned(2),
        Value::Variant(Some((b"1".to_vec(), Box::new(token.to_value())))),
    ]);
    encode_load_team_chain_with_authorization(team, host, authorization, start, options)
}

pub(super) fn encode_load_team_chain_with_authorization(
    team: &EntityId,
    host: &EntityId,
    authorization: Value,
    start: u64,
    options: TeamChainLoadOptions<'_>,
) -> Result<Vec<u8>> {
    let have_ptk_generations = if options.have_ptk_generations.is_empty() {
        Value::Null
    } else {
        Value::Array(
            options
                .have_ptk_generations
                .iter()
                .copied()
                .map(RoleAndGeneration::to_value)
                .collect(),
        )
    };
    let argument = encode(&Value::Array(vec![
        Value::Array(vec![
            Value::Binary(team.as_bytes().to_vec()),
            Value::Binary(host.as_bytes().to_vec()),
        ]),
        authorization,
        Value::Unsigned(start),
        have_ptk_generations,
        options
            .current_name
            .map_or(Value::Null, |(name, next_sequence)| {
                Value::Array(vec![
                    Value::Text(name.to_vec()),
                    Value::Unsigned(next_sequence),
                ])
            }),
        Value::Bool(options.load_removal_key),
        Value::Bool(options.load_remote_view_tokens),
    ]))?;
    encode_call(
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_LOAD_CHAIN_METHOD_POSITION,
        &argument,
        0,
    )
}
