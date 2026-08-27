//! Strict framing and request encoding for FOKS v0.1.9 client RPCs.
//!
//! Snowpack RPC uses an ordinary MessagePack envelope (including named maps)
//! around canonical Snowpack protocol values. This crate keeps the envelope
//! separate from exact public and authenticated protocol payloads and returns
//! response bytes unchanged for protocol verification.

#![forbid(unsafe_code)]

use std::io::{Read, Write};

use foks_proto::{
    AdHocTeamCreateArgument, AddTeamMemberArgument, EntityId, KvDirectory, KvDirent,
    KvLargeFileMetadata, KvNodeId, KvPathVersionVector, KvSmallFileBox, KvUploadChunk,
    NamedTeamCreateArgument, ProvisionDeviceArgument, RegistrationChallenge,
    RemoveTeamMemberArgument, RevokeDeviceArgument, Role, Signature, SoftwareSignupArgument,
    TeamBearerToken, TeamBearerTokenChallenge, TeamEditResult, TeamNameReservation,
    TeamRemovalKeyBox, TeamViewChallenge, TeamViewRequest,
};
use foks_snowpack::{decode, encode, Value};
use thiserror::Error;

mod response;
mod server;

pub use response::{encode_status_response_at, encode_void_success_response_at, RpcStatus};
pub use server::{decode_call, read_call, DecodedCall};

pub const PROBE_PROTOCOL_ID: u64 = 0xc588_4ff6;
pub const PROBE_METHOD_POSITION: u64 = 1;
pub const REG_PROTOCOL_ID: u64 = 0xf7ab_85f3;
pub const REG_RESERVE_USERNAME_METHOD_POSITION: u64 = 0;
pub const REG_GET_CLIENT_CERT_CHAIN_METHOD_POSITION: u64 = 1;
pub const REG_SIGNUP_METHOD_POSITION: u64 = 2;
pub const REG_GET_UID_LOOKUP_CHALLENGE_METHOD_POSITION: u64 = 6;
pub const REG_LOOKUP_UID_BY_DEVICE_METHOD_POSITION: u64 = 7;
pub const REG_SELECT_VHOST_METHOD_POSITION: u64 = 15;
pub const USER_PROTOCOL_ID: u64 = 0x823f_0899;
pub const USER_PROVISION_DEVICE_METHOD_POSITION: u64 = 6;
pub const USER_REVOKE_DEVICE_METHOD_POSITION: u64 = 7;
pub const USER_LOAD_USER_CHAIN_METHOD_POSITION: u64 = 9;
pub const USER_GET_PUK_FOR_ROLE_METHOD_POSITION: u64 = 14;
pub const USER_GET_HOST_CONFIG_METHOD_POSITION: u64 = 24;
pub const MERKLE_QUERY_PROTOCOL_ID: u64 = 0xc041_2aa6;
pub const MERKLE_GET_HISTORICAL_ROOTS_METHOD_POSITION: u64 = 1;
pub const MERKLE_GET_CURRENT_ROOT_METHOD_POSITION: u64 = 2;
pub const MERKLE_SELECT_VHOST_METHOD_POSITION: u64 = 8;
pub const TEAM_LOADER_PROTOCOL_ID: u64 = 0xf912_8579;
pub const TEAM_ADMIN_PROTOCOL_ID: u64 = 0xdbe1_ddbe;
pub const TEAM_GET_VIEW_CHALLENGE_METHOD_POSITION: u64 = 0;
pub const TEAM_ACTIVATE_VIEW_METHOD_POSITION: u64 = 1;
pub const TEAM_LOAD_CHAIN_METHOD_POSITION: u64 = 3;
pub const TEAM_RESERVE_NAME_METHOD_POSITION: u64 = 0;
pub const TEAM_CREATE_NAMED_METHOD_POSITION: u64 = 1;
pub const TEAM_EDIT_METHOD_POSITION: u64 = 2;
pub const TEAM_MAKE_INERT_BEARER_TOKEN_METHOD_POSITION: u64 = 3;
pub const TEAM_ACTIVATE_BEARER_TOKEN_METHOD_POSITION: u64 = 4;
pub const TEAM_LOAD_REMOVAL_KEY_BOX_METHOD_POSITION: u64 = 10;
pub const KV_STORE_PROTOCOL_ID: u64 = 0x8ee3_7b6b;
pub const KV_MKDIR_METHOD_POSITION: u64 = 0;
pub const KV_PUT_METHOD_POSITION: u64 = 1;
pub const KV_PUT_ROOT_METHOD_POSITION: u64 = 2;
pub const KV_FILE_UPLOAD_INIT_METHOD_POSITION: u64 = 3;
pub const KV_FILE_UPLOAD_CHUNK_METHOD_POSITION: u64 = 4;
pub const KV_PUT_SMALL_FILE_OR_SYMLINK_METHOD_POSITION: u64 = 7;
pub const KV_GET_ROOT_METHOD_POSITION: u64 = 8;
pub const KV_GET_NODE_METHOD_POSITION: u64 = 10;
pub const KV_GET_ENCRYPTED_CHUNK_METHOD_POSITION: u64 = 11;
pub const KV_GET_DIR_METHOD_POSITION: u64 = 12;
pub const KV_CACHE_CHECK_METHOD_POSITION: u64 = 13;
pub const KV_LIST_METHOD_POSITION: u64 = 14;
pub const KV_LOCK_ACQUIRE_METHOD_POSITION: u64 = 15;
pub const KV_LOCK_RELEASE_METHOD_POSITION: u64 = 16;
pub const KV_SELECT_VHOST_METHOD_POSITION: u64 = 18;
pub const CURRENT_COMPATIBILITY_VERSION: u64 = 1;
pub const DEFAULT_MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;
pub const STATUS_TX_RETRY_ERROR: u64 = 1014;

const METHOD_CALL_V2: u64 = 5;
const METHOD_RESPONSE: u64 = 1;
const RESPONSE_HEADER: &[u8] = &[
    0x82, 0xa1, b'V', 0x01, 0xa2, b'f', b'1', 0x81, 0xa4, b'V', b'e', b'r', b's', 0x01,
];

#[derive(Debug, Error)]
pub enum Error {
    #[error("RPC I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid canonical Snowpack in RPC payload: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error(
        "invalid canonical Snowpack argument for protocol {protocol_id:#010x} method {method_position}: {source}"
    )]
    ArgumentSnowpack {
        protocol_id: u64,
        method_position: u64,
        source: foks_snowpack::Error,
    },
    #[error("invalid FOKS protocol value: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("RPC frame length {received} exceeds limit {maximum}")]
    FrameTooLarge { received: usize, maximum: usize },
    #[error("invalid or noncanonical RPC frame length marker {0:#04x}")]
    FrameLengthMarker(u8),
    #[error("truncated RPC MessagePack envelope")]
    Truncated,
    #[error("RPC MessagePack nesting exceeds the limit")]
    Depth,
    #[error("unsupported RPC MessagePack marker {0:#04x}")]
    Marker(u8),
    #[error("expected {expected}, found {found}")]
    Envelope {
        expected: &'static str,
        found: &'static str,
    },
    #[error("RPC response sequence is {received}, expected {expected}")]
    Sequence { expected: u64, received: u64 },
    #[error("FOKS server returned status {code}{detail}")]
    RemoteStatus { code: u64, detail: StatusDetail },
    #[error("FOKS KV cache is stale")]
    KvStaleCache(KvPathVersionVector),
    #[error("unsupported FOKS compatibility header")]
    Compatibility,
    #[error("invalid probe hostname")]
    Hostname,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusDetail(Option<String>);

impl std::fmt::Display for StatusDetail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(detail) = &self.0 {
            write!(formatter, ": {detail}")
        } else {
            Ok(())
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvAuth<'a> {
    User,
    Team(&'a [u8; 16]),
}

impl KvAuth<'_> {
    fn to_value(self) -> Value {
        match self {
            Self::User => Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
            Self::Team(token) => Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Binary(token.to_vec())),
                ))),
            ]),
        }
    }
}

fn kv_request_header(auth: KvAuth<'_>, precondition: Option<&KvPathVersionVector>) -> Value {
    Value::Array(vec![
        auth.to_value(),
        precondition.map_or(Value::Null, KvPathVersionVector::to_value),
    ])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvListCursor {
    None,
    Mac([u8; 32]),
    Time(u64),
}

impl KvListCursor {
    fn to_value(self) -> Value {
        match self {
            Self::None => Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
            Self::Mac(mac) => Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((b"1".to_vec(), Box::new(Value::Binary(mac.to_vec()))))),
            ]),
            Self::Time(time) => Value::Array(vec![
                Value::Unsigned(2),
                Value::Variant(Some((b"2".to_vec(), Box::new(Value::Unsigned(time))))),
            ]),
        }
    }
}

/// Encodes one complete, length-prefixed v0.1.9 `Probe.probe` call.
pub fn encode_probe_request(
    hostname: &str,
    hostchain_last_seqno: u64,
    host_id: Option<&[u8]>,
) -> Result<Vec<u8>> {
    if hostname.is_empty() || !hostname.is_ascii() || hostname.bytes().any(|byte| byte == 0) {
        return Err(Error::Hostname);
    }

    let probe_arg = encode(&Value::Array(vec![
        Value::Text(hostname.as_bytes().to_vec()),
        Value::Unsigned(hostchain_last_seqno),
        host_id.map_or(Value::Null, |bytes| Value::Binary(bytes.to_vec())),
    ]))?;

    encode_call(PROBE_PROTOCOL_ID, PROBE_METHOD_POSITION, &probe_arg, 0)
}

/// Encodes one complete FOKS v0.1.9 method call around exact canonical
/// Snowpack argument bytes.
pub fn encode_call(
    protocol_id: u64,
    method_position: u64,
    argument: &[u8],
    sequence: u64,
) -> Result<Vec<u8>> {
    foks_snowpack::validate(argument).map_err(|source| Error::ArgumentSnowpack {
        protocol_id,
        method_position,
        source,
    })?;
    encode_call_with_validated_argument(protocol_id, method_position, argument, sequence)
}

/// Rewrites only the outer RPC sequence number of a framed call.
///
/// The protocol argument bytes are copied verbatim, which preserves signed
/// payloads while allowing an authenticated connection to serve more than one
/// request. The complete input envelope is validated before it is rewritten.
pub fn resequence_call(request: &[u8], sequence: u64, maximum: usize) -> Result<Vec<u8>> {
    let mut framed = std::io::Cursor::new(request);
    let content = read_frame(&mut framed, maximum)?;
    if usize::try_from(framed.position()).ok() != Some(request.len()) {
        return Err(Error::Envelope {
            expected: "one complete RPC call frame",
            found: "trailing data",
        });
    }
    let mut cursor = Cursor::new(&content);
    if cursor.byte()? != 0x95 {
        return Err(Error::Envelope {
            expected: "five-element RPC call array",
            found: "another MessagePack value",
        });
    }
    if unsigned(cursor.value()?)? != METHOD_CALL_V2 {
        return Err(Error::Envelope {
            expected: "RPC call method",
            found: "another RPC method",
        });
    }
    let _old_sequence = unsigned(cursor.value()?)?;
    let suffix_start = cursor.position;
    for _ in 0..3 {
        cursor.value()?;
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of RPC call",
            found: "trailing data",
        });
    }

    let mut rewritten = Vec::with_capacity(content.len() + 9);
    rewritten.push(0x95);
    encode_unsigned(METHOD_CALL_V2, &mut rewritten);
    encode_unsigned(sequence, &mut rewritten);
    rewritten.extend_from_slice(&content[suffix_start..]);
    frame(&rewritten, maximum)
}

fn encode_call_with_validated_argument(
    protocol_id: u64,
    method_position: u64,
    argument: &[u8],
    sequence: u64,
) -> Result<Vec<u8>> {
    let mut content = Vec::with_capacity(argument.len() + 40);
    content.push(0x95); // five-element RPC call
    encode_unsigned(METHOD_CALL_V2, &mut content);
    encode_unsigned(sequence, &mut content);
    encode_unsigned(protocol_id, &mut content);
    encode_unsigned(method_position, &mut content);
    content.push(0x82); // rpc.DataWrap map, canonically ordered by key
    encode_text(b"Data", &mut content);
    content.extend_from_slice(argument);
    encode_text(b"Header", &mut content);
    content.extend_from_slice(RESPONSE_HEADER);

    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

/// Encodes the unauthenticated registration call that obtains an X.509 client
/// certificate chain for an already enrolled device key.
pub fn encode_get_client_cert_chain_request(uid: &[u8], device_id: &[u8]) -> Result<Vec<u8>> {
    encode_get_client_cert_chain_request_at(uid, device_id, 0)
}

pub fn encode_reserve_username_request_at(name: &[u8], sequence: u64) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Text(name.to_vec())]))?;
    encode_call(
        REG_PROTOCOL_ID,
        REG_RESERVE_USERNAME_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_signup_request_at(
    argument: &SoftwareSignupArgument<'_>,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_SIGNUP_METHOD_POSITION,
        &argument.encoded()?,
        sequence,
    )
}

pub fn encode_get_client_cert_chain_request_at(
    uid: &[u8],
    device_id: &[u8],
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        Value::Binary(uid.to_vec()),
        Value::Binary(device_id.to_vec()),
    ]))?;
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_CLIENT_CERT_CHAIN_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_registration_select_vhost_request(host: &EntityId) -> Result<Vec<u8>> {
    encode_select_vhost(REG_PROTOCOL_ID, REG_SELECT_VHOST_METHOD_POSITION, host)
}

/// Encodes an authenticated request for a user's chain, starting at `start`.
pub fn encode_load_user_chain_request(uid: &[u8], start: u64) -> Result<Vec<u8>> {
    encode_load_user_chain_request_from(uid, start, None)
}

pub fn encode_load_user_chain_request_from(
    uid: &[u8],
    start: u64,
    current_name: Option<(&[u8], u64)>,
) -> Result<Vec<u8>> {
    let as_local_user = Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]);
    let name = current_name.map_or(Value::Null, |(name, next_sequence)| {
        Value::Array(vec![
            Value::Text(name.to_vec()),
            Value::Unsigned(next_sequence),
        ])
    });
    let argument = encode(&Value::Array(vec![Value::Array(vec![
        Value::Binary(uid.to_vec()),
        Value::Unsigned(start),
        name,
        as_local_user,
    ])]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_LOAD_USER_CHAIN_METHOD_POSITION,
        &argument,
        0,
    )
}

/// Encodes an authenticated request for the owner-role PUK parcel addressed
/// to `device_id`.
pub fn encode_get_owner_puk_request(device_id: &[u8]) -> Result<Vec<u8>> {
    encode_get_puk_for_role_request(Role::OWNER, device_id)
}

/// Encodes an authenticated request for the current PUK and its historical
/// seed chain at the enrolled device's exact role.
pub fn encode_get_puk_for_role_request(role: Role, device_id: &[u8]) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        role.to_value(),
        Value::Binary(device_id.to_vec()),
    ]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_GET_PUK_FOR_ROLE_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_provision_device_request(argument: &ProvisionDeviceArgument<'_>) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_PROVISION_DEVICE_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_get_uid_lookup_challenge_request(entity: &EntityId) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_UID_LOOKUP_CHALLENGE_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(
            entity.as_bytes().to_vec(),
        )]))?,
        0,
    )
}

pub fn encode_lookup_uid_by_device_request(
    entity: &EntityId,
    challenge: &RegistrationChallenge,
    signature: &Signature,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_LOOKUP_UID_BY_DEVICE_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(entity.as_bytes().to_vec()),
            decode(&challenge.encoded()?)?,
            signature.to_value(),
        ]))?,
        0,
    )
}

pub fn encode_revoke_device_request(argument: &RevokeDeviceArgument<'_>) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_REVOKE_DEVICE_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_create_adhoc_team_request(argument: &AdHocTeamCreateArgument<'_>) -> Result<Vec<u8>> {
    encode_call(TEAM_ADMIN_PROTOCOL_ID, 15, &argument.encoded()?, 0)
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

pub fn encode_get_host_config_request() -> Result<Vec<u8>> {
    // A generated zero-field Snowpack struct is the one sanctioned empty
    // array in the v0.1.9 RPC schema. It is not a general Snowpack Value.
    encode_call_with_validated_argument(
        USER_PROTOCOL_ID,
        USER_GET_HOST_CONFIG_METHOD_POSITION,
        &[0x90],
        0,
    )
}

pub fn encode_get_current_merkle_root_request(host: &EntityId, sequence: u64) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Binary(host.as_bytes().to_vec())]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_CURRENT_ROOT_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_get_historical_merkle_roots_request(
    host: &EntityId,
    full_epochs: &[u64],
    hash_epochs: &[u64],
    sequence: u64,
) -> Result<Vec<u8>> {
    let list = |epochs: &[u64]| {
        if epochs.is_empty() {
            Value::Null
        } else {
            Value::Array(epochs.iter().copied().map(Value::Unsigned).collect())
        }
    };
    let argument = encode(&Value::Array(vec![
        Value::Binary(host.as_bytes().to_vec()),
        list(full_epochs),
        list(hash_epochs),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_HISTORICAL_ROOTS_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_merkle_select_vhost_request(host: &EntityId) -> Result<Vec<u8>> {
    encode_select_vhost(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_SELECT_VHOST_METHOD_POSITION,
        host,
    )
}

pub fn encode_team_view_challenge_request(request: &TeamViewRequest) -> Result<Vec<u8>> {
    encode_call(
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_GET_VIEW_CHALLENGE_METHOD_POSITION,
        &request.encoded()?,
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
    let token = Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((
            b"0".to_vec(),
            Box::new(Value::Binary(token.to_vec())),
        ))),
    ]);
    let argument = encode(&Value::Array(vec![
        Value::Array(vec![
            Value::Binary(team.as_bytes().to_vec()),
            Value::Binary(host.as_bytes().to_vec()),
        ]),
        token,
        Value::Unsigned(start),
        current_name.map_or(Value::Null, |(name, next_sequence)| {
            Value::Array(vec![
                Value::Text(name.to_vec()),
                Value::Unsigned(next_sequence),
            ])
        }),
        Value::Null,
        Value::Bool(false),
        Value::Bool(false),
    ]))?;
    encode_call(
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_LOAD_CHAIN_METHOD_POSITION,
        &argument,
        0,
    )
}

/// Starts a KV connection by selecting the host committed by the pinned host
/// chain. Subsequent calls on the same connection use sequence number 1.
pub fn encode_kv_select_vhost_request(host: &EntityId) -> Result<Vec<u8>> {
    encode_select_vhost(KV_STORE_PROTOCOL_ID, KV_SELECT_VHOST_METHOD_POSITION, host)
}

fn encode_select_vhost(protocol: u64, method: u64, host: &EntityId) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Binary(host.as_bytes().to_vec())]))?;
    encode_call(protocol, method, &argument, 0)
}

pub fn encode_kv_get_root_request(auth: KvAuth<'_>) -> Result<Vec<u8>> {
    encode_kv_get_root_request_at(auth, 1)
}

pub fn encode_kv_mkdir_request_at(
    auth: KvAuth<'_>,
    precondition: Option<&KvPathVersionVector>,
    directory: &KvDirectory,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_MKDIR_METHOD_POSITION,
        Value::Array(vec![
            kv_request_header(auth, precondition),
            directory.to_value(),
        ]),
        sequence,
    )
}

pub fn encode_kv_put_request_at(
    auth: KvAuth<'_>,
    precondition: Option<&KvPathVersionVector>,
    dirents: &[KvDirent],
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_PUT_METHOD_POSITION,
        Value::Array(vec![
            kv_request_header(auth, precondition),
            Value::Array(dirents.iter().map(KvDirent::to_value).collect()),
        ]),
        sequence,
    )
}

pub fn encode_kv_put_root_request_at(
    auth: KvAuth<'_>,
    root: &foks_proto::KvRoot,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_PUT_ROOT_METHOD_POSITION,
        Value::Array(vec![auth.to_value(), root.to_value()]),
        sequence,
    )
}

pub fn encode_kv_file_upload_init_request_at(
    auth: KvAuth<'_>,
    file_id: [u8; 16],
    metadata: &KvLargeFileMetadata,
    chunk: &KvUploadChunk,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_FILE_UPLOAD_INIT_METHOD_POSITION,
        Value::Array(vec![
            auth.to_value(),
            Value::Binary(file_id.to_vec()),
            metadata.to_value(),
            chunk.to_value(),
        ]),
        sequence,
    )
}

pub fn encode_kv_file_upload_chunk_request_at(
    auth: KvAuth<'_>,
    file_id: [u8; 16],
    chunk: &KvUploadChunk,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_FILE_UPLOAD_CHUNK_METHOD_POSITION,
        Value::Array(vec![
            auth.to_value(),
            Value::Binary(file_id.to_vec()),
            chunk.to_value(),
        ]),
        sequence,
    )
}

pub fn encode_kv_put_small_file_or_symlink_request_at(
    auth: KvAuth<'_>,
    id: KvNodeId,
    boxed: &KvSmallFileBox,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_PUT_SMALL_FILE_OR_SYMLINK_METHOD_POSITION,
        Value::Array(vec![auth.to_value(), id.to_value(), boxed.to_value()]),
        sequence,
    )
}

pub fn encode_kv_lock_acquire_request_at(
    auth: KvAuth<'_>,
    parent: [u8; 16],
    dirent: [u8; 16],
    lock_id: [u8; 16],
    timeout_millis: u64,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_lock_request_at(
        KV_LOCK_ACQUIRE_METHOD_POSITION,
        auth,
        parent,
        dirent,
        lock_id,
        Some(timeout_millis),
        sequence,
    )
}

pub fn encode_kv_lock_release_request_at(
    auth: KvAuth<'_>,
    parent: [u8; 16],
    dirent: [u8; 16],
    lock_id: [u8; 16],
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_lock_request_at(
        KV_LOCK_RELEASE_METHOD_POSITION,
        auth,
        parent,
        dirent,
        lock_id,
        None,
        sequence,
    )
}

#[allow(clippy::too_many_arguments)]
fn encode_kv_lock_request_at(
    method: u64,
    auth: KvAuth<'_>,
    parent: [u8; 16],
    dirent: [u8; 16],
    lock_id: [u8; 16],
    timeout_millis: Option<u64>,
    sequence: u64,
) -> Result<Vec<u8>> {
    let lock = Value::Array(vec![
        Value::Array(vec![
            Value::Binary(parent.to_vec()),
            Value::Binary(dirent.to_vec()),
        ]),
        Value::Binary(lock_id.to_vec()),
    ]);
    let mut fields = vec![auth.to_value(), lock];
    if let Some(timeout) = timeout_millis {
        fields.push(Value::Unsigned(timeout));
    }
    encode_kv_call_at(method, Value::Array(fields), sequence)
}

pub fn encode_kv_get_root_request_at(auth: KvAuth<'_>, sequence: u64) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_GET_ROOT_METHOD_POSITION,
        Value::Array(vec![auth.to_value()]),
        sequence,
    )
}

pub fn encode_kv_get_dir_request(auth: KvAuth<'_>, directory: &[u8; 16]) -> Result<Vec<u8>> {
    encode_kv_get_dir_request_at(auth, directory, 1)
}

pub fn encode_kv_get_dir_request_at(
    auth: KvAuth<'_>,
    directory: &[u8; 16],
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_GET_DIR_METHOD_POSITION,
        Value::Array(vec![auth.to_value(), Value::Binary(directory.to_vec())]),
        sequence,
    )
}

pub fn encode_kv_list_request(
    auth: KvAuth<'_>,
    directory: &[u8; 16],
    cursor: KvListCursor,
    number: u64,
    load_small_files: bool,
) -> Result<Vec<u8>> {
    encode_kv_list_request_at(auth, directory, cursor, number, load_small_files, 1)
}

pub fn encode_kv_list_request_at(
    auth: KvAuth<'_>,
    directory: &[u8; 16],
    cursor: KvListCursor,
    number: u64,
    load_small_files: bool,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_LIST_METHOD_POSITION,
        Value::Array(vec![
            auth.to_value(),
            Value::Binary(directory.to_vec()),
            Value::Array(vec![
                cursor.to_value(),
                Value::Unsigned(number),
                Value::Bool(load_small_files),
            ]),
        ]),
        sequence,
    )
}

pub fn encode_kv_get_node_request(auth: KvAuth<'_>, node: KvNodeId) -> Result<Vec<u8>> {
    encode_kv_get_node_request_at(auth, node, 1)
}

pub fn encode_kv_get_node_request_at(
    auth: KvAuth<'_>,
    node: KvNodeId,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_GET_NODE_METHOD_POSITION,
        Value::Array(vec![auth.to_value(), node.to_value()]),
        sequence,
    )
}

pub fn encode_kv_get_encrypted_chunk_request(
    auth: KvAuth<'_>,
    file: KvNodeId,
    offset: u64,
) -> Result<Vec<u8>> {
    encode_kv_get_encrypted_chunk_request_at(auth, file, offset, 1)
}

pub fn encode_kv_get_encrypted_chunk_request_at(
    auth: KvAuth<'_>,
    file: KvNodeId,
    offset: u64,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_GET_ENCRYPTED_CHUNK_METHOD_POSITION,
        Value::Array(vec![
            auth.to_value(),
            Value::Binary(file.object_id().to_vec()),
            Value::Unsigned(offset),
        ]),
        sequence,
    )
}

pub fn encode_kv_cache_check_request(
    auth: KvAuth<'_>,
    versions: &KvPathVersionVector,
) -> Result<Vec<u8>> {
    encode_kv_cache_check_request_at(auth, versions, 1)
}

pub fn encode_kv_cache_check_request_at(
    auth: KvAuth<'_>,
    versions: &KvPathVersionVector,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_CACHE_CHECK_METHOD_POSITION,
        Value::Array(vec![Value::Array(vec![
            auth.to_value(),
            versions.to_value(),
        ])]),
        sequence,
    )
}

fn encode_kv_call_at(method: u64, argument: Value, sequence: u64) -> Result<Vec<u8>> {
    encode_call(KV_STORE_PROTOCOL_ID, method, &encode(&argument)?, sequence)
}

/// Writes one probe call and flushes it before waiting for the response.
pub fn write_probe_request<W: Write>(
    writer: &mut W,
    hostname: &str,
    hostchain_last_seqno: u64,
    host_id: Option<&[u8]>,
) -> Result<()> {
    writer.write_all(&encode_probe_request(
        hostname,
        hostchain_last_seqno,
        host_id,
    )?)?;
    writer.flush()?;
    Ok(())
}

/// Reads and decodes one response, returning exact canonical `ProbeRes` bytes.
pub fn read_probe_response<R: Read>(reader: &mut R, maximum: usize) -> Result<Vec<u8>> {
    read_response(reader, maximum, 0)
}

/// Reads and decodes one generic v0.1.9 response.
pub fn read_response<R: Read>(
    reader: &mut R,
    maximum: usize,
    expected_sequence: u64,
) -> Result<Vec<u8>> {
    let content = read_frame(reader, maximum)?;
    decode_response(&content, expected_sequence)
}

/// Reads a successful RPC response for a method with no return value.
pub fn read_void_response<R: Read>(
    reader: &mut R,
    maximum: usize,
    expected_sequence: u64,
) -> Result<()> {
    let content = read_frame(reader, maximum)?;
    decode_void_response(&content, expected_sequence)
}

/// Reads one MessagePack-length-prefixed RPC frame.
pub fn read_frame<R: Read>(reader: &mut R, maximum: usize) -> Result<Vec<u8>> {
    let length = read_frame_length(reader)?;
    if length > maximum {
        return Err(Error::FrameTooLarge {
            received: length,
            maximum,
        });
    }
    let mut content = vec![0; length];
    reader.read_exact(&mut content)?;
    Ok(content)
}

/// Decodes an unframed RPC response for the given sequence number.
pub fn decode_probe_response(content: &[u8], expected_sequence: u64) -> Result<Vec<u8>> {
    decode_response(content, expected_sequence)
}

/// Decodes an unframed v0.1.9 response and returns its exact canonical result.
pub fn decode_response(content: &[u8], expected_sequence: u64) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(content);
    if cursor.byte()? != 0x94 {
        return Err(Error::Envelope {
            expected: "four-element RPC response array",
            found: "another MessagePack value",
        });
    }
    let method = unsigned(cursor.value()?)?;
    if method != METHOD_RESPONSE {
        return Err(Error::Envelope {
            expected: "RPC response method",
            found: "another RPC method",
        });
    }
    let sequence = unsigned(cursor.value()?)?;
    if sequence != expected_sequence {
        return Err(Error::Sequence {
            expected: expected_sequence,
            received: sequence,
        });
    }
    check_status(cursor.value()?)?;
    let wrapped = cursor.value()?;
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of RPC response",
            found: "trailing data",
        });
    }
    decode_data_wrap(wrapped)
}

pub fn decode_void_response(content: &[u8], expected_sequence: u64) -> Result<()> {
    let mut cursor = Cursor::new(content);
    if cursor.byte()? != 0x94 {
        return Err(Error::Envelope {
            expected: "four-element RPC response array",
            found: "another MessagePack value",
        });
    }
    if unsigned(cursor.value()?)? != METHOD_RESPONSE {
        return Err(Error::Envelope {
            expected: "RPC response method",
            found: "another RPC method",
        });
    }
    let sequence = unsigned(cursor.value()?)?;
    if sequence != expected_sequence {
        return Err(Error::Sequence {
            expected: expected_sequence,
            received: sequence,
        });
    }
    check_status(cursor.value()?)?;
    let wrapped = cursor.value()?;
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of RPC response",
            found: "trailing data",
        });
    }
    let mut wrapped = Cursor::new(wrapped);
    match wrapped.byte()? {
        0x81 => {}
        0x82 => {
            if text(wrapped.value()?)? != b"Data" || decode(wrapped.value()?)? != Value::Null {
                return Err(Error::Envelope {
                    expected: "nil void response data",
                    found: "another response value",
                });
            }
        }
        _ => {
            return Err(Error::Envelope {
                expected: "void RPC data wrapper",
                found: "another MessagePack value",
            });
        }
    }
    if text(wrapped.value()?)? != b"Header" || wrapped.value()? != RESPONSE_HEADER {
        return Err(Error::Compatibility);
    }
    if !wrapped.done() {
        return Err(Error::Envelope {
            expected: "end of void RPC data wrapper",
            found: "trailing data",
        });
    }
    Ok(())
}

/// Encodes a successful response for protocol fixtures and local test servers.
#[doc(hidden)]
pub fn encode_probe_success_response(probe_response: &[u8]) -> Result<Vec<u8>> {
    encode_success_response_at(probe_response, 0)
}

#[doc(hidden)]
pub fn encode_success_response_at(response: &[u8], sequence: u64) -> Result<Vec<u8>> {
    foks_snowpack::validate(response)?;
    let mut content = Vec::with_capacity(response.len() + 32);
    content.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut content);
    encode_unsigned(sequence, &mut content);
    // The RPC layer encodes a nil error pointer on success. A concrete FOKS
    // Status value is present only when the server returns an application
    // error.
    content.push(0xc0);
    content.push(0x82);
    encode_text(b"Data", &mut content);
    content.extend_from_slice(response);
    encode_text(b"Header", &mut content);
    content.extend_from_slice(RESPONSE_HEADER);
    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

fn decode_data_wrap(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(bytes);
    if cursor.byte()? != 0x82 {
        return Err(Error::Envelope {
            expected: "two-field RPC data wrapper",
            found: "another MessagePack value",
        });
    }
    let data_key = text(cursor.value()?)?;
    if data_key != b"Data" {
        return Err(Error::Envelope {
            expected: "canonical Data field",
            found: "another field",
        });
    }
    let data = cursor.value()?.to_vec();
    let header_key = text(cursor.value()?)?;
    if header_key != b"Header" {
        return Err(Error::Envelope {
            expected: "canonical Header field",
            found: "another field",
        });
    }
    let header = cursor.value()?;
    if header != RESPONSE_HEADER {
        return Err(Error::Compatibility);
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of RPC data wrapper",
            found: "trailing data",
        });
    }
    // RPC results are generated MessagePack structs, not authenticated
    // Snowpack values as a whole. In particular, v0.1.9 can emit sanctioned
    // zero-field structs as empty arrays. Type-specific decoders and
    // verifiers still canonicalize every signed or MACed inner object.
    Ok(data)
}

fn check_status(bytes: &[u8]) -> Result<()> {
    if bytes == [0xc0] {
        return Ok(());
    }
    if matches!(bytes.first(), Some(0x92)) {
        return check_positional_status(bytes);
    }
    // RPC status values are encoded by the Go RPC codec as named MessagePack
    // structs, not as canonical positional Snowpack values. For example a
    // stale-cache status is `{Sc: 8012, f11: {Root, Path}}`.
    let mut cursor = Cursor::new(bytes);
    let fields = map_length(&mut cursor)?;
    if !(1..=2).contains(&fields) {
        return Err(Error::Envelope {
            expected: "one- or two-field FOKS status",
            found: "another map length",
        });
    }
    let mut code = None;
    let mut payload = None;
    for _ in 0..fields {
        let key = text(cursor.value()?)?;
        let value = cursor.value()?;
        if key == b"Sc" {
            if code.replace(unsigned(value)?).is_some() {
                return Err(Error::Envelope {
                    expected: "one FOKS status code",
                    found: "duplicate status code",
                });
            }
        } else if payload.replace((key, value)).is_some() {
            return Err(Error::Envelope {
                expected: "one FOKS status payload",
                found: "duplicate status payload",
            });
        }
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of FOKS status",
            found: "trailing data",
        });
    }
    let code = code.ok_or(Error::Envelope {
        expected: "FOKS status code",
        found: "status without Sc",
    })?;
    if code == 0 && payload.is_none() {
        return Ok(());
    }
    if code == 0 {
        return Err(Error::Envelope {
            expected: "payload-free successful FOKS status",
            found: "successful status with a payload",
        });
    }
    if code == 8012 {
        let Some((tag, value)) = payload else {
            return Err(Error::Envelope {
                expected: "KV stale-cache status payload",
                found: "missing status payload",
            });
        };
        if tag != b"f11" {
            return Err(Error::Envelope {
                expected: "KV stale-cache status field f11",
                found: "another status variant",
            });
        }
        return Err(Error::KvStaleCache(named_path_version_vector(value)?));
    }
    let detail = match payload {
        Some((_, value)) => match decode(value) {
            Ok(Value::Text(bytes)) => String::from_utf8(bytes).ok(),
            Ok(other) => Some(format!("{other:?}")),
            Err(_) => None,
        },
        None => None,
    };
    Err(Error::RemoteStatus {
        code,
        detail: StatusDetail(detail),
    })
}

fn check_positional_status(bytes: &[u8]) -> Result<()> {
    let value = decode(bytes)?;
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "two-field FOKS Status",
            found: value.kind(),
        });
    };
    if fields.len() != 2 {
        return Err(Error::Envelope {
            expected: "two-field FOKS Status",
            found: "another array length",
        });
    }
    let Value::Unsigned(code) = fields[0] else {
        return Err(Error::Envelope {
            expected: "unsigned FOKS status code",
            found: fields[0].kind(),
        });
    };
    if code == 0 && fields[1] == Value::Variant(None) {
        return Ok(());
    }
    let detail = match &fields[1] {
        Value::Variant(Some((_, value))) => match value.as_ref() {
            Value::Text(bytes) => String::from_utf8(bytes.clone()).ok(),
            other => Some(format!("{other:?}")),
        },
        _ => None,
    };
    Err(Error::RemoteStatus {
        code,
        detail: StatusDetail(detail),
    })
}

fn named_path_version_vector(bytes: &[u8]) -> Result<KvPathVersionVector> {
    let mut cursor = Cursor::new(bytes);
    let fields = map_length(&mut cursor)?;
    if fields != 2 {
        return Err(Error::Envelope {
            expected: "two-field PathVersionVector",
            found: "another map length",
        });
    }
    let mut root_version = None;
    let mut directories = None;
    for _ in 0..fields {
        match text(cursor.value()?)?.as_slice() {
            b"Root" if root_version.is_none() => root_version = Some(unsigned(cursor.value()?)?),
            b"Path" if directories.is_none() => {
                directories = Some(named_directory_versions(cursor.value()?)?)
            }
            _ => {
                return Err(Error::Envelope {
                    expected: "PathVersionVector field",
                    found: "unknown field",
                });
            }
        }
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of PathVersionVector",
            found: "trailing data",
        });
    }
    Ok(KvPathVersionVector {
        root_version: root_version.ok_or(Error::Envelope {
            expected: "PathVersionVector Root",
            found: "missing field",
        })?,
        directories: directories.ok_or(Error::Envelope {
            expected: "PathVersionVector Path",
            found: "missing field",
        })?,
    })
}

fn named_directory_versions(bytes: &[u8]) -> Result<Vec<foks_proto::KvDirectoryVersion>> {
    let mut cursor = Cursor::new(bytes);
    let length = array_length(&mut cursor)?;
    let mut output = Vec::with_capacity(length);
    for _ in 0..length {
        let value = cursor.value()?;
        let mut fields = Cursor::new(value);
        let count = map_length(&mut fields)?;
        if count != 3 {
            return Err(Error::Envelope {
                expected: "three-field DirVersion",
                found: "another map length",
            });
        }
        let mut id = None;
        let mut version = None;
        let mut entries = None;
        for _ in 0..count {
            match text(fields.value()?)?.as_slice() {
                b"Id" if id.is_none() => id = Some(fixed_16(fields.value()?)?),
                b"Vers" if version.is_none() => version = Some(unsigned(fields.value()?)?),
                b"De" if entries.is_none() => {
                    entries = Some(named_dirent_versions(fields.value()?)?)
                }
                _ => {
                    return Err(Error::Envelope {
                        expected: "DirVersion field",
                        found: "unknown field",
                    });
                }
            }
        }
        if !fields.done() {
            return Err(Error::Envelope {
                expected: "end of DirVersion",
                found: "trailing data",
            });
        }
        output.push(foks_proto::KvDirectoryVersion {
            id: id.ok_or(Error::Envelope {
                expected: "DirVersion Id",
                found: "missing field",
            })?,
            version: version.ok_or(Error::Envelope {
                expected: "DirVersion Vers",
                found: "missing field",
            })?,
            entries: entries.ok_or(Error::Envelope {
                expected: "DirVersion De",
                found: "missing field",
            })?,
        });
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of DirVersion list",
            found: "trailing data",
        });
    }
    Ok(output)
}

fn named_dirent_versions(bytes: &[u8]) -> Result<Vec<foks_proto::KvDirentVersion>> {
    let mut cursor = Cursor::new(bytes);
    let length = array_length(&mut cursor)?;
    let mut output = Vec::with_capacity(length);
    for _ in 0..length {
        let value = cursor.value()?;
        let mut fields = Cursor::new(value);
        let count = map_length(&mut fields)?;
        if count != 2 {
            return Err(Error::Envelope {
                expected: "two-field DirentVersion",
                found: "another map length",
            });
        }
        let mut id = None;
        let mut version = None;
        for _ in 0..count {
            match text(fields.value()?)?.as_slice() {
                b"Id" if id.is_none() => id = Some(fixed_16(fields.value()?)?),
                b"Vers" if version.is_none() => version = Some(unsigned(fields.value()?)?),
                _ => {
                    return Err(Error::Envelope {
                        expected: "DirentVersion field",
                        found: "unknown field",
                    });
                }
            }
        }
        if !fields.done() {
            return Err(Error::Envelope {
                expected: "end of DirentVersion",
                found: "trailing data",
            });
        }
        output.push(foks_proto::KvDirentVersion {
            id: id.ok_or(Error::Envelope {
                expected: "DirentVersion Id",
                found: "missing field",
            })?,
            version: version.ok_or(Error::Envelope {
                expected: "DirentVersion Vers",
                found: "missing field",
            })?,
        });
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of DirentVersion list",
            found: "trailing data",
        });
    }
    Ok(output)
}

fn fixed_16(bytes: &[u8]) -> Result<[u8; 16]> {
    match decode(bytes)? {
        Value::Binary(bytes) => bytes.try_into().map_err(|_| Error::Envelope {
            expected: "16-byte identifier",
            found: "another binary length",
        }),
        other => Err(Error::Envelope {
            expected: "binary identifier",
            found: other.kind(),
        }),
    }
}

fn map_length(cursor: &mut Cursor<'_>) -> Result<usize> {
    match cursor.byte()? {
        marker @ 0x80..=0x8f => Ok(usize::from(marker & 0x0f)),
        0xde => Ok(usize::from(cursor.number_u16()?)),
        0xdf => usize::try_from(cursor.number_u32()?).map_err(|_| Error::Truncated),
        _ => Err(Error::Envelope {
            expected: "MessagePack map",
            found: "another MessagePack value",
        }),
    }
}

fn array_length(cursor: &mut Cursor<'_>) -> Result<usize> {
    match cursor.byte()? {
        0xc0 => Ok(0),
        marker @ 0x90..=0x9f => Ok(usize::from(marker & 0x0f)),
        0xdc => Ok(usize::from(cursor.number_u16()?)),
        0xdd => usize::try_from(cursor.number_u32()?).map_err(|_| Error::Truncated),
        _ => Err(Error::Envelope {
            expected: "MessagePack array",
            found: "another MessagePack value",
        }),
    }
}

fn unsigned(bytes: &[u8]) -> Result<u64> {
    match decode(bytes)? {
        Value::Unsigned(value) => Ok(value),
        value => Err(Error::Envelope {
            expected: "unsigned integer",
            found: value.kind(),
        }),
    }
}

fn text(bytes: &[u8]) -> Result<Vec<u8>> {
    match decode(bytes)? {
        Value::Text(value) => Ok(value),
        value => Err(Error::Envelope {
            expected: "text key",
            found: value.kind(),
        }),
    }
}

fn frame(content: &[u8], maximum: usize) -> Result<Vec<u8>> {
    if content.len() > maximum {
        return Err(Error::FrameTooLarge {
            received: content.len(),
            maximum,
        });
    }
    let mut output = Vec::with_capacity(content.len() + 5);
    encode_unsigned(content.len() as u64, &mut output);
    output.extend_from_slice(content);
    Ok(output)
}

fn read_frame_length<R: Read>(reader: &mut R) -> Result<usize> {
    let marker = read_byte(reader)?;
    let value = match marker {
        0x00..=0x7f => u64::from(marker),
        0xcc => {
            let value = u64::from(read_byte(reader)?);
            if value <= 0x7f {
                return Err(Error::FrameLengthMarker(marker));
            }
            value
        }
        0xcd => {
            let value = u64::from(read_u16(reader)?);
            if value <= u64::from(u8::MAX) {
                return Err(Error::FrameLengthMarker(marker));
            }
            value
        }
        0xce => {
            let value = u64::from(read_u32(reader)?);
            if value <= u64::from(u16::MAX) {
                return Err(Error::FrameLengthMarker(marker));
            }
            value
        }
        _ => return Err(Error::FrameLengthMarker(marker)),
    };
    usize::try_from(value).map_err(|_| Error::FrameTooLarge {
        received: usize::MAX,
        maximum: usize::MAX,
    })
}

fn read_byte<R: Read>(reader: &mut R) -> std::io::Result<u8> {
    let mut byte = [0];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn read_u16<R: Read>(reader: &mut R) -> std::io::Result<u16> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32<R: Read>(reader: &mut R) -> std::io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn encode_unsigned(value: u64, output: &mut Vec<u8>) {
    match value {
        0..=0x7f => output.push(value as u8),
        0x80..=0xff => {
            output.push(0xcc);
            output.push(value as u8);
        }
        0x100..=0xffff => {
            output.push(0xcd);
            output.extend_from_slice(&(value as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            output.push(0xce);
            output.extend_from_slice(&(value as u32).to_be_bytes());
        }
        _ => {
            output.push(0xcf);
            output.extend_from_slice(&value.to_be_bytes());
        }
    }
}

fn encode_text(value: &[u8], output: &mut Vec<u8>) {
    let length = value.len();
    if length <= 31 {
        output.push(0xa0 | length as u8);
    } else if let Ok(length) = u8::try_from(length) {
        output.push(0xd9);
        output.push(length);
    } else if let Ok(length) = u16::try_from(length) {
        output.push(0xda);
        output.extend_from_slice(&length.to_be_bytes());
    } else {
        output.push(0xdb);
        output.extend_from_slice(&(length as u32).to_be_bytes());
    }
    output.extend_from_slice(value);
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn done(&self) -> bool {
        self.position == self.bytes.len()
    }

    fn byte(&mut self) -> Result<u8> {
        let byte = *self.bytes.get(self.position).ok_or(Error::Truncated)?;
        self.position += 1;
        Ok(byte)
    }

    fn take(&mut self, length: usize) -> Result<()> {
        self.position = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(Error::Truncated)?;
        Ok(())
    }

    fn value(&mut self) -> Result<&'a [u8]> {
        let start = self.position;
        self.skip(0)?;
        Ok(&self.bytes[start..self.position])
    }

    fn skip(&mut self, depth: usize) -> Result<()> {
        if depth > foks_snowpack::MAX_DEPTH {
            return Err(Error::Depth);
        }
        let marker = self.byte()?;
        match marker {
            0x00..=0x7f | 0xc0 | 0xc2 | 0xc3 | 0xe0..=0xff => {}
            0x80..=0x8f => self.skip_many(usize::from(marker & 0x0f) * 2, depth)?,
            0x90..=0x9f => self.skip_many(usize::from(marker & 0x0f), depth)?,
            0xa0..=0xbf => self.take(usize::from(marker & 0x1f))?,
            0xc4 | 0xd9 => {
                let length = usize::from(self.byte()?);
                self.take(length)?;
            }
            0xc5 | 0xda => {
                let length = usize::from(self.number_u16()?);
                self.take(length)?;
            }
            0xc6 | 0xdb => {
                let length = usize::try_from(self.number_u32()?).map_err(|_| Error::Truncated)?;
                self.take(length)?;
            }
            0xc7 => {
                let length = usize::from(self.byte()?);
                self.take(length.checked_add(1).ok_or(Error::Truncated)?)?;
            }
            0xc8 => {
                let length = usize::from(self.number_u16()?);
                self.take(length.checked_add(1).ok_or(Error::Truncated)?)?;
            }
            0xc9 => {
                let length = usize::try_from(self.number_u32()?).map_err(|_| Error::Truncated)?;
                self.take(length.checked_add(1).ok_or(Error::Truncated)?)?;
            }
            0xca => self.take(4)?,
            0xcb => self.take(8)?,
            0xcc | 0xd0 => self.take(1)?,
            0xcd | 0xd1 => self.take(2)?,
            0xce | 0xd2 => self.take(4)?,
            0xcf | 0xd3 => self.take(8)?,
            0xd4 => self.take(2)?,
            0xd5 => self.take(3)?,
            0xd6 => self.take(5)?,
            0xd7 => self.take(9)?,
            0xd8 => self.take(17)?,
            0xdc => {
                let length = usize::from(self.number_u16()?);
                self.skip_many(length, depth)?;
            }
            0xdd => {
                let length = usize::try_from(self.number_u32()?).map_err(|_| Error::Truncated)?;
                self.skip_many(length, depth)?;
            }
            0xde => {
                let length = usize::from(self.number_u16()?);
                self.skip_many(length.checked_mul(2).ok_or(Error::Truncated)?, depth)?;
            }
            0xdf => {
                let length = usize::try_from(self.number_u32()?).map_err(|_| Error::Truncated)?;
                self.skip_many(length.checked_mul(2).ok_or(Error::Truncated)?, depth)?;
            }
            0xc1 => return Err(Error::Marker(marker)),
        }
        Ok(())
    }

    fn skip_many(&mut self, count: usize, depth: usize) -> Result<()> {
        if count > self.bytes.len().saturating_sub(self.position) {
            return Err(Error::Truncated);
        }
        for _ in 0..count {
            self.skip(depth + 1)?;
        }
        Ok(())
    }

    fn number_u16(&mut self) -> Result<u16> {
        let end = self.position.checked_add(2).ok_or(Error::Truncated)?;
        let bytes: [u8; 2] = self
            .bytes
            .get(self.position..end)
            .ok_or(Error::Truncated)?
            .try_into()
            .map_err(|_| Error::Truncated)?;
        self.position = end;
        Ok(u16::from_be_bytes(bytes))
    }

    fn number_u32(&mut self) -> Result<u32> {
        let end = self.position.checked_add(4).ok_or(Error::Truncated)?;
        let bytes: [u8; 4] = self
            .bytes
            .get(self.position..end)
            .ok_or(Error::Truncated)?
            .try_into()
            .map_err(|_| Error::Truncated)?;
        self.position = end;
        Ok(u32::from_be_bytes(bytes))
    }
}

trait ValueKind {
    fn kind(&self) -> &'static str;
}

impl ValueKind for Value {
    fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Unsigned(_) => "unsigned integer",
            Self::Negative(_) => "negative integer",
            Self::Binary(_) => "binary",
            Self::Text(_) => "text",
            Self::Array(_) => "array",
            Self::Variant(_) => "map",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        check_status, encode_call, read_frame, resequence_call, unsigned, Cursor, Error,
        METHOD_CALL_V2,
    };

    #[test]
    fn successful_named_status_must_not_hide_a_payload() {
        // {"Sc": 0, "f11": nil}
        let malformed = [0x82, 0xa2, b'S', b'c', 0x00, 0xa3, b'f', b'1', b'1', 0xc0];
        assert!(matches!(
            check_status(&malformed),
            Err(Error::Envelope { .. })
        ));

        // A concrete zero status without a union payload remains accepted,
        // though the canonical RPC success representation is nil.
        let zero = [0x81, 0xa2, b'S', b'c', 0x00];
        check_status(&zero).unwrap();
    }

    #[test]
    fn resequencing_preserves_the_exact_protocol_argument() {
        let argument = foks_snowpack::encode(&foks_snowpack::Value::Binary(
            b"signed protocol payload".to_vec(),
        ))
        .unwrap();
        let request = encode_call(17, 23, &argument, 0).unwrap();
        let rewritten = resequence_call(&request, 300, 1024).unwrap();
        let content = read_frame(&mut std::io::Cursor::new(&rewritten), 1024).unwrap();
        let mut cursor = Cursor::new(&content);
        assert_eq!(cursor.byte().unwrap(), 0x95);
        assert_eq!(unsigned(cursor.value().unwrap()).unwrap(), METHOD_CALL_V2);
        assert_eq!(unsigned(cursor.value().unwrap()).unwrap(), 300);
        assert!(content
            .windows(argument.len())
            .any(|window| window == argument.as_slice()));

        let mut trailing = request;
        trailing.push(0);
        assert!(matches!(
            resequence_call(&trailing, 1, 1024),
            Err(Error::Envelope { .. })
        ));
    }
}
