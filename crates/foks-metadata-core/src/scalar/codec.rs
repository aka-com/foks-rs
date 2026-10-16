//! Finite Value JSON writer and strict catalog reply decoder. The public serde
//! implementation remains unchanged; this codec is private to the SQL host.
mod decode;
use super::Decimal;
use crate::{catalog::Spec, Error, Result, Value};

pub(crate) const MAX_BYTES: usize = 65_536;
fn add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right).ok_or(Error::Bounds)
}
fn string_size(bytes: &[u8]) -> Result<usize> {
    bytes.iter().try_fold(2, |size, byte| {
        add(
            size,
            match byte {
                b'"' | b'\\' | b'\x08' | b'\x0c' | b'\n' | b'\r' | b'\t' => 2,
                0..=31 => 6,
                _ => 1,
            },
        )
    })
}
fn byte_width(byte: u8) -> usize {
    if byte >= 100 {
        3
    } else if byte >= 10 {
        2
    } else {
        1
    }
}
fn value_size(value: &Value) -> Result<usize> {
    match value {
        Value::Null => Ok(6),
        Value::Integer(n) => add(14, Decimal::new(*n).bytes().len()),
        Value::Text(text) => add(9, string_size(text.as_bytes())?),
        Value::Blob(bytes) => {
            let mut size = 11;
            for (index, byte) in bytes.iter().enumerate() {
                size = add(size, byte_width(*byte) + usize::from(index > 0))?;
            }
            Ok(size)
        }
    }
}
struct Writer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Writer {
    fn new(limit: usize) -> Result<Self> {
        crate::page::charge(limit)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(limit).map_err(|_| Error::Bounds)?;
        Ok(Self { bytes, limit })
    }
    fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if add(self.bytes.len(), bytes.len())? > self.limit {
            return Err(Error::Bounds);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn string(&mut self, bytes: &[u8]) -> Result<()> {
        self.push(b"\"")?;
        for byte in bytes {
            match byte {
                b'"' => self.push(b"\\\"")?,
                b'\\' => self.push(b"\\\\")?,
                b'\x08' => self.push(b"\\b")?,
                b'\x0c' => self.push(b"\\f")?,
                b'\n' => self.push(b"\\n")?,
                b'\r' => self.push(b"\\r")?,
                b'\t' => self.push(b"\\t")?,
                0..=31 => {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    self.push(&[
                        b'\\',
                        b'u',
                        b'0',
                        b'0',
                        HEX[usize::from(byte >> 4)],
                        HEX[usize::from(byte & 15)],
                    ])?;
                }
                _ => self.push(&[*byte])?,
            }
        }
        self.push(b"\"")
    }
    fn value(&mut self, value: &Value) -> Result<()> {
        match value {
            Value::Null => self.push(b"\"null\""),
            Value::Integer(n) => {
                self.push(b"{\"integer\":\"")?;
                self.push(Decimal::new(*n).bytes())?;
                self.push(b"\"}")
            }
            Value::Text(text) => {
                self.push(b"{\"text\":")?;
                self.string(text.as_bytes())?;
                self.push(b"}")
            }
            Value::Blob(bytes) => {
                self.push(b"{\"blob\":[")?;
                for (index, byte) in bytes.iter().enumerate() {
                    if index > 0 {
                        self.push(b",")?;
                    }
                    self.push(Decimal::new(i64::from(*byte)).bytes())?;
                }
                self.push(b"]}")
            }
        }
    }
    fn finish(self) -> Result<Vec<u8>> {
        if self.bytes.len() != self.limit {
            return Err(Error::Bounds);
        }
        Ok(self.bytes)
    }
}
pub(crate) fn encode(parameters: &[Value]) -> Result<Vec<u8>> {
    let mut size = 2;
    for (index, value) in parameters.iter().enumerate() {
        size = add(size, add(value_size(value)?, usize::from(index > 0))?)?;
    }
    if size > MAX_BYTES {
        return Err(Error::Bounds);
    }
    let mut writer = Writer::new(size)?;
    writer.push(b"[")?;
    for (index, value) in parameters.iter().enumerate() {
        if index > 0 {
            writer.push(b",")?;
        }
        writer.value(value)?;
    }
    writer.push(b"]")?;
    writer.finish()
}

pub(crate) fn decode_query(spec: &Spec, bytes: &[u8]) -> Result<Option<Vec<Value>>> {
    decode::query(spec, bytes)
}
pub(crate) fn decode_changes(bytes: &[u8]) -> Result<u64> {
    decode::changes(bytes)
}

#[cfg(test)]
mod tests;
