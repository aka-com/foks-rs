use foks_proto::KvPathVersionVector;
use foks_snowpack::{encode, Value};

use super::generated::*;
use super::{
    encode_text, encode_unsigned, frame, Result, DEFAULT_MAX_FRAME_LENGTH, METHOD_RESPONSE,
    RESPONSE_HEADER,
};

/// Application errors that a v1 server may place on the v0.1.9 wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcStatus {
    OAuth2(String),
    OAuth2Auth(Box<RpcStatus>),
    Timeout,
    Realtime(String),
    RtChannelExists,
    RtRace(String),
    RtAmbiguousChannel(String),
    RtNotFound(String),
    RtMessageOrder(String),
    BadArguments(String),
    BadInvite,
    BadPassphrase,
    DeviceAlreadyProvisioned,
    Duplicate(String),
    Expired,
    WrongUser,
    Locked,
    LockTimeout,
    MerkleLeafNotFound,
    MerkleNoRoot,
    MerkleVerify(String),
    KvNoEnt,
    KvRace(String),
    KvPermission { operation: u64, resource: u64 },
    KeyNotFound(String),
    KexBadSecret,
    NameInUse,
    NoChange(String),
    NotFound(String),
    PermissionDenied(String),
    PassphraseNotFound,
    QuotaExceeded,
    RateLimited,
    RevokeRace(String),
    StaleCache(KvPathVersionVector),
    StaleRoot,
    TransactionRetry,
    TeamError(String),
    TeamRace(String),
    TeamBearerTokenStale(String),
    TeamNotFound,
    TeamInviteAlreadyAccepted,
    TeamCertificate(String),
    TeamRoster(String),
    TeamKey(String),
    TeamNoSourceRole,
    TeamRemovalKey(String),
    TeamExplore(String),
    TeamAdHocCreatorIncluded,
    TeamAdHocOpenViewership,
    TeamAdHocInvalidChange(String),
    TeamAdHocDuplicate,
    UserNotFound,
    Unsupported,
}

impl RpcStatus {
    fn code(&self) -> u64 {
        match self {
            Self::OAuth2(_) => STATUS_OAUTH2_ERROR,
            Self::OAuth2Auth(_) => STATUS_OAUTH2_AUTH_ERROR,
            Self::Timeout => STATUS_TIMEOUT_ERROR,
            Self::Realtime(_) => STATUS_RT_GENERIC_ERROR,
            Self::RtChannelExists => STATUS_RT_CHANNEL_EXISTS_ERROR,
            Self::RtRace(_) => STATUS_RT_RACE,
            Self::RtAmbiguousChannel(_) => STATUS_RT_AMBIGUOUS_CHANNEL_ERROR,
            Self::RtNotFound(_) => STATUS_RT_NOT_FOUND_ERROR,
            Self::RtMessageOrder(_) => STATUS_RT_MSG_ORDER_ERROR,
            Self::BadArguments(_) => STATUS_BAD_ARGS_ERROR,
            Self::BadInvite => STATUS_BAD_INVITE_CODE_ERROR,
            Self::BadPassphrase => STATUS_BAD_PASSPHRASE_ERROR,
            Self::Expired => STATUS_EXPIRED_ERROR,
            Self::WrongUser => STATUS_WRONG_USER_ERROR,
            Self::DeviceAlreadyProvisioned => STATUS_DEVICE_ALREADY_PROVISIONED_ERROR,
            Self::Duplicate(_) => STATUS_DUPLICATE_ERROR,
            Self::Locked => STATUS_KV_LOCK_ALREADY_HELD_ERROR,
            Self::LockTimeout => STATUS_KV_LOCK_TIMEOUT_ERROR,
            Self::MerkleLeafNotFound => STATUS_MERKLE_LEAF_NOT_FOUND_ERROR,
            Self::MerkleNoRoot => STATUS_MERKLE_NO_ROOT_ERROR,
            Self::MerkleVerify(_) => STATUS_MERKLE_VERIFY_ERROR,
            Self::KvNoEnt => STATUS_KV_NOENT_ERROR,
            Self::KvRace(_) => STATUS_KV_RACE_ERROR,
            Self::KvPermission { .. } => STATUS_KV_PERM_ERROR,
            Self::KeyNotFound(_) => STATUS_KEY_NOT_FOUND_ERROR,
            Self::KexBadSecret => STATUS_KEX_BAD_SECRET,
            Self::NameInUse => STATUS_USERNAME_IN_USE_ERROR,
            Self::NoChange(_) => STATUS_NO_CHANGE_ERROR,
            Self::NotFound(_) => STATUS_GENERIC_NOT_FOUND_ERROR,
            Self::PermissionDenied(_) => STATUS_PERMISSION_ERROR,
            Self::PassphraseNotFound => STATUS_PASSPHRASE_NOT_FOUND_ERROR,
            Self::QuotaExceeded => STATUS_OVER_QUOTA_ERROR,
            Self::RateLimited => STATUS_RATE_LIMIT_ERROR,
            Self::RevokeRace(_) => STATUS_REVOKE_RACE_ERROR,
            Self::StaleCache(_) => STATUS_KV_STALE_CACHE_ERROR,
            Self::StaleRoot | Self::TransactionRetry => STATUS_TX_RETRY_ERROR,
            Self::TeamError(_) => STATUS_TEAM_ERROR,
            Self::TeamRace(_) => STATUS_TEAM_RACE_ERROR,
            Self::TeamBearerTokenStale(_) => STATUS_TEAM_BEARER_TOKEN_STALE_ERROR,
            Self::TeamNotFound => STATUS_TEAM_NOT_FOUND_ERROR,
            Self::TeamInviteAlreadyAccepted => STATUS_TEAM_INVITE_ALREADY_ACCEPTED_ERROR,
            Self::TeamCertificate(_) => STATUS_TEAM_CERT_ERROR,
            Self::TeamRoster(_) => STATUS_TEAM_ROSTER_ERROR,
            Self::TeamKey(_) => STATUS_TEAM_KEY_ERROR,
            Self::TeamNoSourceRole => STATUS_TEAM_NO_SRC_ROLE_ERROR,
            Self::TeamRemovalKey(_) => STATUS_TEAM_REMOVAL_KEY_ERROR,
            Self::TeamExplore(_) => STATUS_TEAM_EXPLORE_ERROR,
            Self::TeamAdHocCreatorIncluded => STATUS_TEAM_ADHOC_CREATOR_INCLUDED_ERROR,
            Self::TeamAdHocOpenViewership => STATUS_TEAM_ADHOC_OPEN_VIEWERSHIP_ERROR,
            Self::TeamAdHocInvalidChange(_) => STATUS_TEAM_ADHOC_INVALID_TEAM_CHANGE_ERROR,
            Self::TeamAdHocDuplicate => STATUS_TEAM_ADHOC_DUPLICATE_ERROR,
            Self::UserNotFound => STATUS_USER_NOT_FOUND_ERROR,
            Self::Unsupported => STATUS_NOT_IMPLEMENTED,
        }
    }
}

/// Encodes a successful response for a method whose result is `Void`.
pub fn encode_void_success_response_at(sequence: u64) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    content.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut content);
    encode_unsigned(sequence, &mut content);
    content.push(0xc0);
    content.push(0x81);
    encode_text(b"Header", &mut content);
    content.extend_from_slice(RESPONSE_HEADER);
    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

/// Encodes a typed v0.1.9 application error response.
pub fn encode_status_response_at(status: &RpcStatus, sequence: u64) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    content.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut content);
    encode_unsigned(sequence, &mut content);
    encode_status(status, &mut content)?;
    // go-foks returns a nil result alongside an error status (its handlers return
    // a nil result value on error), so the response's result slot is bare nil.
    content.push(0xc0);
    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

fn encode_status(status: &RpcStatus, output: &mut Vec<u8>) -> Result<()> {
    // go-foks encodes a Status as the positional array [Sc, Switch] (a
    // `,toarray` struct), where Switch is a single-arm union. go-codec writes a
    // set union arm as a one-entry map keyed by the field's codec tag character
    // ("1" for a detail string, "a" for KV permission, "b" for the stale-cache
    // path version vector) and an unset union as an empty map — which Snowpack
    // represents as a variant. Building the value through Snowpack therefore
    // reproduces the go-foks bytes exactly, including [Sc, {}] for no payload.
    let status_value = Value::Array(vec![
        Value::Unsigned(status.code()),
        status_switch_variant(status),
    ]);
    output.extend_from_slice(&encode(&status_value)?);
    Ok(())
}

fn status_switch_variant(status: &RpcStatus) -> Value {
    let arm = |tag: &[u8], value: Value| Value::Variant(Some((tag.to_vec(), Box::new(value))));
    match status {
        RpcStatus::OAuth2Auth(inner) => arm(
            b"4",
            Value::Array(vec![
                Value::Unsigned(inner.code()),
                status_switch_variant(inner),
            ]),
        ),
        RpcStatus::NoChange(message)
        | RpcStatus::OAuth2(message)
        | RpcStatus::BadArguments(message)
        | RpcStatus::Duplicate(message)
        | RpcStatus::MerkleVerify(message)
        | RpcStatus::KvRace(message)
        | RpcStatus::NotFound(message)
        | RpcStatus::KeyNotFound(message)
        | RpcStatus::PermissionDenied(message)
        | RpcStatus::RevokeRace(message)
        | RpcStatus::TeamError(message)
        | RpcStatus::TeamRace(message)
        | RpcStatus::TeamBearerTokenStale(message)
        | RpcStatus::TeamCertificate(message)
        | RpcStatus::TeamRoster(message)
        | RpcStatus::TeamKey(message)
        | RpcStatus::TeamRemovalKey(message)
        | RpcStatus::TeamExplore(message)
        | RpcStatus::Realtime(message)
        | RpcStatus::RtRace(message)
        | RpcStatus::RtAmbiguousChannel(message)
        | RpcStatus::RtNotFound(message)
        | RpcStatus::RtMessageOrder(message)
        | RpcStatus::TeamAdHocInvalidChange(message) => {
            arm(b"1", Value::Text(message.as_bytes().to_vec()))
        }
        // go-foks KV_NOENT carries the missing path as its detail string; this
        // slice has no path to report, so emit an empty string to keep the union
        // arm present (a Go client's GetSc requires it).
        RpcStatus::KvNoEnt => arm(b"1", Value::Text(Vec::new())),
        RpcStatus::KvPermission {
            operation,
            resource,
        } => arm(
            b"a",
            Value::Array(vec![
                Value::Unsigned(*operation),
                Value::Unsigned(*resource),
            ]),
        ),
        RpcStatus::StaleCache(versions) => arm(b"b", positional_path_version_vector(versions)),
        _ => Value::Variant(None),
    }
}

fn positional_path_version_vector(versions: &KvPathVersionVector) -> Value {
    // PathVersionVector = [Root, Path]; Path is null when empty, else an array of
    // DirVersion = [Id (16-byte bin), Vers, De]; De is null when empty, else an
    // array of DirentVersion = [Id (16-byte bin), Vers]. Matches go-foks' toarray
    // layout and its nil-when-empty slice pointers.
    let path = if versions.directories.is_empty() {
        Value::Null
    } else {
        Value::Array(
            versions
                .directories
                .iter()
                .map(|directory| {
                    let entries = if directory.entries.is_empty() {
                        Value::Null
                    } else {
                        Value::Array(
                            directory
                                .entries
                                .iter()
                                .map(|entry| {
                                    Value::Array(vec![
                                        Value::Binary(entry.id.to_vec()),
                                        Value::Unsigned(entry.version),
                                    ])
                                })
                                .collect(),
                        )
                    };
                    Value::Array(vec![
                        Value::Binary(directory.id.to_vec()),
                        Value::Unsigned(directory.version),
                        entries,
                    ])
                })
                .collect(),
        )
    };
    Value::Array(vec![Value::Unsigned(versions.root_version), path])
}
