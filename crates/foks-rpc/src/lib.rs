//! Strict framing and request encoding for FOKS v0.1.9 client RPCs.
//!
//! Snowpack RPC uses an ordinary MessagePack envelope (including named maps)
//! around canonical Snowpack protocol values. This crate keeps the envelope
//! separate from exact public and authenticated protocol payloads and returns
//! response bytes unchanged for protocol verification.

#![forbid(unsafe_code)]

use std::io::{Read, Write};

use foks_proto::{
    AdHocTeamCreateArgument, AddTeamMemberArgument, ClientVersionExt, EntityId, FqParty, FqTeam,
    InviteCode, KexReceiveArgument, KexSendArgument, KvDirectory, KvDirent, KvLargeFileMetadata,
    KvNodeId, KvPathVersionVector, KvSmallFileBox, KvUploadChunk, NamedTeamCreateArgument,
    PassphraseUpdateArgument, PermissionToken, ProvisionDeviceArgument, RegistrationChallenge,
    RemoteViewPermissionPayload, RemoveTeamMemberArgument, RevokeDeviceArgument, Role,
    RoleAndGeneration, Signature, SoftwareSignupArgument, TeamBearerToken,
    TeamBearerTokenChallenge, TeamEditResult, TeamMetadataEditArgument, TeamNameReservation,
    TeamRemovalKeyBox, TeamViewChallenge, TeamViewRequest, YubiEncryptedManagementKey,
    YubiSignupArgument,
};
use foks_snowpack::{decode, encode, Value};
use thiserror::Error;

pub mod arguments;
mod generated;
mod response;
mod server;

pub use generated::*;
pub use response::{encode_status_response_at, encode_void_success_response_at, RpcStatus};
pub use server::{
    decode_call, decode_message, read_call, read_message, DecodedCall, InboundMessage,
};

pub const CURRENT_COMPATIBILITY_VERSION: u64 = 1;
pub const DEFAULT_MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

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
    #[error("RPC {kind} count {received} exceeds limit {maximum}")]
    CollectionTooLarge {
        kind: &'static str,
        received: usize,
        maximum: usize,
    },
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
            found: "unexpected RPC method",
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

/// Extracts the protocol id from one framed RPC call without copying its
/// argument. The client uses this to select the wrapped or bare response
/// decoder for the protocol it is about to invoke.
pub fn call_protocol_id(framed_call: &[u8], maximum: usize) -> Result<u64> {
    let mut framed = std::io::Cursor::new(framed_call);
    let content = read_frame(&mut framed, maximum)?;
    let mut cursor = Cursor::new(&content);
    match cursor.byte()? {
        0x95 | 0x96 => {}
        _ => {
            return Err(Error::Envelope {
                expected: "five- or six-element RPC call array",
                found: "another MessagePack value",
            })
        }
    }
    if unsigned(cursor.value()?)? != METHOD_CALL_V2 {
        return Err(Error::Envelope {
            expected: "RPC call method",
            found: "another RPC method",
        });
    }
    let _sequence = unsigned(cursor.value()?)?;
    unsigned(cursor.value()?)
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
    if is_headerless_argument_protocol(protocol_id) {
        // Team and Kex protocols carry the bare argument struct with no
        // DataWrap envelope and no header (go-foks proto/rem/team.go, kex.go).
        content.extend_from_slice(argument);
    } else {
        content.push(0x82); // rpc.DataWrap map, canonically ordered by key
        encode_text(b"Data", &mut content);
        content.extend_from_slice(argument);
        encode_text(b"Header", &mut content);
        content.extend_from_slice(RESPONSE_HEADER);
    }

    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

/// Encodes the unauthenticated registration call that obtains an X.509 client
/// certificate chain for an already enrolled device key.
pub fn encode_get_client_cert_chain_request(uid: &[u8], device_id: &[u8]) -> Result<Vec<u8>> {
    encode_get_client_cert_chain_request_at(uid, device_id, 0)
}

pub fn encode_kex_send_request(argument: &KexSendArgument) -> Result<Vec<u8>> {
    encode_call(
        KEX_PROTOCOL_ID,
        KEX_SEND_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_kex_receive_request(argument: &KexReceiveArgument) -> Result<Vec<u8>> {
    encode_call(
        KEX_PROTOCOL_ID,
        KEX_RECEIVE_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
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

pub fn encode_yubi_signup_request_at(
    argument: &YubiSignupArgument<'_>,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_SIGNUP_METHOD_POSITION,
        &argument.encoded()?,
        sequence,
    )
}

pub fn encode_get_subkey_box_challenge_request(parent: &EntityId) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_SUBKEY_BOX_CHALLENGE_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(
            parent.as_bytes().to_vec(),
        )]))?,
        0,
    )
}

pub fn encode_load_subkey_box_request(
    parent: &EntityId,
    challenge: &RegistrationChallenge,
    signature: &Signature,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_LOAD_SUBKEY_BOX_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(parent.as_bytes().to_vec()),
            decode(&challenge.encoded()?)?,
            signature.to_value(),
        ]))?,
        0,
    )
}

pub fn encode_check_invite_code_request(code: &InviteCode) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_CHECK_INVITE_CODE_METHOD_POSITION,
        &encode(&Value::Array(vec![code.to_value()]))?,
        0,
    )
}

pub fn encode_get_login_challenge_request(uid: &EntityId) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_LOGIN_CHALLENGE_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(uid.as_bytes().to_vec())]))?,
        0,
    )
}

pub fn encode_passphrase_login_request(
    uid: &EntityId,
    challenge: &RegistrationChallenge,
    signature: &Signature,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_LOGIN_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(uid.as_bytes().to_vec()),
            decode(&challenge.encoded()?)?,
            signature.to_value(),
        ]))?,
        0,
    )
}

pub fn encode_registration_stretch_version_request() -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_STRETCH_VERSION_METHOD_POSITION,
        &encode(&Value::Null)?,
        0,
    )
}

pub fn encode_registration_server_config_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        REG_PROTOCOL_ID,
        REG_GET_SERVER_CONFIG_METHOD_POSITION,
        &[0x90],
        0,
    )
}

pub fn encode_check_name_exists_request(name: &[u8]) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_CHECK_NAME_EXISTS_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Text(name.to_vec())]))?,
        0,
    )
}

pub fn encode_probe_key_exists_request(
    uid: &EntityId,
    device_id: &EntityId,
    self_token: &PermissionToken,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_PROBE_KEY_EXISTS_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(uid.as_bytes().to_vec()),
            Value::Binary(device_id.as_bytes().to_vec()),
            self_token.to_value(),
        ]))?,
        0,
    )
}

pub fn encode_join_waitlist_request_at(email: &[u8], sequence: u64) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_JOIN_WAIT_LIST_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Text(email.to_vec())]))?,
        sequence,
    )
}

pub fn encode_log_send_init_request_at(sequence: u64) -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        LOG_SEND_PROTOCOL_ID,
        LOG_SEND_INIT_METHOD_POSITION,
        &[0x90],
        sequence,
    )
}

pub fn encode_log_send_init_file_request_at(
    argument: &arguments::LogSendInitFileArgument,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        LOG_SEND_PROTOCOL_ID,
        LOG_SEND_INIT_FILE_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(argument.id.to_vec()),
            Value::Unsigned(argument.file_id),
            Value::Text(argument.filename.clone()),
            Value::Unsigned(argument.content_length),
            Value::Binary(argument.content_hash.to_vec()),
            Value::Unsigned(argument.block_count),
        ]))?,
        sequence,
    )
}

pub fn encode_log_send_upload_block_request_at(
    argument: &arguments::LogSendUploadBlockArgument,
    sequence: u64,
) -> Result<Vec<u8>> {
    if argument.block.len() > arguments::MAXIMUM_LOG_SEND_BLOCK_BYTES {
        return Err(Error::CollectionTooLarge {
            kind: "log-send block byte",
            received: argument.block.len(),
            maximum: arguments::MAXIMUM_LOG_SEND_BLOCK_BYTES,
        });
    }
    encode_call(
        LOG_SEND_PROTOCOL_ID,
        LOG_SEND_UPLOAD_BLOCK_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(argument.id.to_vec()),
            Value::Unsigned(argument.file_id),
            Value::Unsigned(argument.block_number),
            Value::Binary(argument.block.clone()),
        ]))?,
        sequence,
    )
}

pub fn encode_get_client_version_info_request(version: &ClientVersionExt) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![decode(&version.encoded()?)?]))?;
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_CLIENT_VERSION_INFO_METHOD_POSITION,
        &argument,
        0,
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
    encode_load_user_chain_with_authorization(uid, start, current_name, as_local_user)
}

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

/// Encodes the authenticated `OpenVHost` form used to inspect a prospective
/// local member before admitting it to a team. The server still applies the
/// virtual host's public-user-viewership policy.
pub fn encode_load_user_chain_open_host_request(
    uid: &[u8],
    start: u64,
    current_name: Option<(&[u8], u64)>,
) -> Result<Vec<u8>> {
    let open_host = Value::Array(vec![Value::Unsigned(4), Value::Variant(None)]);
    encode_load_user_chain_with_authorization(uid, start, current_name, open_host)
}

fn encode_load_user_chain_with_authorization(
    uid: &[u8],
    start: u64,
    current_name: Option<(&[u8], u64)>,
    authorization: Value,
) -> Result<Vec<u8>> {
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
        authorization,
    ])]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_LOAD_USER_CHAIN_METHOD_POSITION,
        &argument,
        0,
    )
}

/// Encodes the public registration-service form used to load a remote user
/// after the remote user explicitly grants this caller a bearer permission.
pub fn encode_load_remote_user_chain_request(
    uid: &EntityId,
    start: u64,
    current_name: Option<(&[u8], u64)>,
    token: &PermissionToken,
) -> Result<Vec<u8>> {
    uid.clone().require_type(foks_proto::ENTITY_USER)?;
    if start == 0 {
        return Err(foks_proto::Error::IntegerRange("user-chain start").into());
    }
    let argument = arguments::load_user_chain_argument_value(
        uid,
        start,
        current_name,
        arguments::remote_token_authorization(token),
    );
    encode_call(
        REG_PROTOCOL_ID,
        REG_LOAD_USER_CHAIN_METHOD_POSITION,
        &encode(&argument)?,
        0,
    )
}

pub fn encode_beacon_lookup_request(host: &EntityId) -> Result<Vec<u8>> {
    host.clone().require_type(foks_proto::ENTITY_HOST)?;
    encode_call(
        BEACON_PROTOCOL_ID,
        BEACON_LOOKUP_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(host.as_bytes().to_vec())]))?,
        0,
    )
}

pub fn decode_beacon_lookup_response(
    host: EntityId,
    response: &[u8],
) -> Result<foks_proto::BeaconHint> {
    foks_proto::BeaconHint::decode_address(host, response).map_err(Into::into)
}

pub fn encode_grant_remote_view_permission_for_user_request(
    payload: &RemoteViewPermissionPayload,
) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_GRANT_REMOTE_VIEW_PERMISSION_METHOD_POSITION,
        &encode(&Value::Array(vec![payload.to_value()]))?,
        0,
    )
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

pub fn encode_set_passphrase_request(argument: &PassphraseUpdateArgument) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_SET_PASSPHRASE_METHOD_POSITION,
        &argument.encoded_set()?,
        0,
    )
}

pub fn encode_change_passphrase_request(argument: &PassphraseUpdateArgument) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_CHANGE_PASSPHRASE_METHOD_POSITION,
        &argument.encoded_change()?,
        0,
    )
}

fn encode_user_void_request(position: u64) -> Result<Vec<u8>> {
    encode_call(USER_PROTOCOL_ID, position, &encode(&Value::Null)?, 0)
}

pub fn encode_get_passphrase_salt_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_GET_SALT_METHOD_POSITION)
}

pub fn encode_next_passphrase_generation_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_NEXT_PASSPHRASE_GENERATION_METHOD_POSITION)
}

pub fn encode_user_stretch_version_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_STRETCH_VERSION_METHOD_POSITION)
}

pub fn encode_put_yubi_management_key_request(
    value: &YubiEncryptedManagementKey,
) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_PUT_YUBI_MANAGEMENT_KEY_METHOD_POSITION,
        &encode(&Value::Array(vec![value.to_value()?]))?,
        0,
    )
}

pub fn encode_get_yubi_management_key_request(parent: &EntityId) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_GET_YUBI_MANAGEMENT_KEY_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(
            parent.as_bytes().to_vec(),
        )]))?,
        0,
    )
}

pub fn encode_get_all_yubi_management_keys_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_GET_ALL_YUBI_MANAGEMENT_KEYS_METHOD_POSITION)
}

pub fn encode_get_ppe_parcel_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_GET_PPE_PARCEL_METHOD_POSITION)
}

pub fn encode_load_generic_chain_request(
    entity: &EntityId,
    chain_type: u64,
    start: u64,
) -> Result<Vec<u8>> {
    if start == 0
        || !matches!(
            chain_type,
            foks_proto::CHAIN_TYPE_USER_SETTINGS | foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP
        )
    {
        return Err(foks_proto::Error::IntegerRange("generic-chain request").into());
    }
    let argument = encode(&Value::Array(vec![
        Value::Binary(entity.as_bytes().to_vec()),
        Value::Unsigned(chain_type),
        Value::Unsigned(start),
    ]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_LOAD_GENERIC_CHAIN_METHOD_POSITION,
        &argument,
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

pub fn encode_post_generic_link_request(
    argument: &foks_proto::PostGenericLinkArgument,
) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_POST_GENERIC_LINK_METHOD_POSITION,
        &argument.encoded()?,
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

pub fn encode_user_ping_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(USER_PROTOCOL_ID, USER_PING_METHOD_POSITION, &[0x90], 0)
}

pub fn encode_resolve_username_request(name: &[u8], open_host: bool) -> Result<Vec<u8>> {
    let authorization = Value::Array(vec![
        Value::Unsigned(if open_host { 4 } else { 0 }),
        Value::Variant(None),
    ]);
    let argument = encode(&Value::Array(vec![Value::Array(vec![
        Value::Text(name.to_vec()),
        authorization,
    ])]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_RESOLVE_USERNAME_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_get_device_nag_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        USER_PROTOCOL_ID,
        USER_GET_DEVICE_NAG_METHOD_POSITION,
        &[0x90],
        0,
    )
}

pub fn encode_clear_device_nag_request(cleared: bool) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Bool(cleared)]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_CLEAR_DEVICE_NAG_METHOD_POSITION,
        &argument,
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

pub fn encode_merkle_lookup_request(
    host: Option<&EntityId>,
    key: [u8; 32],
    signed: bool,
    root: Option<u64>,
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        optional_entity(host),
        Value::Binary(key.to_vec()),
        Value::Bool(signed),
        root.map_or(Value::Null, Value::Unsigned),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_LOOKUP_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_merkle_multi_lookup_request(
    host: Option<&EntityId>,
    keys: &[[u8; 32]],
    signed: bool,
    root: Option<u64>,
    sequence: u64,
) -> Result<Vec<u8>> {
    let keys = if keys.is_empty() {
        Value::Null
    } else {
        Value::Array(keys.iter().map(|key| Value::Binary(key.to_vec())).collect())
    };
    let argument = encode(&Value::Array(vec![
        optional_entity(host),
        keys,
        Value::Bool(signed),
        root.map_or(Value::Null, Value::Unsigned),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_MULTI_LOOKUP_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_get_current_merkle_root_hash_request(
    host: Option<&EntityId>,
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![optional_entity(host)]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_CURRENT_ROOT_HASH_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_merkle_check_key_exists_request(
    host: Option<&EntityId>,
    key: [u8; 32],
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        optional_entity(host),
        Value::Binary(key.to_vec()),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_CHECK_KEY_EXISTS_METHOD_POSITION,
        &argument,
        sequence,
    )
}

fn optional_entity(entity: Option<&EntityId>) -> Value {
    entity.map_or(Value::Null, |entity| {
        Value::Binary(entity.as_bytes().to_vec())
    })
}

/// Requests the signed current Merkle root (getCurrentRootSigned @5). The v0.1.9
/// getCurrentRoot @2 route returns a bare unsigned root; the authenticated
/// advance path must use the signed form so it can verify the delegated
/// Merkle-signer signature before accepting the root.
pub fn encode_get_current_merkle_root_signed_request(
    host: &EntityId,
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Binary(host.as_bytes().to_vec())]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_CURRENT_ROOT_SIGNED_METHOD_POSITION,
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

fn encode_load_team_chain_with_authorization(
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

pub fn encode_kv_get_request_at(
    auth: KvAuth<'_>,
    precondition: Option<&KvPathVersionVector>,
    parent: &[u8; 16],
    names: &[(u64, [u8; 32])],
    follow: u64,
    sequence: u64,
) -> Result<Vec<u8>> {
    let names = if names.is_empty() {
        Value::Null
    } else {
        Value::Array(
            names
                .iter()
                .map(|(version, mac)| {
                    Value::Array(vec![Value::Unsigned(*version), Value::Binary(mac.to_vec())])
                })
                .collect(),
        )
    };
    encode_kv_call_at(
        KV_GET_METHOD_POSITION,
        Value::Array(vec![
            kv_request_header(auth, precondition),
            Value::Array(vec![Value::Binary(parent.to_vec()), names]),
            Value::Unsigned(follow),
        ]),
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

pub fn encode_kv_usage_request_at(auth: KvAuth<'_>, sequence: u64) -> Result<Vec<u8>> {
    encode_kv_call_at(
        KV_USAGE_METHOD_POSITION,
        Value::Array(vec![auth.to_value()]),
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

/// Reads a response from a headerless (team or Kex) protocol, whose result is
/// the bare protocol struct with no DataWrap envelope.
pub fn read_bare_response<R: Read>(
    reader: &mut R,
    maximum: usize,
    expected_sequence: u64,
) -> Result<Vec<u8>> {
    let content = read_frame(reader, maximum)?;
    decode_bare_response(&content, expected_sequence)
}

/// Reads a headerless (team or Kex) response for a method with no return value,
/// whose result slot is a bare nil.
pub fn read_bare_void_response<R: Read>(
    reader: &mut R,
    maximum: usize,
    expected_sequence: u64,
) -> Result<()> {
    let content = read_frame(reader, maximum)?;
    decode_bare_void_response(&content, expected_sequence)
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
    if text(wrapped.value()?)? != b"Header" {
        return Err(Error::Compatibility);
    }
    check_compatibility_header(wrapped.value()?)?;
    if !wrapped.done() {
        return Err(Error::Envelope {
            expected: "end of void RPC data wrapper",
            found: "trailing data",
        });
    }
    Ok(())
}

/// Decodes a response from a headerless (team or Kex) protocol and returns its
/// exact result bytes. Unlike [`decode_response`], the result slot is the bare
/// protocol struct rather than a `{Data, Header}` DataWrap.
pub fn decode_bare_response(content: &[u8], expected_sequence: u64) -> Result<Vec<u8>> {
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
    let result = cursor.value()?.to_vec();
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of RPC response",
            found: "trailing data",
        });
    }
    Ok(result)
}

/// Decodes a headerless (team or Kex) response for a method with no return
/// value. go-foks void handlers return a nil result, so the result slot is a
/// bare nil rather than a DataWrap carrying only a header.
pub fn decode_bare_void_response(content: &[u8], expected_sequence: u64) -> Result<()> {
    let result = decode_bare_response(content, expected_sequence)?;
    if result != [0xc0] {
        return Err(Error::Envelope {
            expected: "nil void response result",
            found: "another response value",
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

/// Encodes a successful response for a headerless (team or Kex) protocol, whose
/// result is the bare protocol struct with no DataWrap envelope.
pub fn encode_bare_success_response_at(response: &[u8], sequence: u64) -> Result<Vec<u8>> {
    foks_snowpack::validate(response)?;
    let mut content = Vec::with_capacity(response.len() + 8);
    content.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut content);
    encode_unsigned(sequence, &mut content);
    // Nil error pointer on success.
    content.push(0xc0);
    content.extend_from_slice(response);
    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

/// Encodes a successful headerless (team or Kex) response for a method with no
/// return value. go-foks void handlers return a nil result, so the result slot
/// is a bare nil.
pub fn encode_bare_void_success_response_at(sequence: u64) -> Result<Vec<u8>> {
    let mut content = Vec::with_capacity(8);
    content.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut content);
    encode_unsigned(sequence, &mut content);
    // Nil error pointer, then a bare nil result.
    content.push(0xc0);
    content.push(0xc0);
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
    check_compatibility_header(cursor.value()?)?;
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
            found: "unexpected map length",
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
    if code == 8012 {
        let Value::Variant(Some((tag, value))) = &fields[1] else {
            return Err(Error::Envelope {
                expected: "KV stale-cache status payload",
                found: "missing status payload",
            });
        };
        if tag.as_slice() != b"b" {
            return Err(Error::Envelope {
                expected: "KV stale-cache status variant b",
                found: "another status variant",
            });
        }
        return Err(Error::KvStaleCache(positional_path_version_vector(value)?));
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

/// Decodes go-foks' positional `PathVersionVector` (`[Root, Path]`) carried in a
/// stale-cache status, applying the same directory/dirent bounds as the cached
/// projection so a hostile server cannot force an unbounded allocation.
fn positional_path_version_vector(value: &Value) -> Result<KvPathVersionVector> {
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "positional PathVersionVector",
            found: value.kind(),
        });
    };
    if fields.len() != 2 {
        return Err(Error::Envelope {
            expected: "two-field PathVersionVector",
            found: "another array length",
        });
    }
    let Value::Unsigned(root_version) = fields[0] else {
        return Err(Error::Envelope {
            expected: "unsigned PathVersionVector Root",
            found: fields[0].kind(),
        });
    };
    let directories = match &fields[1] {
        Value::Null => Vec::new(),
        Value::Array(entries) => {
            if entries.len() > foks_proto::MAXIMUM_KV_DIRECTORIES {
                return Err(Error::CollectionTooLarge {
                    kind: "cached directory",
                    received: entries.len(),
                    maximum: foks_proto::MAXIMUM_KV_DIRECTORIES,
                });
            }
            let mut total_dirents = 0usize;
            let mut output = Vec::with_capacity(entries.len());
            for entry in entries {
                output.push(positional_directory_version(entry, &mut total_dirents)?);
            }
            output
        }
        other => {
            return Err(Error::Envelope {
                expected: "PathVersionVector Path array or null",
                found: other.kind(),
            })
        }
    };
    Ok(KvPathVersionVector {
        root_version,
        directories,
    })
}

fn positional_directory_version(
    value: &Value,
    total_dirents: &mut usize,
) -> Result<foks_proto::KvDirectoryVersion> {
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "positional DirVersion",
            found: value.kind(),
        });
    };
    if fields.len() != 3 {
        return Err(Error::Envelope {
            expected: "three-field DirVersion",
            found: "another array length",
        });
    }
    let id = positional_kv_id(&fields[0])?;
    let Value::Unsigned(version) = fields[1] else {
        return Err(Error::Envelope {
            expected: "unsigned DirVersion Vers",
            found: fields[1].kind(),
        });
    };
    let entries = match &fields[2] {
        Value::Null => Vec::new(),
        Value::Array(items) => {
            *total_dirents = total_dirents.saturating_add(items.len());
            if *total_dirents > foks_proto::MAXIMUM_KV_DIRENTS {
                return Err(Error::CollectionTooLarge {
                    kind: "cached dirent",
                    received: *total_dirents,
                    maximum: foks_proto::MAXIMUM_KV_DIRENTS,
                });
            }
            items
                .iter()
                .map(positional_dirent_version)
                .collect::<Result<Vec<_>>>()?
        }
        other => {
            return Err(Error::Envelope {
                expected: "DirVersion De array or null",
                found: other.kind(),
            })
        }
    };
    Ok(foks_proto::KvDirectoryVersion {
        id,
        version,
        entries,
    })
}

fn positional_dirent_version(value: &Value) -> Result<foks_proto::KvDirentVersion> {
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "positional DirentVersion",
            found: value.kind(),
        });
    };
    if fields.len() != 2 {
        return Err(Error::Envelope {
            expected: "two-field DirentVersion",
            found: "another array length",
        });
    }
    let id = positional_kv_id(&fields[0])?;
    let Value::Unsigned(version) = fields[1] else {
        return Err(Error::Envelope {
            expected: "unsigned DirentVersion Vers",
            found: fields[1].kind(),
        });
    };
    Ok(foks_proto::KvDirentVersion { id, version })
}

fn positional_kv_id(value: &Value) -> Result<[u8; 16]> {
    let Value::Binary(bytes) = value else {
        return Err(Error::Envelope {
            expected: "16-byte KV id",
            found: value.kind(),
        });
    };
    <[u8; 16]>::try_from(bytes.as_slice()).map_err(|_| Error::Envelope {
        expected: "16-byte KV id",
        found: "another length",
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
    if length > foks_proto::MAXIMUM_KV_DIRECTORIES {
        return Err(Error::CollectionTooLarge {
            kind: "cached directory",
            received: length,
            maximum: foks_proto::MAXIMUM_KV_DIRECTORIES,
        });
    }
    if length > cursor.remaining() {
        return Err(Error::Truncated);
    }
    let mut total_dirents = 0usize;
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
                    entries = Some(named_dirent_versions_counted(
                        fields.value()?,
                        &mut total_dirents,
                    )?)
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

#[cfg(test)]
fn named_dirent_versions(bytes: &[u8]) -> Result<Vec<foks_proto::KvDirentVersion>> {
    let mut total = 0;
    named_dirent_versions_counted(bytes, &mut total)
}

fn named_dirent_versions_counted(
    bytes: &[u8],
    total: &mut usize,
) -> Result<Vec<foks_proto::KvDirentVersion>> {
    let mut cursor = Cursor::new(bytes);
    let length = array_length(&mut cursor)?;
    let next_total = total.checked_add(length).ok_or(Error::CollectionTooLarge {
        kind: "cached dirent",
        received: usize::MAX,
        maximum: foks_proto::MAXIMUM_KV_DIRENTS,
    })?;
    if next_total > foks_proto::MAXIMUM_KV_DIRENTS {
        return Err(Error::CollectionTooLarge {
            kind: "cached dirent",
            received: next_total,
            maximum: foks_proto::MAXIMUM_KV_DIRENTS,
        });
    }
    if length > cursor.remaining() {
        return Err(Error::Truncated);
    }
    *total = next_total;
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

/// Checks the fixed v1 compatibility-header shape while accepting every
/// nonzero compatibility version, matching go-foks' argument and response
/// checks. Version zero is the only compatibility generation v0.1.9 treats
/// as categorically too old.
fn check_compatibility_header(bytes: &[u8]) -> Result<()> {
    let mut header = Cursor::new(bytes);
    if map_length(&mut header)? != 2
        || text(header.value()?)? != b"V"
        || unsigned(header.value()?)? != 1
        || text(header.value()?)? != b"f1"
    {
        return Err(Error::Compatibility);
    }
    let mut v1 = Cursor::new(header.value()?);
    if map_length(&mut v1)? != 1
        || text(v1.value()?)? != b"Vers"
        || unsigned(v1.value()?)? == 0
        || !v1.done()
        || !header.done()
    {
        return Err(Error::Compatibility);
    }
    Ok(())
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

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
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
        check_compatibility_header, check_status, decode_call, encode_call,
        encode_call_with_validated_argument, encode_status_response_at, read_frame, read_response,
        resequence_call, unsigned, Cursor, Error, RpcStatus, DEFAULT_MAX_FRAME_LENGTH,
        METHOD_CALL_V2, RESPONSE_HEADER,
    };
    use foks_proto::{KvDirectoryVersion, KvDirentVersion, KvPathVersionVector};

    #[test]
    fn compatibility_headers_accept_nonzero_versions_only() {
        check_compatibility_header(RESPONSE_HEADER).unwrap();
        let mut newer = RESPONSE_HEADER.to_vec();
        *newer.last_mut().unwrap() = 2;
        check_compatibility_header(&newer).unwrap();

        let mut zero = RESPONSE_HEADER.to_vec();
        *zero.last_mut().unwrap() = 0;
        assert!(matches!(
            check_compatibility_header(&zero),
            Err(Error::Compatibility)
        ));

        let mut unknown_header_format = RESPONSE_HEADER.to_vec();
        unknown_header_format[3] = 2;
        assert!(matches!(
            check_compatibility_header(&unknown_header_format),
            Err(Error::Compatibility)
        ));
    }

    #[test]
    fn status_responses_round_trip_in_the_positional_go_shape() {
        // No-payload status: [Sc, {}].
        let framed = encode_status_response_at(&RpcStatus::RateLimited, 9).unwrap();
        let error = read_response(
            &mut std::io::Cursor::new(&framed),
            DEFAULT_MAX_FRAME_LENGTH,
            9,
        )
        .unwrap_err();
        assert!(matches!(error, Error::RemoteStatus { code: 1012, .. }));

        // Detail-string status: [Sc, {"1": <text>}].
        let framed =
            encode_status_response_at(&RpcStatus::BadArguments("bad".to_owned()), 9).unwrap();
        let error = read_response(
            &mut std::io::Cursor::new(&framed),
            DEFAULT_MAX_FRAME_LENGTH,
            9,
        )
        .unwrap_err();
        assert!(
            matches!(&error, Error::RemoteStatus { code: 1030, detail } if detail.0.as_deref() == Some("bad"))
        );

        let framed =
            encode_status_response_at(&RpcStatus::Duplicate("block".to_owned()), 9).unwrap();
        let error = read_response(
            &mut std::io::Cursor::new(&framed),
            DEFAULT_MAX_FRAME_LENGTH,
            9,
        )
        .unwrap_err();
        assert!(
            matches!(&error, Error::RemoteStatus { code: 1001, detail } if detail.0.as_deref() == Some("block"))
        );

        let framed = encode_status_response_at(&RpcStatus::KvRace("dirent".to_owned()), 9).unwrap();
        let error = read_response(
            &mut std::io::Cursor::new(&framed),
            DEFAULT_MAX_FRAME_LENGTH,
            9,
        )
        .unwrap_err();
        assert!(
            matches!(&error, Error::RemoteStatus { code: 8003, detail } if detail.0.as_deref() == Some("dirent"))
        );

        let framed =
            encode_status_response_at(&RpcStatus::MerkleVerify("proof".to_owned()), 9).unwrap();
        let error = read_response(
            &mut std::io::Cursor::new(&framed),
            DEFAULT_MAX_FRAME_LENGTH,
            9,
        )
        .unwrap_err();
        assert!(
            matches!(&error, Error::RemoteStatus { code: 4003, detail } if detail.0.as_deref() == Some("proof"))
        );

        // Stale-cache status: [8012, {"b": [Root, [ [Id, Vers, [[Id, Vers]]] ]]}].
        let pvv = KvPathVersionVector {
            root_version: 7,
            directories: vec![KvDirectoryVersion {
                id: [1; 16],
                version: 3,
                entries: vec![KvDirentVersion {
                    id: [2; 16],
                    version: 4,
                }],
            }],
        };
        let framed = encode_status_response_at(&RpcStatus::StaleCache(pvv.clone()), 10).unwrap();
        let error = read_response(
            &mut std::io::Cursor::new(&framed),
            DEFAULT_MAX_FRAME_LENGTH,
            10,
        )
        .unwrap_err();
        assert!(matches!(error, Error::KvStaleCache(v) if v == pvv));
    }

    #[test]
    fn decode_call_accepts_optional_log_tags_element() {
        let argument =
            foks_snowpack::encode(&foks_snowpack::Value::Binary(b"payload".to_vec())).unwrap();
        let framed = encode_call(17, 23, &argument, 42).unwrap();
        let content = read_frame(&mut std::io::Cursor::new(&framed), 4096).unwrap();
        assert_eq!(content[0], 0x95);

        // go-snowpack-rpc emits a six-element call with a trailing log-tags
        // element (here an empty map); the server must accept and ignore it.
        let mut tagged = content.clone();
        tagged[0] = 0x96;
        tagged.push(0x80);
        let decoded = decode_call(&tagged).unwrap();
        assert_eq!(decoded.protocol_id(), 17);
        assert_eq!(decoded.method_position(), 23);
        assert_eq!(decoded.sequence(), 42);
        assert_eq!(decoded.argument(), argument.as_slice());

        // The plain five-element call still decodes; a stray seventh element does not.
        assert!(decode_call(&content).is_ok());
        let mut over = tagged;
        over.push(0x80);
        assert!(matches!(decode_call(&over), Err(Error::Envelope { .. })));
    }

    #[test]
    fn decode_call_accepts_empty_array_niladic_argument() {
        // Go encodes a niladic argument as an empty array (0x90); the server must
        // accept it for any method (here User.getSalt @3), not just getHostConfig.
        // Build the call directly since encode_call's canonical validator (rightly)
        // forbids empty arrays in general Rust-emitted encodings.
        let framed = encode_call_with_validated_argument(0x823f_0899, 3, &[0x90], 7).unwrap();
        let content = read_frame(&mut std::io::Cursor::new(&framed), 4096).unwrap();
        let decoded = decode_call(&content).unwrap();
        assert_eq!(decoded.protocol_id(), 0x823f_0899);
        assert_eq!(decoded.method_position(), 3);
        assert_eq!(decoded.argument(), [0x90]);
        super::arguments::decode_void(decoded.argument()).unwrap();

        // A genuinely malformed (truncated) argument is still rejected by the
        // canonical validator, so the empty-array carve-out does not weaken it.
        let bad = encode_call_with_validated_argument(0x823f_0899, 3, &[0x91], 7).unwrap();
        let bad_content = read_frame(&mut std::io::Cursor::new(&bad), 4096).unwrap();
        assert!(decode_call(&bad_content).is_err());
    }

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

    #[test]
    fn stale_cache_array_lengths_are_bounded_before_allocation() {
        // array32(u32::MAX) with no following elements.
        let huge = [0xdd, 0xff, 0xff, 0xff, 0xff];
        assert!(matches!(
            super::named_directory_versions(&huge),
            Err(Error::CollectionTooLarge { .. })
        ));
        assert!(matches!(
            super::named_dirent_versions(&huge),
            Err(Error::CollectionTooLarge { .. })
        ));

        let mut total = foks_proto::MAXIMUM_KV_DIRENTS;
        assert!(matches!(
            super::named_dirent_versions_counted(&[0x91, 0x80], &mut total),
            Err(Error::CollectionTooLarge { .. })
        ));
    }
}
