//! Strict framing for the FOKS v0.1.9 Snowpack RPC public probe.
//!
//! Snowpack RPC uses a standard MessagePack envelope (including named maps)
//! around canonical Snowpack protocol values. This crate parses only the
//! envelope needed for the unauthenticated public probe and returns the exact
//! `ProbeRes` bytes for signature verification by `foks-verify`.

#![forbid(unsafe_code)]

use std::io::{Read, Write};

use foks_snowpack::{decode, encode, Value};
use thiserror::Error;

pub const PROBE_PROTOCOL_ID: u64 = 0xc588_4ff6;
pub const PROBE_METHOD_POSITION: u64 = 1;
pub const REG_PROTOCOL_ID: u64 = 0xf7ab_85f3;
pub const REG_GET_CLIENT_CERT_CHAIN_METHOD_POSITION: u64 = 1;
pub const USER_PROTOCOL_ID: u64 = 0x823f_0899;
pub const USER_LOAD_USER_CHAIN_METHOD_POSITION: u64 = 9;
pub const USER_GET_PUK_FOR_ROLE_METHOD_POSITION: u64 = 14;
pub const MERKLE_QUERY_PROTOCOL_ID: u64 = 0xc041_2aa6;
pub const MERKLE_GET_HISTORICAL_ROOTS_METHOD_POSITION: u64 = 1;
pub const MERKLE_GET_CURRENT_ROOT_METHOD_POSITION: u64 = 2;
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
    foks_snowpack::validate(argument)?;
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
    let argument = encode(&Value::Array(vec![
        Value::Binary(uid.to_vec()),
        Value::Binary(device_id.to_vec()),
    ]))?;
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_CLIENT_CERT_CHAIN_METHOD_POSITION,
        &argument,
        0,
    )
}

/// Encodes an authenticated request for a user's chain, starting at `start`.
pub fn encode_load_user_chain_request(uid: &[u8], start: u64) -> Result<Vec<u8>> {
    let as_local_user = Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]);
    let argument = encode(&Value::Array(vec![Value::Array(vec![
        Value::Binary(uid.to_vec()),
        Value::Unsigned(start),
        Value::Null,
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
    let owner = Value::Array(vec![Value::Unsigned(3), Value::Variant(None)]);
    let argument = encode(&Value::Array(vec![
        owner,
        Value::Binary(device_id.to_vec()),
    ]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_GET_PUK_FOR_ROLE_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_get_current_merkle_root_request() -> Result<Vec<u8>> {
    let mut content = Vec::with_capacity(40);
    content.push(0x95);
    encode_unsigned(METHOD_CALL_V2, &mut content);
    encode_unsigned(0, &mut content);
    encode_unsigned(MERKLE_QUERY_PROTOCOL_ID, &mut content);
    encode_unsigned(MERKLE_GET_CURRENT_ROOT_METHOD_POSITION, &mut content);
    content.push(0x81); // nil pointer data is omitted by the Go RPC wrapper
    encode_text(b"Header", &mut content);
    content.extend_from_slice(RESPONSE_HEADER);
    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

pub fn encode_get_historical_merkle_roots_request(
    full_epochs: &[u64],
    hash_epochs: &[u64],
) -> Result<Vec<u8>> {
    let list = |epochs: &[u64]| {
        if epochs.is_empty() {
            Value::Null
        } else {
            Value::Array(epochs.iter().copied().map(Value::Unsigned).collect())
        }
    };
    let argument = encode(&Value::Array(vec![
        Value::Null,
        list(full_epochs),
        list(hash_epochs),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_HISTORICAL_ROOTS_METHOD_POSITION,
        &argument,
        0,
    )
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

/// Encodes a successful response for protocol fixtures and local test servers.
#[doc(hidden)]
pub fn encode_probe_success_response(probe_response: &[u8]) -> Result<Vec<u8>> {
    foks_snowpack::validate(probe_response)?;
    let mut content = Vec::with_capacity(probe_response.len() + 32);
    content.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut content);
    encode_unsigned(0, &mut content);
    // The RPC layer encodes a nil error pointer on success. A concrete FOKS
    // Status value is present only when the server returns an application
    // error.
    content.push(0xc0);
    content.push(0x82);
    encode_text(b"Data", &mut content);
    content.extend_from_slice(probe_response);
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
    foks_snowpack::validate(&data)?;
    Ok(data)
}

fn check_status(bytes: &[u8]) -> Result<()> {
    let value = decode(bytes)?;
    if value == Value::Null {
        return Ok(());
    }
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "FOKS Status array",
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
