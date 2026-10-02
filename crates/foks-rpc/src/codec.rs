//! Bounded MessagePack framing and cursor primitives.

use crate::{Error, Result};
use foks_snowpack::{decode, Value};
use std::io::Read;

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

pub(super) fn map_length(cursor: &mut Cursor<'_>) -> Result<usize> {
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

pub(super) fn array_length(cursor: &mut Cursor<'_>) -> Result<usize> {
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

pub(super) fn unsigned(bytes: &[u8]) -> Result<u64> {
    match decode(bytes)? {
        Value::Unsigned(value) => Ok(value),
        value => Err(Error::Envelope {
            expected: "unsigned integer",
            found: value.kind(),
        }),
    }
}

pub(super) fn text(bytes: &[u8]) -> Result<Vec<u8>> {
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
pub(super) fn check_compatibility_header(bytes: &[u8]) -> Result<()> {
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

pub(super) fn frame(content: &[u8], maximum: usize) -> Result<Vec<u8>> {
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

pub(super) fn read_frame_length<R: Read>(reader: &mut R) -> Result<usize> {
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

pub(super) fn read_byte<R: Read>(reader: &mut R) -> std::io::Result<u8> {
    let mut byte = [0];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
}

pub(super) fn read_u16<R: Read>(reader: &mut R) -> std::io::Result<u16> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
}

pub(super) fn read_u32<R: Read>(reader: &mut R) -> std::io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

pub(super) fn encode_unsigned(value: u64, output: &mut Vec<u8>) {
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

pub(super) fn encode_text(value: &[u8], output: &mut Vec<u8>) {
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

pub(super) struct Cursor<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) position: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(super) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    pub(super) fn done(&self) -> bool {
        self.position == self.bytes.len()
    }

    pub(super) fn byte(&mut self) -> Result<u8> {
        let byte = *self.bytes.get(self.position).ok_or(Error::Truncated)?;
        self.position += 1;
        Ok(byte)
    }

    pub(super) fn take(&mut self, length: usize) -> Result<()> {
        self.position = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(Error::Truncated)?;
        Ok(())
    }

    pub(super) fn value(&mut self) -> Result<&'a [u8]> {
        let start = self.position;
        self.skip(0)?;
        Ok(&self.bytes[start..self.position])
    }

    pub(super) fn skip(&mut self, depth: usize) -> Result<()> {
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

    pub(super) fn skip_many(&mut self, count: usize, depth: usize) -> Result<()> {
        if count > self.bytes.len().saturating_sub(self.position) {
            return Err(Error::Truncated);
        }
        for _ in 0..count {
            self.skip(depth + 1)?;
        }
        Ok(())
    }

    pub(super) fn number_u16(&mut self) -> Result<u16> {
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

    pub(super) fn number_u32(&mut self) -> Result<u32> {
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

pub(super) trait ValueKind {
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
