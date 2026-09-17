use std::io::Read;

use super::{
    check_compatibility_header, is_headerless_argument_protocol, read_frame, text, unsigned,
    Cursor, Error, Result, METHOD_CALL_V2,
};

/// A validated v0.1.9 RPC call whose protocol argument remains byte-exact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedCall {
    sequence: u64,
    protocol_id: u64,
    method_position: u64,
    argument: Vec<u8>,
}

impl DecodedCall {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn protocol_id(&self) -> u64 {
        self.protocol_id
    }

    pub fn method_position(&self) -> u64 {
        self.method_position
    }

    pub fn argument(&self) -> &[u8] {
        &self.argument
    }
}

/// Reads and strictly decodes one bounded, length-prefixed RPC call.
pub fn read_call<R: Read>(reader: &mut R, maximum: usize) -> Result<DecodedCall> {
    decode_call(&read_frame(reader, maximum)?)
}

// go-snowpack-rpc method types (protocol.go). CALL_V2 is the only request this
// server answers; NOTIFY and CANCEL (and their V2 forms) are control messages
// that a peer may send and that are never answered.
const METHOD_NOTIFY: u64 = 2;
const METHOD_CANCEL: u64 = 3;
const METHOD_NOTIFY_V2: u64 = 6;
const METHOD_CANCEL_V2: u64 = 7;

/// One classified inbound RPC message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InboundMessage {
    /// A `CALL_V2` request to route and answer.
    Call(DecodedCall),
    /// A well-formed `NOTIFY` or `CANCEL` control message (method types 2, 3,
    /// 6, 7). go-snowpack-rpc never sends a reply for these, so the server
    /// parses and discards them rather than closing the connection.
    Control,
}

/// Reads and classifies one bounded, length-prefixed inbound RPC message.
pub fn read_message<R: Read>(reader: &mut R, maximum: usize) -> Result<InboundMessage> {
    decode_message(&read_frame(reader, maximum)?)
}

/// Classifies one unframed inbound RPC message, tolerating the control
/// messages go-snowpack-rpc may send on an otherwise idle connection.
///
/// A `CALL_V2` is decoded with the same strictness as [`decode_call`]. A
/// `NOTIFY`/`CANCEL` frame is fully parsed for well-formedness and reported as
/// [`InboundMessage::Control`] so the caller can discard it without replying.
/// Any other method type, or a non-array envelope, is a fatal protocol error,
/// matching go-snowpack-rpc's packetizer (which requires a fixarray envelope).
pub fn decode_message(content: &[u8]) -> Result<InboundMessage> {
    let mut cursor = Cursor::new(content);
    let elements = match cursor.byte()? {
        byte @ 0x91..=0x9f => usize::from(byte - 0x90),
        _ => {
            return Err(Error::Envelope {
                expected: "fixarray RPC message envelope",
                found: "unexpected MessagePack value type",
            })
        }
    };
    match unsigned(cursor.value()?)? {
        METHOD_CALL_V2 => decode_call(content).map(InboundMessage::Call),
        METHOD_NOTIFY | METHOD_CANCEL | METHOD_NOTIFY_V2 | METHOD_CANCEL_V2 => {
            // Parse and discard the remaining elements so a malformed control
            // frame is still rejected, but never construct a reply.
            for _ in 1..elements {
                cursor.value()?;
            }
            if !cursor.done() {
                return Err(Error::Envelope {
                    expected: "end of RPC control message",
                    found: "trailing data",
                });
            }
            Ok(InboundMessage::Control)
        }
        _ => Err(Error::Envelope {
            expected: "a known RPC method",
            found: "an unsupported RPC method",
        }),
    }
}

/// Strictly decodes one unframed v0.1.9 RPC call.
pub fn decode_call(content: &[u8]) -> Result<DecodedCall> {
    let mut cursor = Cursor::new(content);
    let call_elements = match cursor.byte()? {
        0x95 => 5,
        // go-snowpack-rpc appends the caller's context log tags as an optional
        // sixth element (go-foks tags pervasively, including the first probe).
        // Accept and ignore it so tagged Go calls complete first contact instead
        // of being fatally rejected. The element stays bounded by the already
        // length-capped frame, so this does not widen the parser's memory limits.
        0x96 => 6,
        _ => {
            return Err(Error::Envelope {
                expected: "five- or six-element RPC call array",
                found: "another MessagePack value",
            })
        }
    };
    if unsigned(cursor.value()?)? != METHOD_CALL_V2 {
        return Err(Error::Envelope {
            expected: "RPC call method",
            found: "another RPC method",
        });
    }
    let sequence = unsigned(cursor.value()?)?;
    let protocol_id = unsigned(cursor.value()?)?;
    let method_position = unsigned(cursor.value()?)?;
    let payload = cursor.value()?;
    if call_elements == 6 {
        // Discard the optional trailing log-tags element.
        let _ = cursor.value()?;
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of RPC call",
            found: "trailing data",
        });
    }

    // Team and Kex protocols place the bare argument struct directly in the
    // payload slot; every other protocol wraps it in a `{Data, Header}`
    // DataWrap map (go-foks proto/rem/team.go and kex.go versus reg.go et al.).
    let argument = if is_headerless_argument_protocol(protocol_id) {
        payload.to_vec()
    } else {
        decode_wrapped_argument(payload)?
    };
    // Go encodes a niladic (zero-field) argument as an empty array (0x90), which
    // the canonical validator forbids; Rust encodes Void as null. Accept the empty
    // array form for any method so tagged Go niladic calls (getSalt, stretchVersion,
    // getPpeParcel, getHostConfig, ...) are not fatally rejected. A method that
    // actually expects fields still fails closed when its handler decodes the empty
    // argument.
    if argument != [0x90] {
        foks_snowpack::validate(&argument).map_err(|source| Error::ArgumentSnowpack {
            protocol_id,
            method_position,
            source,
        })?;
    }

    Ok(DecodedCall {
        sequence,
        protocol_id,
        method_position,
        argument,
    })
}

/// Unwraps the `{Data, Header}` DataWrap that wraps arguments for every
/// non-headerless protocol, returning the exact canonical argument bytes.
fn decode_wrapped_argument(payload: &[u8]) -> Result<Vec<u8>> {
    let mut wrapped = Cursor::new(payload);
    if wrapped.byte()? != 0x82 {
        return Err(Error::Envelope {
            expected: "two-field RPC data wrapper",
            found: "another MessagePack value",
        });
    }
    if text(wrapped.value()?)? != b"Data" {
        return Err(Error::Envelope {
            expected: "canonical Data field",
            found: "unexpected field key",
        });
    }
    let argument = wrapped.value()?.to_vec();
    if text(wrapped.value()?)? != b"Header" {
        return Err(Error::Envelope {
            expected: "canonical Header field",
            found: "another field",
        });
    }
    check_compatibility_header(wrapped.value()?)?;
    if !wrapped.done() {
        return Err(Error::Envelope {
            expected: "end of RPC data wrapper",
            found: "trailing data",
        });
    }
    Ok(argument)
}
