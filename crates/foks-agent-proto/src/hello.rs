//! The version handshake, whose shapes do not depend on [`PROTOCOL_VERSION`].
//!
//! Every other frame must carry exactly [`PROTOCOL_VERSION`], so a client and
//! an agent of different versions cannot decode each other's requests and
//! responses. Two shapes are exempt and frozen, so that a client can determine
//! which versions a running agent accepts whatever either version is:
//!
//! - a Hello request and the agent's reply to it ([`HelloRequest`],
//!   [`HelloReply`]), which the agent answers before it checks the version;
//! - the agent's `version-mismatch` error, whose `version`, `status`, `code`
//!   and `fields.minimum_protocol` / `fields.maximum_protocol` keys every
//!   client reads whatever the frame's version (see
//!   [`crate::decode_response`]).
//!
//! A later version may add fields to these shapes, and decoders here ignore
//! fields they do not know, but no version removes, renames or retypes one.
//! An agent that answers `version-mismatch` has not decoded the request's
//! operation, so a mutation refused that way did not run. An agent answers
//! each request it accepts in the version that request carries, so a client
//! whose version is in the agent's supported range can decode every reply.

use serde::{Deserialize, Serialize};

use crate::{Error, Result, PROTOCOL_VERSION};

/// The protocol versions this build accepts and answers.
///
/// Requests and responses are not shaped per version, and many of their
/// types reject unknown fields, so a field added in one version breaks a
/// peer of the previous one. This build therefore accepts exactly
/// [`PROTOCOL_VERSION`]; the range exists so the handshake can describe a
/// wider one without changing shape. Widening it requires the agent to
/// decode requests of every version in the range and to answer each in the
/// version it carries, which no build does yet.
pub const SUPPORTED_PROTOCOL_VERSIONS: ProtocolRange = ProtocolRange::exactly(PROTOCOL_VERSION);

/// The longest peer kind, peer version or capability name a Hello carries.
pub const MAXIMUM_HELLO_FIELD_BYTES: usize = 64;

/// The most capabilities one Hello names.
pub const MAXIMUM_HELLO_CAPABILITIES: usize = 64;

/// Capability names an agent advertises in its [`HelloReply`].
pub mod capability {
    /// `ChatAction::History` accepts an exclusive `after` cursor.
    pub const HISTORY_AFTER: &str = "history-after";
}

/// An inclusive range of agent protocol versions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtocolRange {
    pub minimum: u32,
    pub maximum: u32,
}

impl ProtocolRange {
    pub const fn exactly(version: u32) -> Self {
        Self {
            minimum: version,
            maximum: version,
        }
    }

    pub const fn contains(&self, version: u32) -> bool {
        self.minimum <= version && version <= self.maximum
    }
}

impl std::fmt::Display for ProtocolRange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.minimum == self.maximum {
            write!(formatter, "{}", self.minimum)
        } else {
            write!(formatter, "{} to {}", self.minimum, self.maximum)
        }
    }
}

/// One end of the handshake: a program name such as `foks-cli` and its
/// build version, for diagnostics only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Peer {
    pub kind: String,
    pub version: String,
}

impl Peer {
    pub fn new(kind: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            version: version.into(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.kind.is_empty() || self.kind.len() > MAXIMUM_HELLO_FIELD_BYTES {
            return Err(Error::HelloInvalid("peer kind is empty or too long"));
        }
        if self.version.len() > MAXIMUM_HELLO_FIELD_BYTES {
            return Err(Error::HelloInvalid("peer version is too long"));
        }
        Ok(())
    }
}

/// The operation tag of a Hello, `"hello"` on the wire. It is not an
/// [`crate::Operation`], so a Hello never reaches the versioned dispatcher.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HelloTag {
    Hello,
}

/// A client's Hello. The envelope matches [`crate::Request`], so an agent
/// that predates the handshake reads its version first and answers
/// `version-mismatch`, which the client decodes as such.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HelloRequest {
    pub version: u32,
    pub id: u64,
    pub operation: Hello,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Hello {
    pub operation: HelloTag,
    pub client: Peer,
    /// The versions the client can send and decode.
    pub accept: ProtocolRange,
    /// What the client supports beyond its version. No client capability is
    /// defined yet; the list exists so one can be added without a new shape.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl HelloRequest {
    /// A Hello from a client of this build.
    pub fn new(id: u64, client: Peer) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            operation: Hello {
                operation: HelloTag::Hello,
                client,
                accept: SUPPORTED_PROTOCOL_VERSIONS,
                capabilities: Vec::new(),
            },
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.operation.client.validate()?;
        validate_capabilities(&self.operation.capabilities)
    }
}

/// The agent's answer to a Hello, carried as the `value` of a success
/// response stamped with the agent's own [`PROTOCOL_VERSION`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HelloReply {
    /// The agent's own version, within `supported`.
    pub protocol: u32,
    /// The versions the agent accepts. It answers each request in the
    /// version that request carries.
    pub supported: ProtocolRange,
    pub agent: Peer,
    /// Features the agent offers within its version, from [`capability`].
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl HelloReply {
    /// The reply of an agent of this build.
    pub fn new(agent: Peer, capabilities: &[&str]) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            supported: SUPPORTED_PROTOCOL_VERSIONS,
            agent,
            capabilities: capabilities.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.agent.validate()?;
        if !self.supported.contains(self.protocol) {
            return Err(Error::HelloInvalid(
                "agent version is outside its supported range",
            ));
        }
        validate_capabilities(&self.capabilities)
    }

    pub fn has_capability(&self, name: &str) -> bool {
        self.capabilities
            .iter()
            .any(|capability| capability == name)
    }

    /// Refuses an agent that does not accept this build's version.
    pub fn require_compatible(&self) -> Result<()> {
        if self.supported.contains(PROTOCOL_VERSION) {
            Ok(())
        } else {
            Err(Error::VersionMismatch {
                agent: self.protocol,
                supported: Some(self.supported),
            })
        }
    }
}

fn validate_capabilities(capabilities: &[String]) -> Result<()> {
    if capabilities.len() > MAXIMUM_HELLO_CAPABILITIES
        || capabilities
            .iter()
            .any(|name| name.is_empty() || name.len() > MAXIMUM_HELLO_FIELD_BYTES)
    {
        return Err(Error::HelloInvalid("capability list exceeds its bounds"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        decode_hello, decode_hello_response, decode_request, decode_response, encode, ErrorCode,
        Operation, Request, Response, ResponseResult,
    };

    fn frame(value: serde_json::Value) -> zeroize::Zeroizing<Vec<u8>> {
        encode(&value).unwrap()
    }

    #[test]
    fn the_hello_request_shape_is_frozen() {
        let hello = HelloRequest::new(3, Peer::new("foks-cli", "0.1.0"));
        assert_eq!(
            serde_json::to_value(&hello).unwrap(),
            serde_json::json!({
                "version": PROTOCOL_VERSION,
                "id": 3,
                "operation": {
                    "operation": "hello",
                    "client": { "kind": "foks-cli", "version": "0.1.0" },
                    "accept": { "minimum": PROTOCOL_VERSION, "maximum": PROTOCOL_VERSION },
                    "capabilities": [],
                }
            })
        );
        // A Hello is not a versioned request, so it never reaches dispatch.
        assert!(decode_request(&encode(&hello).unwrap()).is_err());
        assert_eq!(decode_hello(&encode(&hello).unwrap()).unwrap(), Some(hello));
    }

    #[test]
    fn a_hello_of_any_version_is_recognized_and_other_frames_are_not() {
        for version in [1, PROTOCOL_VERSION - 1, PROTOCOL_VERSION + 7] {
            let hello = decode_hello(&frame(serde_json::json!({
                "version": version,
                "id": 9,
                "operation": {
                    "operation": "hello",
                    "client": { "kind": "foks-desktop", "version": "9.9.9" },
                    "accept": { "minimum": version, "maximum": version + 1 },
                    "capabilities": ["future-capability"],
                    "future-field": true,
                }
            })))
            .unwrap()
            .expect("a hello");
            assert_eq!(hello.version, version);
            assert_eq!(hello.id, 9);
            assert_eq!(hello.operation.accept.maximum, version + 1);
        }
        let ping = encode(&Request::new(1, Operation::Ping)).unwrap();
        assert_eq!(decode_hello(&ping).unwrap(), None);
        assert_eq!(decode_hello(&[0, 0, 0, 1, b'{']).unwrap(), None);
        let oversized = frame(serde_json::json!({
            "version": PROTOCOL_VERSION,
            "id": 9,
            "operation": {
                "operation": "hello",
                "client": { "kind": "k".repeat(MAXIMUM_HELLO_FIELD_BYTES + 1), "version": "1" },
                "accept": { "minimum": 1, "maximum": 1 },
            }
        }));
        assert!(matches!(
            decode_hello(&oversized),
            Err(Error::HelloInvalid(_))
        ));
    }

    #[test]
    fn the_version_mismatch_refusal_is_typed_whatever_its_version() {
        let current = Response::version_mismatch(Some(4));
        assert_eq!(
            serde_json::to_value(&current).unwrap(),
            serde_json::json!({
                "version": PROTOCOL_VERSION,
                "id": 4,
                "status": "error",
                "code": "version-mismatch",
                "message": "unsupported protocol version",
                "fields": {
                    "minimum_protocol": PROTOCOL_VERSION,
                    "maximum_protocol": PROTOCOL_VERSION,
                },
            })
        );
        // From an agent of this version it is an ordinary response.
        assert!(matches!(
            decode_response(&encode(&current).unwrap()).unwrap().result,
            ResponseResult::Error {
                code: ErrorCode::VersionMismatch,
                ..
            }
        ));

        // A newer agent, with fields this build does not know.
        let newer = frame(serde_json::json!({
            "version": PROTOCOL_VERSION + 2,
            "id": 4,
            "status": "error",
            "code": "version-mismatch",
            "message": "unsupported protocol version",
            "fields": {
                "minimum_protocol": PROTOCOL_VERSION + 1,
                "maximum_protocol": PROTOCOL_VERSION + 2,
                "future_field": "x",
            },
            "future_envelope_field": 1,
        }));
        let error = decode_response(&newer).unwrap_err();
        assert!(matches!(
            error,
            Error::VersionMismatch {
                agent,
                supported: Some(ProtocolRange { minimum, maximum }),
            } if agent == PROTOCOL_VERSION + 2
                && minimum == PROTOCOL_VERSION + 1
                && maximum == PROTOCOL_VERSION + 2
        ));
        assert_eq!(
            error.to_string(),
            format!(
                "agent protocol version {} (accepted: {} to {}) does not match client protocol version {}",
                PROTOCOL_VERSION + 2,
                PROTOCOL_VERSION + 1,
                PROTOCOL_VERSION + 2,
                PROTOCOL_VERSION
            )
        );

        // An agent that predates the handshake sends no range.
        let older = frame(serde_json::json!({
            "version": 34,
            "id": 4,
            "status": "error",
            "code": "version-mismatch",
            "message": "unsupported protocol version",
        }));
        let error = decode_response(&older).unwrap_err();
        assert!(matches!(
            error,
            Error::VersionMismatch {
                agent: 34,
                supported: None
            }
        ));
        assert_eq!(
            error.to_string(),
            format!(
                "agent protocol version 34 does not match client protocol version {PROTOCOL_VERSION}"
            )
        );

        // Anything else of another version is not a claim that nothing ran.
        for other in [
            serde_json::json!({
                "version": PROTOCOL_VERSION + 1, "id": 4, "status": "success", "value": 1,
            }),
            serde_json::json!({
                "version": PROTOCOL_VERSION + 1, "id": 4, "status": "error",
                "code": "busy", "message": "busy",
            }),
        ] {
            assert!(matches!(
                decode_response(&frame(other)),
                Err(Error::Version)
            ));
        }
    }

    #[test]
    fn a_hello_reply_decodes_whatever_its_version_and_is_checked_for_compatibility() {
        let reply = HelloReply::new(
            Peer::new("foks-agent", "0.1.0"),
            &[capability::HISTORY_AFTER],
        );
        let current = encode(&Response::success(5, serde_json::to_value(&reply).unwrap())).unwrap();
        let decoded = decode_hello_response(&current).unwrap();
        assert_eq!(decoded.id, Some(5));
        assert_eq!(decoded.reply, reply);
        assert!(decoded.reply.has_capability(capability::HISTORY_AFTER));
        assert!(!decoded.reply.has_capability("unknown"));
        decoded.reply.require_compatible().unwrap();

        // A newer agent that still accepts this version.
        let wider = decode_hello_response(&frame(serde_json::json!({
            "version": PROTOCOL_VERSION + 1,
            "id": 5,
            "status": "success",
            "value": {
                "protocol": PROTOCOL_VERSION + 1,
                "supported": { "minimum": PROTOCOL_VERSION, "maximum": PROTOCOL_VERSION + 1 },
                "agent": { "kind": "foks-agent", "version": "2.0.0" },
                "capabilities": ["history-after", "future"],
                "future_field": [1, 2],
            },
        })))
        .unwrap();
        wider.reply.require_compatible().unwrap();

        // A newer agent that does not.
        let newer = decode_hello_response(&frame(serde_json::json!({
            "version": PROTOCOL_VERSION + 1,
            "id": 5,
            "status": "success",
            "value": {
                "protocol": PROTOCOL_VERSION + 1,
                "supported": { "minimum": PROTOCOL_VERSION + 1, "maximum": PROTOCOL_VERSION + 1 },
                "agent": { "kind": "foks-agent", "version": "2.0.0" },
            },
        })))
        .unwrap();
        assert!(matches!(
            newer.reply.require_compatible(),
            Err(Error::VersionMismatch { agent, supported: Some(_) })
                if agent == PROTOCOL_VERSION + 1
        ));

        // An agent that predates the handshake refuses the Hello's version.
        assert!(matches!(
            decode_hello_response(&frame(serde_json::json!({
                "version": 34,
                "id": 5,
                "status": "error",
                "code": "version-mismatch",
                "message": "unsupported protocol version",
            }))),
            Err(Error::VersionMismatch {
                agent: 34,
                supported: None
            })
        ));

        // Any other refusal keeps its wire code, known or not.
        assert!(matches!(
            decode_hello_response(&frame(serde_json::json!({
                "version": PROTOCOL_VERSION + 1,
                "id": 5,
                "status": "error",
                "code": "future-code",
                "message": "later",
            }))),
            Err(Error::HelloRefused { code, message }) if code == "future-code" && message == "later"
        ));
        assert!(matches!(
            decode_hello_response(&frame(serde_json::json!({
                "version": PROTOCOL_VERSION,
                "id": 5,
                "status": "success",
                "value": {
                    "protocol": PROTOCOL_VERSION,
                    "supported": { "minimum": 2, "maximum": 1 },
                    "agent": { "kind": "foks-agent", "version": "1" },
                },
            }))),
            Err(Error::HelloInvalid(_))
        ));
    }
}
