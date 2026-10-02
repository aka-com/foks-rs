//! Call envelopes and connection sequence rewriting.

use crate::codec::{encode_text, encode_unsigned, frame, read_frame, unsigned, Cursor};
use crate::generated::is_headerless_argument_protocol;
use crate::{Error, Result, DEFAULT_MAX_FRAME_LENGTH, METHOD_CALL_V2, RESPONSE_HEADER};
use foks_proto::EntityId;
use foks_snowpack::{encode, Value};

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

pub(super) fn encode_call_with_validated_argument(
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

pub(super) fn encode_select_vhost(protocol: u64, method: u64, host: &EntityId) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Binary(host.as_bytes().to_vec())]))?;
    encode_call(protocol, method, &argument, 0)
}
