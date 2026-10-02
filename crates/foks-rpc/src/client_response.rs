//! Client response envelopes and exact result extraction.

use crate::codec::{
    check_compatibility_header, encode_text, encode_unsigned, frame, read_frame, text, unsigned,
    Cursor,
};
use crate::status::check_status;
use crate::{Error, Result, DEFAULT_MAX_FRAME_LENGTH, METHOD_RESPONSE, RESPONSE_HEADER};
use foks_proto::EntityId;
use foks_snowpack::{decode, Value};
use std::io::Read;

pub fn decode_beacon_lookup_response(
    host: EntityId,
    response: &[u8],
) -> Result<foks_proto::BeaconHint> {
    foks_proto::BeaconHint::decode_address(host, response).map_err(Into::into)
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

pub(super) fn decode_data_wrap(bytes: &[u8]) -> Result<Vec<u8>> {
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
