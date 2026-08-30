use foks_proto::KvPathVersionVector;
use foks_snowpack::{encode, Value};

use super::{
    encode_text, encode_unsigned, frame, Result, DEFAULT_MAX_FRAME_LENGTH, METHOD_RESPONSE,
    RESPONSE_HEADER, STATUS_BAD_PASSPHRASE_ERROR, STATUS_PASSPHRASE_NOT_FOUND_ERROR,
};

/// Application errors that a v1 server may place on the v0.1.9 wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcStatus {
    BadArguments(String),
    BadInvite,
    BadPassphrase,
    DeviceAlreadyProvisioned,
    Expired,
    Locked,
    LockTimeout,
    MerkleLeafNotFound,
    MerkleNoRoot,
    KvNoEnt,
    KvPermission { operation: u64, resource: u64 },
    KeyNotFound(String),
    NameInUse,
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
            Self::BadArguments(_) => 1030,
            Self::BadInvite => 1019,
            Self::BadPassphrase => STATUS_BAD_PASSPHRASE_ERROR,
            Self::Expired => 1062,
            Self::DeviceAlreadyProvisioned => 1072,
            Self::Locked => 8014,
            Self::LockTimeout => 8015,
            Self::MerkleLeafNotFound => 4002,
            Self::MerkleNoRoot => 4001,
            Self::KvNoEnt => 8016,
            Self::KvPermission { .. } => 8011,
            Self::KeyNotFound(_) => 1025,
            Self::NameInUse => 1023,
            Self::NotFound(_) => 1049,
            Self::PermissionDenied(_) => 1013,
            Self::PassphraseNotFound => STATUS_PASSPHRASE_NOT_FOUND_ERROR,
            Self::QuotaExceeded => 1060,
            Self::RateLimited => 1012,
            Self::RevokeRace(_) => 1044,
            Self::StaleCache(_) => 8012,
            Self::StaleRoot | Self::TransactionRetry => 1014,
            Self::TeamError(_) => 7001,
            Self::TeamRace(_) => 7002,
            Self::TeamBearerTokenStale(_) => 7003,
            Self::TeamNotFound => 7004,
            Self::TeamCertificate(_) => 7005,
            Self::TeamRoster(_) => 7006,
            Self::TeamKey(_) => 7007,
            Self::TeamNoSourceRole => 7008,
            Self::TeamRemovalKey(_) => 7009,
            Self::TeamExplore(_) => 7010,
            Self::TeamAdHocCreatorIncluded => 7101,
            Self::TeamAdHocOpenViewership => 7102,
            Self::TeamAdHocInvalidChange(_) => 7103,
            Self::TeamAdHocDuplicate => 7104,
            Self::UserNotFound => 1027,
            Self::Unsupported => 1020,
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
        RpcStatus::BadArguments(message)
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
